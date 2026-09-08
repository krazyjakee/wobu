//! The YAML source form, and the two ways it is allowed to be rejected.
//!
//! Structured YAML is the canonical authored form (#151): the Source view edits
//! it directly (US-14) and the forms edit the same model. Two rules make that
//! safe to save over.
//!
//! **Unknown fields are refused.** Every struct in this crate carries
//! `deny_unknown_fields`. Serde's default is to skip what it does not
//! understand, which for a file the app rewrites means a mistyped key — or a
//! key written by a newer Wobu — is dropped on the next save, and the author is
//! never told. A rejected file can be fixed; a silently truncated one is data
//! loss that shows up months later as a branch nobody can find.
//!
//! **Versions are checked before shape.** The version probe below reads
//! `schema_version` on its own, before anything strict runs, so a file from a
//! newer build is reported as a newer file rather than as thirty unknown
//! fields. Reporting the shape errors first would invite exactly the wrong
//! response: hand-deleting the fields this build does not know about.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::{Error, Result};
use crate::scene::Scene;
use crate::state::{StateSchema, VariableDecl};

/// The narrative source format this build reads and writes.
///
/// Separate from `wobu_core::SCHEMA_VERSION`, which versions the project folder
/// and the existing world files. Bumping one must not invalidate the other:
/// narrative source is additive to a project that may contain none of it, and
/// making every existing art project look like it needed a migration would be a
/// migration nobody asked for.
pub const SOURCE_SCHEMA_VERSION: u32 = 1;

/// Read `schema_version` and nothing else.
///
/// Deliberately permissive — no `deny_unknown_fields`, every other key ignored —
/// because its entire job is to work on a file this build cannot otherwise
/// parse.
#[derive(Debug, Deserialize)]
struct VersionProbe {
    #[serde(default)]
    schema_version: Option<u32>,
}

/// Reject a file this build should not touch, before trying to understand it.
pub(crate) fn check_version(yaml: &str) -> Result<()> {
    let probe: VersionProbe = serde_norway::from_str(yaml).map_err(|e| Error::from_yaml(&e))?;
    match probe.schema_version {
        None => Err(Error::MissingSchemaVersion),
        Some(SOURCE_SCHEMA_VERSION) => Ok(()),
        // Older versions are refused here too, rather than silently accepted.
        // A migration is a deliberate, tested step (#152's acceptance criteria);
        // reading a v0 file as if it were a v1 file is how a format acquires
        // undocumented shapes it can never drop.
        Some(found) => {
            Err(Error::UnsupportedSchemaVersion { found, supported: SOURCE_SCHEMA_VERSION })
        }
    }
}

/// All authored document types share this adapter. Singleton maps support nested
/// enums (not/compare, restrictions, enum state) that YAML tags cannot serialize.
/// Keep the direct readers first so ordinary syntax/shape errors retain libyaml
/// locations. The Value fallback accepts mixed legacy tags and canonical maps;
/// it is never used as a second, permissive schema or to discard unknown fields.
pub(crate) fn parse_yaml<T: DeserializeOwned>(yaml: &str) -> Result<T> {
    use serde_norway::with::singleton_map_recursive;
    let canonical_error =
        match singleton_map_recursive::deserialize(serde_norway::Deserializer::from_str(yaml)) {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
    let legacy_error = match serde_norway::from_str(yaml) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if let Ok(mut value) = serde_norway::from_str::<serde_norway::Value>(yaml)
        && normalize_tags(&mut value)
        && let Ok(document) = singleton_map_recursive::deserialize(value)
    {
        return Ok(document);
    }
    // Use the reader that got furthest, so a valid legacy tag does not mask an
    // unknown field or malformed value later in its payload.
    let position = |error: &serde_norway::Error| {
        error.location().map(|location| (location.line(), location.column()))
    };
    let error = if position(&legacy_error) > position(&canonical_error) {
        legacy_error
    } else {
        canonical_error
    };
    Err(Error::from_yaml(&error))
}

fn normalize_tags(value: &mut serde_norway::Value) -> bool {
    use serde_norway::{Mapping, Value};
    match value {
        Value::Tagged(tagged) => {
            let key = Value::String(tagged.tag.to_string().trim_start_matches('!').into());
            let mut payload = std::mem::take(&mut tagged.value);
            normalize_tags(&mut payload);
            *value = Value::Mapping(Mapping::from_iter([(key, payload)]));
            true
        }
        Value::Sequence(values) => {
            values.iter_mut().fold(false, |found, value| normalize_tags(value) | found)
        }
        Value::Mapping(values) => {
            values.values_mut().fold(false, |found, value| normalize_tags(value) | found)
        }
        _ => false,
    }
}

pub(crate) fn print_yaml<T: Serialize>(value: &T) -> Result<String> {
    #[derive(Serialize)]
    struct Canonical<'a, T: Serialize>(
        #[serde(with = "serde_norway::with::singleton_map_recursive")] &'a T,
    );
    serde_norway::to_string(&Canonical(value)).map_err(|error| Error::from_yaml(&error))
}

/// One scene as a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SceneDocument {
    pub schema_version: u32,
    pub scene: Scene,
}

impl SceneDocument {
    pub fn new(scene: Scene) -> SceneDocument {
        SceneDocument { schema_version: SOURCE_SCHEMA_VERSION, scene }
    }

    pub fn parse(yaml: &str) -> Result<SceneDocument> {
        check_version(yaml)?;
        parse_yaml(yaml)
    }

    pub fn to_yaml(&self) -> Result<String> {
        print_yaml(self)
    }
}

/// The declared state variables as a file.
///
/// Its own document rather than a block inside each scene, and that is the seam
/// to #155: variables are project-wide, so inlining them per scene would create
/// as many authoritative copies as there are scenes, and the first disagreement
/// between two of them would be a branch that types in one file and not in
/// another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StateDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub variables: Vec<VariableDecl>,
}

impl StateDocument {
    pub fn new(variables: Vec<VariableDecl>) -> StateDocument {
        StateDocument { schema_version: SOURCE_SCHEMA_VERSION, variables }
    }

    pub fn parse(yaml: &str) -> Result<StateDocument> {
        check_version(yaml)?;
        parse_yaml(yaml)
    }

    pub fn to_yaml(&self) -> Result<String> {
        print_yaml(self)
    }

    /// Build the checked schema. Fails on a declaration that is wrong on its own
    /// terms — a duplicate name, a default outside its own range.
    pub fn schema(&self) -> Result<StateSchema> {
        StateSchema::new(self.variables.iter().cloned())
    }
}
