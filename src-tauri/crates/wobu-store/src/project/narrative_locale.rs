//! Locale records use the production registry, immutable receipts and guarded conflict siblings.
mod sources;
use super::Project;
use crate::{
    Error, NarrativeRecordDocument as Document, NarrativeRecordFile as File,
    NarrativeRecordKind as Kind, Result, SourceSave,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wobu_core::Id;
use wobu_narrative::review::hash;
use wobu_narrative_locale::{
    self as locale, LocaleId, Policy, Row, SourceLine, Translation, TranslationVersion,
};
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Malformed { path: "narrative/localisation".into(), reason: e.to_string() }
}
fn identity(key: &impl Serialize) -> Id {
    let hex = hash(key);
    Id::from(u128::from_str_radix(&hex[..32], 16).expect("hash hex"))
}
fn translation_id(locale: &LocaleId, id: &str) -> Id {
    identity(&("locale_translation_v1", locale, id))
}
fn policy_id() -> Id {
    identity(&"locale_policy_v1")
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTranslation {
    #[serde(rename = "type")]
    kind: String,
    variant_id: String,
    translation: Translation,
    receipt: Id,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocaleView {
    pub policy: Policy,
    pub policy_guard: String,
    pub sources: BTreeMap<String, SourceLine>,
    pub translations: Vec<Translation>,
    pub translation_guards: BTreeMap<String, String>,
}
#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub applied: Vec<String>,
    pub diagnostics: Vec<locale::Diagnostic>,
    pub conflicts: BTreeMap<String, String>,
}
impl Project {
    pub fn locale_view(&self) -> Result<LocaleView> {
        let (policy, policy_guard) = self.locale_policy()?;
        let sources = self.locale_sources()?;
        let translations: Vec<_> = self.locale_translations()?.into_values().collect();
        let translation_guards = translations
            .iter()
            .map(|t| (format!("{}/{}", t.locale, t.variant_id), t.token()))
            .collect();
        Ok(LocaleView { policy, policy_guard, sources, translations, translation_guards })
    }
    pub fn locale_policy(&self) -> Result<(Policy, String)> {
        let file = self.narrative_record(Kind::Policy, policy_id())?;
        let policy: Policy = match file {
            Some(file) => serde_json::from_value(file.document.payload["policy"].clone())?,
            None => Policy::default(),
        };
        policy.validate().map_err(invalid)?;
        Ok((policy.clone(), hash(&policy)))
    }
    pub fn save_locale_policy(&mut self, policy: Policy, expected: &str) -> Result<SourceSave> {
        policy.validate().map_err(invalid)?;
        let (old, guard) = self.locale_policy()?;
        if guard != expected {
            return Err(invalid("Locale policy changed; reload before saving."));
        }
        let existing = self.narrative_record(Kind::Policy, policy_id())?;
        if existing
            .as_ref()
            .is_some_and(|f| f.document.payload["policy"] != serde_json::to_value(&old).unwrap())
        {
            return Err(invalid("Locale policy changed while capturing its stamp."));
        }
        let mut file = File {
            document: Document::new(
                Kind::Policy,
                policy_id(),
                "Locale release policy",
                serde_json::json!({"type":"narrative_locale_policy","policy":policy}),
            ),
            stamp: existing.and_then(|f| f.stamp),
        };
        self.save_narrative_record(&mut file)
    }
    pub fn locale_translations(&self) -> Result<BTreeMap<(LocaleId, String), Translation>> {
        let mut result = BTreeMap::new();
        for file in self.narrative_records(Kind::Production)? {
            if file.document.payload["type"] != "narrative_locale" {
                continue;
            }
            let stored: StoredTranslation = serde_json::from_value(file.document.payload)?;
            stored.translation.validate().map_err(invalid)?;
            if stored.variant_id != stored.translation.variant_id
                || file.document.id
                    != translation_id(&stored.translation.locale, &stored.variant_id)
            {
                return Err(invalid("Locale record identity mismatch."));
            }
            let receipt = self
                .narrative_record(Kind::Receipt, stored.receipt)?
                .ok_or_else(|| invalid("Locale history receipt is missing."))?;
            if receipt.document.payload
                != serde_json::json!({"type":"narrative_locale_decision","translation":stored.translation})
            {
                return Err(invalid("Locale history does not match its immutable decision."));
            }
            result
                .insert((stored.translation.locale.clone(), stored.variant_id), stored.translation);
        }
        Ok(result)
    }
    pub fn locale_export(&self, locale: &LocaleId, csv: bool) -> Result<String> {
        let sources = self.locale_sources()?;
        let translations = self.locale_translations()?;
        locale::interchange::encode(&locale::export_rows(locale, &sources, &translations), csv)
            .map_err(invalid)
    }
    pub fn locale_preview(&self, input: &str, csv: bool) -> Result<Vec<locale::Diagnostic>> {
        let rows = locale::interchange::decode(input, csv).map_err(invalid)?;
        Ok(locale::preview(&rows, &self.locale_sources()?, &self.locale_translations()?))
    }
    pub fn locale_import(&mut self, input: &str, csv: bool) -> Result<ImportReport> {
        self.ensure_writable()?;
        let rows = locale::interchange::decode(input, csv).map_err(invalid)?;
        let (sources, snapshots) = self.locale_capture()?;
        let existing = self.locale_translations()?;
        let mut report = ImportReport {
            applied: vec![],
            diagnostics: locale::preview(&rows, &sources, &existing),
            conflicts: BTreeMap::new(),
        };
        let blocked: std::collections::BTreeSet<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code != "missing_id")
            .map(|d| d.id.clone())
            .collect();
        for row in &rows {
            if blocked.contains(&row.source.id) {
                continue;
            }
            match self.locale_write_captured(row, false, &sources, &existing, &snapshots) {
                Ok(SourceSave::Saved(_)) => report.applied.push(row.source.id.clone()),
                Ok(SourceSave::Conflict { conflict_path }) => {
                    report.conflicts.insert(row.source.id.clone(), conflict_path);
                }
                Err(e) => report.diagnostics.push(locale::Diagnostic {
                    id: row.source.id.clone(),
                    code: "conflict".into(),
                    message: e.to_string(),
                }),
            }
        }
        Ok(report)
    }
    pub fn locale_approve(&mut self, row: &Row) -> Result<SourceSave> {
        self.locale_write(row, true)
    }
    fn locale_write(&mut self, row: &Row, approved: bool) -> Result<SourceSave> {
        let (sources, snapshots) = self.locale_capture()?;
        let existing = self.locale_translations()?;
        self.locale_write_captured(row, approved, &sources, &existing, &snapshots)
    }
    fn locale_write_captured(
        &mut self,
        row: &Row,
        approved: bool,
        sources: &BTreeMap<String, SourceLine>,
        existing: &BTreeMap<(LocaleId, String), Translation>,
        snapshots: &[super::narrative_review::ReviewSnapshot],
    ) -> Result<SourceSave> {
        self.ensure_writable()?;
        let source = sources.get(&row.source.id).ok_or_else(|| invalid("Unknown source ID."))?;
        let container: wobu_narrative::SceneId = source.container.parse().map_err(invalid)?;
        let _lock = super::narrative_review::scene_lock(self, container)?;
        let snapshot = snapshots
            .iter()
            .find(|s| s.scene().id == container)
            .ok_or_else(|| invalid("Missing source capture."))?;
        snapshot.check_observations(self)?;
        let selected = BTreeMap::from([(source.id.clone(), source.clone())]);
        let diagnostics = locale::preview(std::slice::from_ref(row), &selected, existing);
        if let Some(problem) = diagnostics.iter().find(|d| d.code != "missing_id") {
            return Err(invalid(&problem.message));
        }
        let id = translation_id(&row.locale, &row.source.id);
        let current_file = self.narrative_record(Kind::Production, id)?;
        let current = existing.get(&(row.locale.clone(), row.source.id.clone()));
        if current_file.as_ref().map(|f| f.document.payload["translation"].clone())
            != current.map(|t| serde_json::to_value(t).unwrap())
        {
            return Err(invalid("Translation changed during capture."));
        }
        if approved && current.is_none_or(|t| t.latest().forms != row.forms) {
            return Err(invalid("Import and inspect the exact translation before approving it."));
        }
        let mut translation = current.cloned().unwrap_or_else(|| Translation {
            version: locale::VERSION,
            variant_id: row.source.id.clone(),
            locale: row.locale.clone(),
            history: vec![],
        });
        translation.history.push(TranslationVersion {
            source_revision: row.source.revision.clone(),
            source_guard: row.source.guard.clone(),
            forms: row.forms.clone(),
            approved,
            actor: self.peer.to_string(),
        });
        translation.validate().map_err(invalid)?;
        // Recheck source immediately before publishing a decision, under its editorial lock.
        snapshot.check_observations(self)?;
        let receipt_id = wobu_core::new_id();
        let mut receipt = File {
            document: Document::new(
                Kind::Receipt,
                receipt_id,
                "Locale decision",
                serde_json::json!({"type":"narrative_locale_decision","translation":translation}),
            ),
            stamp: None,
        };
        self.save_narrative_record(&mut receipt)?;
        let stored = StoredTranslation {
            kind: "narrative_locale".into(),
            variant_id: row.source.id.clone(),
            translation,
            receipt: receipt_id,
        };
        let mut file = File {
            document: Document::new(
                Kind::Production,
                id,
                format!("{} {}", row.locale, row.source.id),
                serde_json::to_value(stored)?,
            ),
            stamp: current_file.and_then(|f| f.stamp),
        };
        self.save_narrative_record(&mut file)
    }
    pub fn locale_release(&self) -> Result<(locale::release::Bundle, Vec<locale::Diagnostic>)> {
        let fingerprint = self.narrative_fingerprint()?;
        let (policy, guard) = self.locale_policy()?;
        if policy.required.is_empty() {
            if self.locale_policy()?.1 != guard {
                return Err(invalid("Locale policy changed during export capture."));
            }
            return Ok(locale::release::prepare(&policy, &BTreeMap::new(), &BTreeMap::new()));
        }
        let (sources, snapshots) = self.locale_capture()?;
        let translations = self.locale_translations()?;
        let result = locale::release::prepare(&policy, &sources, &translations);
        for snapshot in snapshots {
            snapshot.check_observations(self)?;
        }
        if self.narrative_fingerprint()? != fingerprint
            || self.locale_policy()?.1 != guard
            || self.locale_translations()? != translations
        {
            return Err(invalid(
                "Locale policy, source or translations changed during export capture.",
            ));
        }
        Ok(result)
    }
}
