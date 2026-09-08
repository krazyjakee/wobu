//! Prepared native lookup. Authoring history and approval receipts do not enter runtime assets.
use crate::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub key: Key,
    pub origin: LocaleId,
    pub source_revision: String,
    pub translation_revision: Option<String>,
    pub template: String,
    pub spoken_text: String,
    pub parameters: BTreeMap<String, String>,
    pub audio: Blob,
    pub info: wav::Info,
    pub timing: Option<Blob>,
}
impl From<&Take> for Prepared {
    fn from(take: &Take) -> Self {
        Self {
            key: take.row.key.clone(),
            origin: take.row.origin.clone(),
            source_revision: take.row.source.revision.clone(),
            translation_revision: take.row.translation_guard.clone(),
            template: take.row.text.clone(),
            spoken_text: take.spoken_text.clone(),
            parameters: take.row.parameters.clone(),
            audio: take.audio.clone(),
            info: take.info.clone(),
            timing: take.timing.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub version: u32,
    /// Required recording locales; true permits visible text-only fallback.
    pub required: BTreeMap<LocaleId, bool>,
    pub timing: BTreeSet<LocaleId>,
    pub takes: BTreeMap<String, Prepared>,
    pub fallback: BTreeSet<String>,
}
impl Bundle {
    /// A different rendered sentence has no matching take. Hosts keep text visible on None.
    pub fn lookup(&self, key: &Key, parameters: &BTreeMap<String, String>) -> Option<&Prepared> {
        self.takes.get(&key.token()).filter(|take| &take.parameters == parameters)
    }
}
