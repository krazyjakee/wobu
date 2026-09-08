//! Provider-neutral recording manifests and immutable prepared media. No device or provider IO.
pub mod interchange;
pub mod release;
pub mod timing;
pub mod wav;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wobu_narrative::review::hash;
use wobu_narrative_locale::{LocaleId, PluralCategory, SourceLine, Translation};
pub const VERSION: u32 = 1;
pub const MAX_AUDIO_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_TIMING_BYTES: usize = 2 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
#[error("Invalid prepared media: {0}")]
pub struct Error(pub String);
pub type Result<T> = std::result::Result<T, Error>;
pub fn invalid(message: impl Into<String>) -> Error {
    Error(message.into())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notes {
    pub pronunciation: String,
    pub delivery: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    /// Required recording locales; true explicitly permits text-only fallback in Release.
    pub required: BTreeMap<LocaleId, bool>,
    pub notes: BTreeMap<String, Notes>,
    /// Locales whose required takes must include a validated timing sidecar.
    #[serde(default)]
    pub timing: BTreeSet<LocaleId>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            version: VERSION,
            required: BTreeMap::new(),
            notes: BTreeMap::new(),
            timing: BTreeSet::new(),
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION
            || self.required.len() > 64
            || self.timing.iter().any(|locale| !self.required.contains_key(locale))
            || self.notes.len() > 100_000
            || self.notes.values().any(|n| n.pronunciation.len() + n.delivery.len() > 16_384)
        {
            return Err(invalid("Unsupported or oversized recording policy."));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub id: String,
    pub locale: LocaleId,
    pub form: PluralCategory,
}
impl Key {
    pub fn token(&self) -> String {
        format!("{}/{}/{}", self.locale, self.id, self.form_name())
    }
    pub fn form_name(&self) -> String {
        serde_json::to_value(self.form)
            .expect("plural enum")
            .as_str()
            .expect("plural string")
            .into()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub version: u32,
    pub key: Key,
    pub source: SourceLine,
    /// Exact origin, never silently substitute another locale's voice.
    pub origin: LocaleId,
    pub translation_guard: Option<String>,
    pub text: String,
    pub notes: Notes,
    /// Named token -> preformatted plain text. Empty for static takes.
    pub parameters: BTreeMap<String, String>,
    pub media_guard: Option<String>,
    pub audio_path: String,
    pub timing_path: Option<String>,
    pub audio_hash: Option<String>,
    pub timing_hash: Option<String>,
    pub ready: bool,
}
impl Row {
    pub fn spoken_text(&self) -> Result<String> {
        let tokens = wobu_narrative_locale::format::placeholders(&self.text);
        if tokens != self.parameters.keys().cloned().collect::<BTreeSet<_>>() {
            return Err(invalid(
                "Freeze exactly every named placeholder in parameters before recording; unresolved templates require text-only fallback.",
            ));
        }
        // Use the same explicit token substitution contract as locale runtime lookup.
        let bundle = wobu_narrative_locale::release::Bundle {
            version: 1,
            policy: wobu_narrative_locale::Policy::default(),
            strings: BTreeMap::from([(
                self.key.locale.clone(),
                BTreeMap::from([(
                    self.key.id.clone(),
                    wobu_narrative_locale::release::Localized {
                        locale: self.origin.clone(),
                        forms: BTreeMap::from([(self.key.form, self.text.clone())]),
                    },
                )]),
            )]),
        };
        bundle
            .render(&self.key.locale, &self.key.id, self.key.form, &self.parameters)
            .map_err(|e| invalid(e.to_string()))
    }
    /// Changes to export-only paths/expected hashes do not change the frozen script.
    pub fn same_script(&self, other: &Self) -> bool {
        self.version == other.version
            && self.key == other.key
            && self.source == other.source
            && self.origin == other.origin
            && self.translation_guard == other.translation_guard
            && self.text == other.text
            && self.notes == other.notes
            && self.ready == other.ready
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    pub path: String,
    pub hash: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    pub row: Row,
    pub spoken_text: String,
    pub audio: Blob,
    pub info: wav::Info,
    pub timing: Option<Blob>,
    pub actor: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub version: u32,
    pub key: Key,
    pub history: Vec<Take>,
}
impl Binding {
    pub fn token(&self) -> String {
        hash(self)
    }
    pub fn latest(&self) -> Option<&Take> {
        self.history.last()
    }
    pub fn current(&self, row: &Row) -> bool {
        row.ready
            && self.latest().is_some_and(|t| {
                t.row.same_script(row)
                    && t.row.spoken_text().is_ok_and(|text| text == t.spoken_text)
            })
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION
            || self.key.id.parse::<wobu_core::Id>().is_err()
            || self.history.is_empty()
            || self.history.iter().any(|t| {
                t.row.version != VERSION
                    || t.row.source.id != self.key.id
                    || t.row.key != self.key
                    || t.actor.is_empty()
                    || t.row.spoken_text().ok().as_ref() != Some(&t.spoken_text)
            })
        {
            return Err(invalid("Media history identity or frozen text is invalid."));
        }
        for take in &self.history {
            validate_blob(&take.audio, "wav")?;
            if let Some(timing) = &take.timing {
                validate_blob(timing, "json")?;
            }
        }
        Ok(())
    }
}
pub fn validate_blob(blob: &Blob, extension: &str) -> Result<()> {
    if blob.hash.len() != 64
        || !blob.hash.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        || blob.path != format!("assets/media/{}.{}", blob.hash, extension)
        || blob.bytes == 0
        || blob.bytes > MAX_AUDIO_BYTES as u64
    {
        return Err(invalid(
            "Media blob must use its lowercase BLAKE3 hash and portable content-addressed path.",
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub key: String,
    pub code: String,
    pub message: String,
}
impl Diagnostic {
    pub fn new(key: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        Self { key: key.into(), code: code.into(), message: message.into() }
    }
}
pub fn rows(
    locale: &LocaleId,
    source_locale: &LocaleId,
    policy: &Policy,
    sources: &BTreeMap<String, SourceLine>,
    translations: &BTreeMap<(LocaleId, String), Translation>,
    bindings: &BTreeMap<String, Binding>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for source in sources.values() {
        let translation = translations.get(&(locale.clone(), source.id.clone()));
        let forms = if locale == source_locale {
            BTreeMap::from([(PluralCategory::Other, source.text.clone())])
        } else {
            translation
                .map(|t| t.latest().forms.clone())
                .unwrap_or_else(|| BTreeMap::from([(PluralCategory::Other, String::new())]))
        };
        let ready = source.ready
            && (locale == source_locale
                || translation.is_some_and(|t| t.current(source) && t.latest().approved));
        for (form, text) in forms {
            let key = Key { id: source.id.clone(), locale: locale.clone(), form };
            let token = key.token();
            let stem =
                format!("recordings/{}/{}/{}", locale.file_stem(), source.id, key.form_name());
            rows.push(Row {
                version: VERSION,
                key,
                source: source.clone(),
                origin: locale.clone(),
                translation_guard: if locale == source_locale {
                    None
                } else {
                    translation.map(Translation::token)
                },
                text,
                notes: policy.notes.get(&token).cloned().unwrap_or_else(|| Notes {
                    pronunciation: String::new(),
                    delivery: source.delivery_notes.clone(),
                }),
                parameters: BTreeMap::new(),
                media_guard: bindings.get(&token).map(Binding::token),
                audio_path: format!("{stem}.wav"),
                timing_path: None,
                audio_hash: None,
                timing_hash: None,
                ready,
            });
        }
    }
    rows
}
pub fn preview(rows: &[Row], current: &BTreeMap<String, Row>) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let key = row.key.token();
        let mut add = |code, message| out.push(Diagnostic::new(&key, code, message));
        if !seen.insert(key.clone()) {
            add("duplicate", "Duplicate locale/string/form; every copy is skipped.");
        }
        let Some(now) = current.get(&key) else {
            add("unknown", "Unknown source ID, locale or plural form.");
            continue;
        };
        if !row.same_script(now) {
            add("stale_script", "Source, translation or recording notes changed since export.");
        }
        if !now.ready {
            add(
                "not_ready",
                "Source and selected translation must be current and approved; source wording must be locked.",
            );
        }
        if row.media_guard != now.media_guard {
            add("media_conflict", "A newer take was imported since export.");
        }
        if row.spoken_text().is_err() {
            add(
                "parameters",
                "Freeze exactly every placeholder as a preformatted plain-text parameter.",
            );
        }
        if row.audio_path.is_empty() {
            add("missing_audio", "Audio path is required.");
        }
    }
    for key in current.keys().filter(|k| !seen.contains(*k)) {
        out.push(Diagnostic::new(key, "missing", "Not supplied; existing media is unchanged."));
    }
    out
}
