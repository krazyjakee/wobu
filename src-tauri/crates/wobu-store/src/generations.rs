//! Append-only generation records.
//!
//! A generation is canonical JSON in the project folder, not a row in the
//! local index. Its ULID gives every attempt its own name, and that name is
//! claimed exactly once: there is no update operation and therefore no shared-
//! folder conflict to resolve. The index only mirrors these files so a Concepts
//! grid can ask for one node's history without walking the share.

use std::path::{Path, PathBuf};

use wobu_core::Generation;

use crate::atomic::{self, Stamp};
use crate::error::{Error, Result};
use crate::paths;

pub const GENERATIONS_DIR: &str = "generations";
const ARCHIVE_DIR: &str = ".deleted";
const SPEND_AGGREGATE: &str = ".wobu/spend/aggregate.json";

/// The shell may keep a disposable aggregate of this canonical ledger for
/// display polling. Receipt changes invalidate it; failure to remove a cache is
/// deliberately ignored because paid admission never trusts it.
pub(crate) fn invalidate_spend_aggregate(root: &Path) {
    let path = root.join(SPEND_AGGREGATE);
    let _ = std::fs::remove_file(path);
}

/// Persist one complete generation, refusing to reuse its id forever.
pub fn write(root: &Path, generation: &Generation) -> Result<(String, Stamp)> {
    let rel = generation.rel_path();
    let target = paths::from_rel_string(root, &rel);
    let mut bytes = serde_json::to_vec_pretty(generation)?;
    bytes.push(b'\n');
    let stamp = atomic::write_once(root, &target, &bytes)?;
    invalidate_spend_aggregate(root);
    Ok((rel, stamp))
}

/// Remove a receipt from the visible ledger without erasing its accounting record.
///
/// Receipt bytes remain unchanged below `generations/.deleted/`; Concepts and
/// History scan only the month shards, while strict spend reconstruction reads
/// both locations. Generated assets have their own lifecycle and are untouched.
pub fn archive(root: &Path, generation: &Generation) -> Result<String> {
    let rel = generation.rel_path();
    let source = paths::from_rel_string(root, &rel);
    let month = generation.created_at.format("%Y-%m").to_string();
    let archived_rel = format!("{GENERATIONS_DIR}/{ARCHIVE_DIR}/{month}/{}.json", generation.id);
    let target = paths::from_rel_string(root, &archived_rel);
    let parent = target.parent().expect("an archived generation path always has a parent");
    std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    if target.exists() {
        return Err(Error::AlreadyExists(target));
    }
    std::fs::rename(&source, &target).map_err(|error| Error::io(&source, error))?;
    Ok(rel)
}

/// Read and validate one generation file.
///
/// The record decides its own canonical path from its timestamp and id. A JSON
/// document moved or copied to a different month/name is not a second
/// generation and must not be indexed as one: accepting it would make an index
/// rebuild change the number of results in the Concepts grid.
pub fn read_at(root: &Path, path: &Path) -> Result<Option<(Generation, String, Stamp)>> {
    let Some((text, stamp)) = atomic::read_stamped(path)? else { return Ok(None) };
    let generation: Generation = serde_json::from_str(&text).map_err(|error| {
        Error::MalformedGeneration { path: path.to_path_buf(), reason: error.to_string() }
    })?;
    let rel = path.strip_prefix(root).map(paths::to_rel_string).map_err(|_| {
        Error::MalformedGeneration {
            path: path.to_path_buf(),
            reason: "generation file is outside the project".to_string(),
        }
    })?;
    let expected = generation.rel_path();
    if rel != expected {
        return Err(Error::MalformedGeneration {
            path: path.to_path_buf(),
            reason: format!("generation belongs at {expected}"),
        });
    }
    Ok(Some((generation, rel, stamp)))
}

/// Every correctly-shaped JSON path under the month shards.
pub(crate) fn list_paths(root: &Path) -> Vec<(String, PathBuf)> {
    let generations = root.join(GENERATIONS_DIR);
    walkdir::WalkDir::new(&generations)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let rel = entry.path().strip_prefix(root).ok().map(paths::to_rel_string)?;
            Some((rel, entry.into_path()))
        })
        .collect()
}

fn archived_paths(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root.join(GENERATIONS_DIR).join(ARCHIVE_DIR))
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(walkdir::DirEntry::into_path)
        .collect()
}

