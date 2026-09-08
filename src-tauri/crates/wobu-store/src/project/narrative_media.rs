//! Prepared recording takes are immutable blobs linked by guarded production records.
mod files;
mod imports;
mod release;
use super::Project;
use crate::{
    Error, NarrativeRecordDocument as Document, NarrativeRecordFile as File,
    NarrativeRecordKind as Kind, Result, SourceSave,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wobu_core::Id;
use wobu_narrative::review::hash;
use wobu_narrative_locale::LocaleId;
use wobu_narrative_media::{self as media, Binding, Diagnostic, Policy, Row};
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Malformed { path: "narrative/media".into(), reason: e.to_string() }
}
fn identity(key: &impl Serialize) -> Id {
    Id::from(u128::from_str_radix(&hash(key)[..32], 16).expect("hash hex"))
}
fn policy_id() -> Id {
    identity(&"media_policy_v1")
}
fn binding_id(key: &str) -> Id {
    identity(&("media_binding_v1", key))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBinding {
    #[serde(rename = "type")]
    kind: String,
    variant_id: String,
    binding: Binding,
    receipt: Id,
}
#[derive(Debug, Serialize)]
pub struct MediaView {
    pub policy: Policy,
    pub policy_guard: String,
    pub locale: LocaleId,
    pub rows: Vec<Row>,
    pub bindings: BTreeMap<String, Binding>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub applied: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub conflicts: BTreeMap<String, String>,
}
#[derive(Debug, Serialize)]
pub struct Audition {
    pub path: String,
    pub take: media::Take,
    pub timing: Option<media::timing::Track>,
    pub current: bool,
}
impl Project {
    pub fn media_policy(&self) -> Result<(Policy, String)> {
        let policy: Policy = self.production_policy(policy_id())?;
        policy.validate().map_err(invalid)?;
        Ok((policy.clone(), hash(&policy)))
    }
    pub fn save_media_policy(&mut self, policy: Policy, expected: &str) -> Result<SourceSave> {
        self.ensure_writable()?;
        policy.validate().map_err(invalid)?;
        let (old, guard) = self.media_policy()?;
        if guard != expected {
            return Err(invalid("Recording policy changed; reload before saving."));
        }
        let existing = self.narrative_record(Kind::Policy, policy_id())?;
        if existing
            .as_ref()
            .is_some_and(|f| f.document.payload["policy"] != serde_json::to_value(&old).unwrap())
        {
            return Err(invalid("Recording policy changed while capturing."));
        }
        let mut file = File {
            document: Document::new(
                Kind::Policy,
                policy_id(),
                "Recording release policy",
                serde_json::json!({"type":"narrative_media_policy","policy":policy}),
            ),
            stamp: existing.and_then(|f| f.stamp),
        };
        self.save_narrative_record(&mut file)
    }
    pub fn media_bindings(&self) -> Result<BTreeMap<String, Binding>> {
        let mut result = BTreeMap::new();
        for file in self.narrative_records(Kind::Production)? {
            if file.document.payload["type"] != "narrative_media" {
                continue;
            }
            let stored: StoredBinding = serde_json::from_value(file.document.payload)?;
            stored.binding.validate().map_err(invalid)?;
            let key = stored.binding.key.token();
            if stored.variant_id != stored.binding.key.id || file.document.id != binding_id(&key) {
                return Err(invalid("Recording identity mismatch."));
            }
            let receipt = self
                .narrative_record(Kind::Receipt, stored.receipt)?
                .ok_or_else(|| invalid("Recording history receipt is missing."))?;
            if receipt.document.payload
                != serde_json::json!({"type":"narrative_media_decision","binding":stored.binding})
            {
                return Err(invalid("Recording history differs from immutable receipt."));
            }
            result.insert(key, stored.binding);
        }
        Ok(result)
    }
    pub fn media_view(&self, locale: &LocaleId) -> Result<MediaView> {
        let capture = self.production_capture()?;
        let (policy, policy_guard) = self.media_policy()?;
        let bindings = self.media_bindings()?;
        let rows = media::rows(
            locale,
            &capture.policy.source,
            &policy,
            &capture.sources,
            &capture.translations,
            &bindings,
        );
        let mut diagnostics = Vec::new();
        for row in &rows {
            if let Some(take) = bindings.get(&row.key.token()).and_then(Binding::latest)
                && let Err(e) = files::check_take_metadata(self.root(), take)
            {
                diagnostics.push(Diagnostic::new(row.key.token(), "invalid_media", e.to_string()));
            }
        }
        capture.verify(self)?;
        if self.media_policy()?.1 != policy_guard || self.media_bindings()? != bindings {
            return Err(invalid("Recording records changed while capturing."));
        }
        Ok(MediaView { policy, policy_guard, locale: locale.clone(), rows, bindings, diagnostics })
    }
    pub fn media_export(&self, locale: &LocaleId, csv: bool) -> Result<String> {
        let view = self.media_view(locale)?;
        media::interchange::encode(
            &view.rows.into_iter().filter(|r| r.ready).collect::<Vec<_>>(),
            csv,
        )
        .map_err(invalid)
    }
    pub fn media_audition(&self, key: &media::Key, history: usize) -> Result<Audition> {
        let binding =
            self.media_binding(key)?.ok_or_else(|| invalid("No recording for this line."))?;
        let container = binding
            .latest()
            .ok_or_else(|| invalid("Recording has no take."))?
            .row
            .source
            .container
            .parse()
            .map_err(invalid)?;
        let capture = self.production_capture_selected(Some(container))?;
        let (policy, _) = self.media_policy()?;
        let sources = capture
            .sources
            .get(&key.id)
            .map(|source| BTreeMap::from([(source.id.clone(), source.clone())]))
            .unwrap_or_default();
        let rows = media::rows(
            &key.locale,
            &capture.policy.source,
            &policy,
            &sources,
            &capture.translations,
            &BTreeMap::new(),
        );
        let take = binding.history.get(history).ok_or_else(|| invalid("Unknown take version."))?;
        let (_, timing) = files::read_take(self.root(), take)?;
        let path = files::safe_path(self.root(), &take.audio.path)?;
        capture.verify(self)?;
        Ok(Audition {
            path: path.to_string_lossy().into(),
            take: take.clone(),
            timing,
            current: rows.iter().find(|r| r.key == *key).is_some_and(|r| binding.current(r))
                && history + 1 == binding.history.len(),
        })
    }
    fn media_binding(&self, key: &media::Key) -> Result<Option<Binding>> {
        let Some(file) = self.narrative_record(Kind::Production, binding_id(&key.token()))? else {
            return Ok(None);
        };
        let stored: StoredBinding = serde_json::from_value(file.document.payload)?;
        stored.binding.validate().map_err(invalid)?;
        if stored.binding.key != *key || stored.variant_id != key.id {
            return Err(invalid("Recording identity mismatch."));
        }
        let receipt = self
            .narrative_record(Kind::Receipt, stored.receipt)?
            .ok_or_else(|| invalid("Recording receipt is missing."))?;
        if receipt.document.payload
            != serde_json::json!({"type":"narrative_media_decision","binding":stored.binding})
        {
            return Err(invalid("Recording history differs from immutable receipt."));
        }
        Ok(Some(stored.binding))
    }
    /// Include every immutable historical take in peer transfer; the index is not canonical.
    pub fn media_sync_blobs(&self) -> Result<Vec<media::Blob>> {
        let mut result = BTreeMap::new();
        for binding in self.media_bindings()?.values() {
            for take in &binding.history {
                for blob in std::iter::once(&take.audio).chain(take.timing.iter()) {
                    // A partially received binding may precede its blob. Do not advertise absent data.
                    if files::check_blob_metadata(self.root(), blob).is_ok() {
                        result.insert(blob.path.clone(), blob.clone());
                    }
                }
            }
        }
        Ok(result.into_values().collect())
    }
}
