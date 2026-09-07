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

use serde::{Deserialize, Serialize};

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
fn check_version(yaml: &str) -> Result<()> {
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
        serde_norway::from_str(yaml).map_err(|e| Error::from_yaml(&e))
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_norway::to_string(self).map_err(|e| Error::from_yaml(&e))
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
        serde_norway::from_str(yaml).map_err(|e| Error::from_yaml(&e))
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_norway::to_string(self).map_err(|e| Error::from_yaml(&e))
    }

    /// Build the checked schema. Fails on a declaration that is wrong on its own
    /// terms — a duplicate name, a default outside its own range.
    pub fn schema(&self) -> Result<StateSchema> {
        StateSchema::new(self.variables.iter().cloned())
    }
}