fn read_archived(root: &Path, path: &Path) -> Result<Option<Generation>> {
    let Some((text, _)) = atomic::read_stamped(path)? else { return Ok(None) };
    let generation: Generation = serde_json::from_str(&text).map_err(|error| {
        Error::MalformedGeneration { path: path.to_path_buf(), reason: error.to_string() }
    })?;
    let month = generation.created_at.format("%Y-%m").to_string();
    let expected = root
        .join(GENERATIONS_DIR)
        .join(ARCHIVE_DIR)
        .join(month)
        .join(format!("{}.json", generation.id));
    if path != expected {
        return Err(Error::MalformedGeneration {
            path: path.to_path_buf(),
            reason: format!("archived generation belongs at {}", expected.display()),
        });
    }
    Ok(Some(generation))
}

/// Rehydrate all valid generation records from the canonical folder.
///
/// A sync client can expose an incomplete JSON file while copying. Like asset
/// scanning, a rebuild leaves an unreadable immutable file alone and continues;
/// a later reconcile indexes it once the copy is complete.
pub(crate) fn scan(root: &Path) -> Vec<(Generation, String, Stamp)> {
    list_paths(root)
        .into_iter()
        .filter_map(|(_, path)| read_at(root, &path).ok().flatten())
        .collect()
}

/// Read every canonical receipt and fail closed on a malformed one.
///
/// Concepts can skip a file while a sync client is copying it and catch it on
/// the next reconcile. A spend ceiling cannot: silently omitting a paid receipt
/// would authorise more work than the project allows.
pub(crate) fn read_all_strict(root: &Path) -> Result<Vec<Generation>> {
    let mut receipts = Vec::new();
    for (_, path) in list_paths(root) {
        if let Some((generation, _, _)) = read_at(root, &path)? {
            receipts.push(generation);
        }
    }
    for path in archived_paths(root) {
        if let Some(generation) = read_archived(root, &path)? {
            receipts.push(generation);
        }
    }
    receipts.sort_by(|left, right| {
        left.created_at.cmp(&right.created_at).then_with(|| left.id.cmp(&right.id))
    });
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use wobu_core::{Generation, InfluenceSnapshot};

    use super::*;

    fn record(id: &str) -> Generation {
        Generation {
            id: wobu_core::Id::from_string(id).unwrap(),
            node_id: wobu_core::new_id(),
            created_at: "2026-07-31T14:22:11Z".parse::<DateTime<Utc>>().unwrap(),
            preset: "character_sheet".into(),
            view_type: None,
            user_prompt: "at dusk".into(),
            compiled_prompt: "full compiled prompt".into(),
            negative_prompt: "text, watermark".into(),
            backend: "gemini".into(),
            model: "gemini-2.5-flash-image".into(),
            seed: 42,
            params: Default::default(),
            output_asset_ids: vec![],
            influence_snapshot: InfluenceSnapshot { layers: vec![] },
        }
    }

    #[test]
    fn records_are_month_sharded_and_cannot_be_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".wobu/tmp")).unwrap();
        let generation = record("01ARZ3NDEKTSV4RRFFQ69G5FAV");

        let (rel, _) = write(dir.path(), &generation).unwrap();
        assert_eq!(rel, "generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json");
        assert!(matches!(write(dir.path(), &generation), Err(Error::AlreadyExists(_))));

        let loaded = scan(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, generation);
    }

    #[test]
    fn a_record_under_the_wrong_name_is_not_indexable() {
        let dir = tempfile::tempdir().unwrap();
        let generation = record("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let wrong = dir.path().join("generations/2026-07/wrong.json");
        std::fs::create_dir_all(wrong.parent().unwrap()).unwrap();
        std::fs::write(&wrong, serde_json::to_vec(&generation).unwrap()).unwrap();

        assert!(matches!(read_at(dir.path(), &wrong), Err(Error::MalformedGeneration { .. })));
    }

    #[test]
    fn archiving_removes_a_receipt_from_galleries_but_not_spend_history() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".wobu/tmp")).unwrap();
        let generation = record("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        write(dir.path(), &generation).unwrap();

        archive(dir.path(), &generation).unwrap();

        assert!(scan(dir.path()).is_empty());
        assert_eq!(read_all_strict(dir.path()).unwrap(), vec![generation]);
    }
}
