//! Guarded canonical world source; absent in existing art-only projects.
use super::{SourceSave, relative};
use crate::{
    atomic::{self, Stamp, WriteOutcome},
    error::{Error, Result},
};
use std::path::Path;
use wobu_narrative::WorldDocument;

pub const WORLD_FILE: &str = "narrative/world.yaml";

pub fn read(root: &Path) -> Result<Option<(WorldDocument, Stamp)>> {
    let path = super::registry::safe_path(root, WORLD_FILE)?;
    let Some((yaml, stamp)) = atomic::read_stamped(&path)? else { return Ok(None) };
    let document = WorldDocument::parse(&yaml)
        .map_err(|error| Error::Malformed { path, reason: error.to_string() })?;
    Ok(Some((document, stamp)))
}

pub fn write(
    root: &Path,
    document: &WorldDocument,
    expected: Option<&Stamp>,
    peer: &str,
) -> Result<SourceSave> {
    let path = super::registry::safe_path(root, WORLD_FILE)?;
    let yaml = document
        .to_yaml()
        .map_err(|error| Error::Malformed { path: path.clone(), reason: error.to_string() })?;
    match atomic::guarded_write(root, &path, &yaml, expected, peer)? {
        WriteOutcome::Written(stamp) => Ok(SourceSave::Saved(stamp)),
        WriteOutcome::Conflict { conflict_path, .. } => {
            Ok(SourceSave::Conflict { conflict_path: relative(root, &conflict_path) })
        }
    }
}
