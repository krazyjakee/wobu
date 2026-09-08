//! Versioned native JSON assets. No authoring database, providers or engine adapter.
mod disk;
mod validate;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wobu_narrative::Name;
use wobu_narrative_compiler::{Graph, Profile, SourceRef, StateVariable};

pub use disk::{publish, read};
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_STRINGS: usize = 100_000;
pub const MAX_STRING_BYTES: usize = 1024 * 1024;
pub const INCOMPLETE: &str = ".incomplete";
const REQUIRED: [&str; 4] = ["graph.json", "state.json", "strings/en.json", "media.json"];
/// Declared by a package whose graph contains supporting text assets (#167).
///
/// A capability rather than a silent addition, because a reader written against
/// version 1 would otherwise load such a package, ignore the `texts` map it does
/// not understand, and ship a game that never says a single bark — a failure
/// with no symptom. Refusing to load is the only honest answer available to a
/// reader that cannot deliver the content. It is absent from a package with no
/// supporting text, so existing packages and their identities are unchanged.
pub const SUPPORTING_TEXT: &str = "supporting_text";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid narrative package: {0}")]
    Invalid(String),
    #[error("package filesystem error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    // Reject duplicate object fields as well as unknown typed fields. JSON maps otherwise
    // silently replace duplicate string IDs, schema names and manifest paths.
    let value: validate::UniqueJson =
        serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    serde_json::from_value(value.0).map_err(|e| invalid(e.to_string()))
}
fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecord {
    pub bytes: u64,
    pub hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub graph_version: u32,
    pub profile: Profile,
    pub locale: String,
    pub required_capabilities: BTreeMap<String, u32>,
    pub files: BTreeMap<String, FileRecord>,
    /// Hash of canonical file records, independent of publication paths and wall clocks.
    pub payload_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizedString {
    pub text: String,
    pub revision: Option<String>,
}
type Strings = BTreeMap<String, LocalizedString>;
/// v1 has no authored media bindings. Reject non-empty rather than claim to package them.
type Media = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

impl Package {
    /// Callers compile using the requested profile first. This checks structural integrity
    /// again; approval provenance deliberately never enters the runtime package.
    pub fn build(mut graph: Graph, debug: bool) -> Result<Self> {
        validate::graph(&graph)?;
        if debug && graph.profile == Profile::Release {
            return Err(invalid("release packages cannot contain debug source maps"));
        }
        let mut strings = Strings::new();
        for scene in graph.scenes.values_mut() {
            for beat in scene.beats.values_mut() {
                for slot in &mut beat.dialogue {
                    for variant in &mut slot.variants {
                        strings.insert(
                            variant.id.clone(),
                            LocalizedString {
                                text: std::mem::take(&mut variant.text),
                                revision: Some(std::mem::take(&mut variant.revision)),
                            },
                        );
                    }
                }
                for choice in &mut beat.choices {
                    strings.insert(
                        choice.id.clone(),
                        LocalizedString { text: std::mem::take(&mut choice.label), revision: None },
                    );
                }
            }
        }
        // Supporting text goes into the same table, keyed by the same variant
        // identity and carrying the same wording revision. That is what makes a
        // locale pack or a recording script one list rather than two: a
        // translator working from `strings/en.json` cannot tell, and should not
        // need to tell, whether a line is spoken in a scene or shouted at a gate.
        let supporting_text = !graph.texts.is_empty();
        for asset in graph.texts.values_mut() {
            for entry in &mut asset.entries {
                for slot in &mut entry.lines {
                    for variant in &mut slot.variants {
                        strings.insert(
                            variant.id.clone(),
                            LocalizedString {
                                text: std::mem::take(&mut variant.text),
                                revision: Some(std::mem::take(&mut variant.revision)),
                            },
                        );
                    }
                }
            }
        }
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        files.insert("strings/en.json".into(), json(&strings)?);
        files.insert("state.json".into(), json(&std::mem::take(&mut graph.state))?);
        files.insert("media.json".into(), json(&Media::new())?);
        let source_map = std::mem::take(&mut graph.source_map);
        if debug {
            files.insert("debug/source-map.json".into(), json(&source_map)?);
        }
        let profile = graph.profile;
        files.insert("graph.json".into(), json(&graph)?);
        let records: BTreeMap<_, _> = files
            .iter()
            .map(|(name, bytes)| {
                (name.clone(), FileRecord { bytes: bytes.len() as u64, hash: hash(bytes) })
            })
            .collect();
        let manifest = Manifest {
            format: "wobu-narrative".into(),
            version: FORMAT_VERSION,
            graph_version: graph.version,
            profile,
            locale: "en".into(),
            required_capabilities: validate::capabilities(supporting_text),
            payload_hash: hash(&json(&records)?),
            files: records,
        };
        let package = Self { manifest, files };
        package.graph()?;
        Ok(package)
    }
    pub fn manifest_bytes(&self) -> Result<Vec<u8>> {
        json(&self.manifest)
    }
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|bytes| bytes.len() as u64).sum()
    }
    pub fn string_count(&self) -> Result<usize> {
        Ok(self.strings()?.len())
    }
    fn strings(&self) -> Result<Strings> {
        parse(self.file("strings/en.json")?)
    }
    fn file(&self, name: &str) -> Result<&[u8]> {
        self.files.get(name).map(Vec::as_slice).ok_or_else(|| invalid(format!("missing {name}")))
    }
    pub fn graph(&self) -> Result<Graph> {
        self.check_manifest()?;
        for (name, record) in &self.manifest.files {
            let bytes = self.file(name)?;
            if bytes.len() as u64 != record.bytes || hash(bytes) != record.hash {
                return Err(invalid(format!("content hash/size mismatch: {name}")));
            }
        }
        if self.files.len() != self.manifest.files.len() {
            return Err(invalid("unlisted payload file"));
        }
        let mut graph: Graph = parse(self.file("graph.json")?)?;
        if graph.version != self.manifest.graph_version
            || graph.profile != self.manifest.profile
            || !graph.state.is_empty()
            || !graph.source_map.is_empty()
        {
            return Err(invalid("graph envelope disagrees with manifest"));
        }
        let state: BTreeMap<Name, StateVariable> = parse(self.file("state.json")?)?;
        let media: Media = parse(self.file("media.json")?)?;
        if !media.is_empty() {
            return Err(invalid("media bindings require a future supported capability"));
        }
        let mut strings = self.strings()?;
        if strings.len() > MAX_STRINGS {
            return Err(invalid("too many strings"));
        }
        for scene in graph.scenes.values_mut() {
            for beat in scene.beats.values_mut() {
                for slot in &mut beat.dialogue {
                    for variant in &mut slot.variants {
                        if !variant.text.is_empty() || !variant.revision.is_empty() {
                            return Err(invalid("inline dialogue in packaged graph"));
                        }
                        let text = strings
                            .remove(&variant.id)
                            .ok_or_else(|| invalid(format!("missing string {}", variant.id)))?;
                        variant.text = text.text;
                        variant.revision =
                            text.revision.ok_or_else(|| invalid("dialogue revision missing"))?;
                    }
                }
                for choice in &mut beat.choices {
                    if !choice.label.is_empty() {
                        return Err(invalid("inline choice text in packaged graph"));
                    }
                    let text = strings
                        .remove(&choice.id)
                        .ok_or_else(|| invalid(format!("missing choice string {}", choice.id)))?;
                    if text.revision.is_some() {
                        return Err(invalid("choice strings do not carry dialogue revisions"));
                    }
                    choice.label = text.text;
                }
            }
        }
        for asset in graph.texts.values_mut() {
            for entry in &mut asset.entries {
                for slot in &mut entry.lines {
                    for variant in &mut slot.variants {
                        if !variant.text.is_empty() || !variant.revision.is_empty() {
                            return Err(invalid("inline supporting text in packaged graph"));
                        }
                        let text = strings
                            .remove(&variant.id)
                            .ok_or_else(|| invalid(format!("missing string {}", variant.id)))?;
                        variant.text = text.text;
                        variant.revision = text
                            .revision
                            .ok_or_else(|| invalid("supporting text revision missing"))?;
                    }
                }
            }
        }
        // The manifest is checked before the graph is parsed, so this is the
        // first point at which the declared capability can be compared with what
        // the payload actually contains. A package claiming supporting text and
        // shipping none — or the reverse — is malformed either way.
        if self.manifest.required_capabilities != validate::capabilities(!graph.texts.is_empty()) {
            return Err(invalid("declared capabilities disagree with the packaged graph"));
        }
        if !strings.is_empty() {
            return Err(invalid("unreferenced strings"));
        }
        graph.state = state;
        if self.manifest.files.contains_key("debug/source-map.json") {
            graph.source_map =
                parse::<BTreeMap<String, SourceRef>>(self.file("debug/source-map.json")?)?;
        }
        validate::graph(&graph)?;
        Ok(graph)
    }
    fn check_manifest(&self) -> Result<()> {
        validate::manifest(&self.manifest)
    }
}
