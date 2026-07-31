//! The recent-projects list, shown on the Launcher.
//!
//! This is the one piece of global state Wobu keeps, and it is deliberately
//! only a *hint*: it holds paths, which are machine-specific, so a missing or
//! stale entry is normal rather than an error.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

use crate::error::Result;
use crate::project::ProjectSummary;

const MAX_RECENTS: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub id: Id,
    pub name: String,
    pub path: String,
    pub last_opened_at: DateTime<Utc>,
}

pub fn list() -> Vec<RecentProject> {
    list_in(&crate::paths::recents_path())
}

/// The list logic, against an explicit file. Split out from [`list`] so the
/// behaviour can be tested without writing to the real user's app data.
fn list_in(path: &Path) -> Vec<RecentProject> {
    let Ok(raw) = std::fs::read_to_string(path) else { return Vec::new() };
    // A hand-mangled or truncated recents file is a hint we can rebuild, not an
    // error worth blocking the launcher on.
    let mut entries: Vec<RecentProject> = serde_json::from_str(&raw).unwrap_or_default();
    // A project folder can be deleted, unmounted or renamed between sessions.
    entries.retain(|e| Path::new(&e.path).join("project.json").is_file());
    entries.sort_by(|a, b| b.last_opened_at.cmp(&a.last_opened_at));
    entries
}

/// The launcher wants these as full summaries, but a share that is currently
/// unmounted must not stall the list — so the flags are read from the recorded
/// path without touching the folder.
pub fn list_summaries() -> Vec<ProjectSummary> {
    list()
        .into_iter()
        .map(|e| ProjectSummary {
            id: e.id,
            name: e.name,
            on_network_share: crate::paths::is_network_path(Path::new(&e.path)),
            read_only: false,
            path: e.path,
            last_opened_at: Some(e.last_opened_at),
        })
        .collect()
}

pub fn record(summary: &ProjectSummary) -> Result<()> {
    record_in(&crate::paths::recents_path(), summary)
}

fn record_in(path: &Path, summary: &ProjectSummary) -> Result<()> {
    let mut entries = list_in(path);
    // Keyed by id, not path: reopening a project that moved should update the
    // existing entry rather than leave a dead one beside it.
    entries.retain(|e| e.id != summary.id);
    entries.insert(
        0,
        RecentProject {
            id: summary.id,
            name: summary.name.clone(),
            path: summary.path.clone(),
            last_opened_at: Utc::now(),
        },
    );
    entries.truncate(MAX_RECENTS);
    write_in(path, &entries)
}

pub fn forget(id: Id) -> Result<()> {
    forget_in(&crate::paths::recents_path(), id)
}

fn forget_in(path: &Path, id: Id) -> Result<()> {
    let mut entries = list_in(path);
    entries.retain(|e| e.id != id);
    write_in(path, &entries)
}

fn write_in(path: &Path, entries: &[RecentProject]) -> Result<()> {
    let path: PathBuf = path.to_path_buf();
    if let Some(parent) = path.parent() {
        crate::paths::ensure_dir(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(entries)?)
        .map_err(|e| crate::error::Error::io(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory that looks enough like a project for [`list_in`]'s liveness
    /// check, plus the summary that points at it.
    fn project(dir: &Path, name: &str) -> ProjectSummary {
        let root = dir.join(format!("{name}.wobu"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("project.json"), "{}").unwrap();
        ProjectSummary {
            id: wobu_core::new_id(),
            name: name.to_string(),
            path: root.to_string_lossy().into_owned(),
            on_network_share: false,
            read_only: false,
            last_opened_at: None,
        }
    }

    #[test]
    fn a_missing_recents_file_is_an_empty_list_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_in(&dir.path().join("recent.json")).is_empty());
    }

    #[test]
    fn a_corrupt_recents_file_is_an_empty_list_not_an_error() {
        // The launcher must still open if this file gets truncated by a crash.
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        std::fs::write(&recents, "[{\"id\": not json").unwrap();
        assert!(list_in(&recents).is_empty());
    }

    #[test]
    fn the_most_recently_opened_project_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        let (a, b) = (project(dir.path(), "ashfall"), project(dir.path(), "brine"));

        record_in(&recents, &a).unwrap();
        record_in(&recents, &b).unwrap();

        let names: Vec<_> = list_in(&recents).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["brine", "ashfall"]);
    }

    #[test]
    fn reopening_a_project_moves_it_up_rather_than_duplicating_it() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        let (a, b) = (project(dir.path(), "ashfall"), project(dir.path(), "brine"));

        record_in(&recents, &a).unwrap();
        record_in(&recents, &b).unwrap();
        record_in(&recents, &a).unwrap();

        let entries = list_in(&recents);
        assert_eq!(entries.len(), 2, "no duplicate entry for ashfall");
        assert_eq!(entries[0].name, "ashfall");
    }

    #[test]
    fn a_project_that_moved_updates_its_entry_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        let mut p = project(dir.path(), "ashfall");
        record_in(&recents, &p).unwrap();

        // Same project (same id), opened from a new path — a remounted share.
        let moved = project(dir.path(), "ashfall-on-nas");
        p.path = moved.path.clone();
        record_in(&recents, &p).unwrap();

        let entries = list_in(&recents);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, moved.path);
    }

    #[test]
    fn a_project_folder_that_disappeared_drops_off_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        let (a, b) = (project(dir.path(), "ashfall"), project(dir.path(), "brine"));
        record_in(&recents, &a).unwrap();
        record_in(&recents, &b).unwrap();

        // An unmounted share or a deleted folder.
        std::fs::remove_dir_all(&a.path).unwrap();

        let names: Vec<_> = list_in(&recents).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["brine"]);
    }

    #[test]
    fn the_list_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        for i in 0..MAX_RECENTS + 5 {
            record_in(&recents, &project(dir.path(), &format!("p{i}"))).unwrap();
        }
        let entries = list_in(&recents);
        assert_eq!(entries.len(), MAX_RECENTS);
        assert_eq!(entries[0].name, format!("p{}", MAX_RECENTS + 4), "newest survives");
    }

    #[test]
    fn forgetting_a_project_removes_only_that_one() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        let (a, b) = (project(dir.path(), "ashfall"), project(dir.path(), "brine"));
        record_in(&recents, &a).unwrap();
        record_in(&recents, &b).unwrap();

        forget_in(&recents, a.id).unwrap();

        let names: Vec<_> = list_in(&recents).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["brine"]);
    }

    #[test]
    fn forgetting_something_that_was_never_there_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let recents = dir.path().join("recent.json");
        forget_in(&recents, wobu_core::new_id()).unwrap();
        assert!(list_in(&recents).is_empty());
    }
}
