//! Versioned localisation interchange, review and deterministic native lookup.
pub mod error;
pub mod format;
pub mod id;
pub mod interchange;
pub mod release;
pub use error::{Error, Result};
pub use format::PluralCategory;
pub use id::LocaleId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wobu_narrative::review::hash;

pub const VERSION: u32 = 1;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLine {
    pub id: String,
    pub slot: String,
    pub container: String,
    pub speaker: String,
    pub text: String,
    pub revision: String,
    /// Source context and protection decision, independent of translation review.
    pub guard: String,
    pub context: String,
    pub delivery_notes: String,
    pub placeholders: BTreeSet<String>,
    pub ready: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub source: LocaleId,
    /// Each required locale explicitly either refuses fallback or permits truncation then source.
    pub required: BTreeMap<LocaleId, bool>,
}
impl Default for Policy {
    fn default() -> Self {
        Self { version: VERSION, source: id::default_source(), required: BTreeMap::new() }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.version,
                supported: VERSION,
            });
        }
        if self.required.contains_key(&self.source) {
            return Err(Error::Malformed("Source locale cannot also require translation.".into()));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationVersion {
    pub source_revision: String,
    pub source_guard: String,
    pub forms: BTreeMap<PluralCategory, String>,
    pub approved: bool,
    pub actor: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Translation {
    pub version: u32,
    pub variant_id: String,
    pub locale: LocaleId,
    /// Append-only decisions preserve imported words even when source changes.
    pub history: Vec<TranslationVersion>,
}
impl Translation {
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.version,
                supported: VERSION,
            });
        }
        if self.variant_id.parse::<wobu_core::Id>().is_err()
            || self.history.is_empty()
            || self
                .history
                .iter()
                .any(|v| v.actor.is_empty() || !v.forms.contains_key(&PluralCategory::Other))
        {
            return Err(Error::Malformed(
                "Translation requires a stable ID, author and other plural form.".into(),
            ));
        }
        Ok(())
    }
    pub fn latest(&self) -> &TranslationVersion {
        self.history.last().expect("validated translation history")
    }
    pub fn current(&self, source: &SourceLine) -> bool {
        let latest = self.latest();
        source.ready
            && latest.source_revision == source.revision
            && latest.source_guard == source.guard
            && valid_forms(source, &latest.forms)
    }
    pub fn token(&self) -> String {
        hash(self)
    }
}
pub fn valid_forms(source: &SourceLine, forms: &BTreeMap<PluralCategory, String>) -> bool {
    forms.contains_key(&PluralCategory::Other)
        && forms
            .values()
            .all(|text| !text.trim().is_empty() && format::compare(&source.text, text).is_empty())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub version: u32,
    pub locale: LocaleId,
    pub source: SourceLine,
    /// Optimistic translation revision captured at export, not reset by preview.
    pub translation_guard: Option<String>,
    pub forms: BTreeMap<PluralCategory, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub code: String,
    pub message: String,
}
pub fn preview(
    rows: &[Row],
    sources: &BTreeMap<String, SourceLine>,
    existing: &BTreeMap<(LocaleId, String), Translation>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |id: &str, code: &str, message: &str| {
        diagnostics.push(Diagnostic { id: id.into(), code: code.into(), message: message.into() })
    };
    for row in rows {
        let id = &row.source.id;
        if !seen.insert((row.locale.clone(), id.clone())) {
            add(id, "duplicate_id", "Duplicate locale/ID; all copies will be skipped.");
        }
        if row.version != VERSION {
            add(id, "version", "Unsupported interchange version.");
        }
        let Some(source) = sources.get(id) else {
            add(id, "unknown_id", "This stable ID is not in the saved source.");
            continue;
        };
        if &row.source != source {
            add(
                id,
                "stale_source",
                "Source changed or was unlocked since export; retain this translation for comparison and export again.",
            );
        }
        if !source.ready {
            add(
                id,
                "source_not_ready",
                "Source must be approved, current and locked before translation import.",
            );
        }
        if !valid_forms(source, &row.forms) {
            add(
                id,
                "placeholders",
                "Every plural form must be nonempty and preserve the source placeholder token set; other is required.",
            );
        }
        if row.translation_guard
            != existing.get(&(row.locale.clone(), id.clone())).map(Translation::token)
        {
            add(
                id,
                "translation_conflict",
                "Translation changed since export; this row will not overwrite it.",
            );
        }
    }
    let supplied: BTreeSet<_> = rows.iter().map(|row| row.source.id.as_str()).collect();
    for id in sources.keys().filter(|id| !supplied.contains(id.as_str())) {
        add(
            id,
            "missing_id",
            "Source ID absent from import; its existing translation is unchanged.",
        );
    }
    diagnostics
}
pub fn export_rows(
    locale: &LocaleId,
    sources: &BTreeMap<String, SourceLine>,
    existing: &BTreeMap<(LocaleId, String), Translation>,
) -> Vec<Row> {
    sources
        .values()
        .filter(|source| source.ready)
        .map(|source| {
            let translation = existing.get(&(locale.clone(), source.id.clone()));
            Row {
                version: VERSION,
                locale: locale.clone(),
                source: source.clone(),
                translation_guard: translation.map(Translation::token),
                forms: translation
                    .map(|t| t.latest().forms.clone())
                    .unwrap_or_else(|| BTreeMap::from([(PluralCategory::Other, String::new())])),
            }
        })
        .collect()
}
