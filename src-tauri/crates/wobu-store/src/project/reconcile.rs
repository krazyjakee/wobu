//! Noticing that the folder changed underneath us.
//!
//! Wobu does not own the folder — Obsidian, a text editor, Dropbox and a sync
//! round all write to it. Reconciliation observes what is on disk, decides what
//! that means for the index, and applies it as one step, so an edit made
//! outside Wobu is a first-class change rather than corruption.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use wobu_core::{Asset, Generation, Id, Node};

use super::*;
use crate::assets::{self};
use crate::atomic::{self, Stamp};
use crate::conflict::{self};
use crate::error::{Error, Result};
use crate::generations;
use crate::markdown;
use crate::paths;
use crate::scan::{Cancel, ScanProgress};

/// The index-only half of a full reconciliation.
///
/// Capturing this is deliberately cheap and does not touch the project folder.
/// A network watcher can therefore release the shell's project mutex before
/// [`observe`](Self::observe) performs directory listings and file reads.
pub struct ReconcilePlan {
    root: PathBuf,
    project_id: Id,
    node_stamps: HashMap<String, (i64, u64)>,
    corrupt: HashSet<String>,
    assets: HashSet<String>,
    generations: HashSet<String>,
}

enum ObservedNode {
    Valid { rel: String, node: Box<Node>, stamp: Stamp },
    Corrupt { rel: String, error: String, stamp: Stamp },
}

/// Filesystem evidence gathered without holding the shell's project mutex.
pub struct ReconcileObservation {
    plan: ReconcilePlan,
    nodes: Vec<ObservedNode>,
    seen_nodes: HashSet<String>,
    seen_node_stamps: HashMap<String, (i64, u64)>,
    assets: Vec<Asset>,
    seen_assets: HashSet<String>,
    generations: Vec<(Generation, String, Stamp)>,
    seen_generations: HashSet<String>,
    generation_ledger_changed: bool,
}

impl ReconcilePlan {
    /// Perform every potentially slow directory listing, stat and read.
    pub fn observe(self) -> Result<ReconcileObservation> {
        if !paths::project_is_present(&self.root) {
            return Err(Error::Disconnected);
        }

        // A remount can put a different project at the same path. Checking its
        // canonical identity outside the mutex prevents an old observation
        // from being applied to the newly mounted world.
        let meta_path = self.root.join(PROJECT_FILE);
        let raw = std::fs::read_to_string(&meta_path).map_err(|e| Error::io(&meta_path, e))?;
        let meta: ProjectMeta = serde_json::from_str(&raw)?;
        if meta.id != self.project_id {
            return Err(Error::NotAProject(self.root.clone()));
        }

        let mut nodes = Vec::new();
        let mut seen_nodes = HashSet::new();
        let mut seen_node_stamps = HashMap::new();
        for (rel, path) in
            markdown_files_at(&self.root).into_iter().filter(|(_, path)| !is_conflict_path(path))
        {
            let Some((mtime, size)) = atomic::peek(&path)? else { continue };
            seen_nodes.insert(rel.clone());
            seen_node_stamps.insert(rel.clone(), (mtime, size));
            if self.node_stamps.get(&rel) == Some(&(mtime, size)) {
                continue;
            }
            let Some((text, stamp)) = atomic::read_stamped(&path)? else { continue };
            match markdown::from_markdown(&text, &path) {
                Ok(node) => nodes.push(ObservedNode::Valid { rel, node: Box::new(node), stamp }),
                Err(error) => nodes.push(ObservedNode::Corrupt {
                    rel,
                    error: relative_message_at(&self.root, &error.to_string()),
                    stamp,
                }),
            }
        }

        let mut assets_seen = HashSet::new();
        let mut asset_updates = Vec::new();
        for (rel, path) in assets::list_paths(&self.root) {
            assets_seen.insert(rel.clone());
            if !self.assets.contains(&rel)
                && let Some(asset) = assets::describe_at(&self.root, &path)
            {
                asset_updates.push(asset);
            }
        }

        let mut generations_seen = HashSet::new();
        let mut generation_updates = Vec::new();
        let mut generation_ledger_changed = false;
        for (rel, path) in generations::list_paths(&self.root) {
            generations_seen.insert(rel.clone());
            if self.generations.contains(&rel) {
                continue;
            }
            generation_ledger_changed = true;
            if let Ok(Some(record)) = generations::read_at(&self.root, &path) {
                generation_updates.push(record);
            }
        }

        // A share disappearing during a walk can look exactly like mass
        // deletion. Recheck after all listings and discard that observation.
        if !paths::project_is_present(&self.root) {
            return Err(Error::Disconnected);
        }

        Ok(ReconcileObservation {
            plan: self,
            nodes,
            seen_nodes,
            seen_node_stamps,
            assets: asset_updates,
            seen_assets: assets_seen,
            generations: generation_updates,
            seen_generations: generations_seen,
            generation_ledger_changed,
        })
    }
}

impl ReconcileObservation {
    /// Recheck the evidence immediately before the shell acquires its mutex.
    ///
    /// This is intentionally filesystem-only. A mismatch means somebody wrote
    /// between observation and apply; callers discard this snapshot and
    /// coalesce a fresh pass rather than installing stale parsed content.
    pub fn revalidate(&self) -> Result<bool> {
        if !paths::project_is_present(&self.plan.root) {
            return Err(Error::Disconnected);
        }

        let meta_path = self.plan.root.join(PROJECT_FILE);
        let raw = std::fs::read_to_string(&meta_path).map_err(|e| Error::io(&meta_path, e))?;
        let meta: ProjectMeta = serde_json::from_str(&raw)?;
        if meta.id != self.plan.project_id {
            return Ok(false);
        }

        let current_nodes: HashSet<_> = markdown_files_at(&self.plan.root)
            .into_iter()
            .filter(|(_, path)| !is_conflict_path(path))
            .map(|(rel, _)| rel)
            .collect();
        if current_nodes != self.seen_nodes {
            return Ok(false);
        }
        for (rel, observed_stamp) in &self.seen_node_stamps {
            let path = paths::from_rel_string(&self.plan.root, rel);
            if atomic::peek(&path)?.as_ref() != Some(observed_stamp) {
                return Ok(false);
            }
        }
        for observed in &self.nodes {
            let (rel, stamp) = match observed {
                ObservedNode::Valid { rel, stamp, .. }
                | ObservedNode::Corrupt { rel, stamp, .. } => (rel, stamp),
            };
            let path = paths::from_rel_string(&self.plan.root, rel);
            if atomic::read_stamped(&path)?.map(|(_, current)| current) != Some(stamp.clone()) {
                return Ok(false);
            }
        }

        let current_assets: HashSet<_> =
            assets::list_paths(&self.plan.root).into_iter().map(|(rel, _)| rel).collect();
        if current_assets != self.seen_assets {
            return Ok(false);
        }
        let current_generations: HashSet<_> =
            generations::list_paths(&self.plan.root).into_iter().map(|(rel, _)| rel).collect();
        if current_generations != self.seen_generations {
            return Ok(false);
        }
        for (_, rel, stamp) in &self.generations {
            let path = paths::from_rel_string(&self.plan.root, rel);
            if atomic::read_stamped(&path)?.map(|(_, current)| current) != Some(stamp.clone()) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(super) fn markdown_files_at(root: &Path) -> Vec<(String, PathBuf)> {
    markdown_files_under(root, &root.join(NODES_DIR))
}

fn markdown_files_under(root: &Path, start: &Path) -> Vec<(String, PathBuf)> {
    let Ok(start_relative) = start.strip_prefix(root) else { return Vec::new() };
    let start_depth = start_relative.components().count();
    let Some(remaining_depth) = 3usize.checked_sub(start_depth) else { return Vec::new() };
    walkdir::WalkDir::new(start)
        .max_depth(remaining_depth)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")))
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?;
            Some((paths::to_rel_string(relative), entry.into_path()))
        })
        .collect()
}

fn relative_message_at(root: &Path, message: &str) -> String {
    let root = root.to_string_lossy();
    let stripped = message.replace(&format!("{root}/"), "");
    stripped.replace(&format!("{}\\", root.replace('/', "\\")), "")
}

/// Conflict siblings are for a human to resolve. Indexing one would put a ghost
/// duplicate of the node in the navigator, and — far worse — make it a save
/// target, so that resolving the conflict could start a new one.
fn is_conflict_path(path: &Path) -> bool {
    path.file_name().is_some_and(|n| conflict::is_sibling(&n.to_string_lossy()))
}

impl Project {
    /// Read the canonical project files and rebuild the index from scratch.
    pub fn rescan(&mut self) -> Result<()> {
        self.rescan_with(&Cancel::new(), &mut |_| {})
    }

    /// `rescan`, reporting progress and stopping when asked.
    ///
    /// This is the one operation that can take minutes rather than
    /// milliseconds: it is every node file, read, over whatever the folder
    /// happens to be mounted on. Everything else in this crate touches one file
    /// at a time.
    ///
    /// The cancel is checked between files, not inside a read. A single read on
    /// a wedged SMB mount blocks until the mount's own timeout — minutes, by
    /// default — and nothing in userspace can shorten that. Checking per file
    /// bounds the wait by one file rather than by the rest of the folder, which
    /// is the best that can honestly be offered.
    pub fn rescan_with(
        &mut self,
        cancel: &Cancel,
        on_progress: &mut impl FnMut(ScanProgress),
    ) -> Result<()> {
        let files = self.node_files();
        let total = files.len();
        on_progress(ScanProgress { done: 0, total });

        // Nothing is cleared until we know we are going to finish. Clearing up
        // front and then being cancelled would leave an empty index for a world
        // that is entirely intact, and the next open would silently rebuild it
        // — slowly, over the same slow share.
        let mut fresh = Vec::with_capacity(total);
        let mut broken = Vec::new();

        for (done, (rel, path)) in files.into_iter().enumerate() {
            cancel.check()?;
            match atomic::read_stamped(&path)? {
                Some((text, stamp)) => match markdown::from_markdown(&text, &path) {
                    Ok(node) => fresh.push((node, rel, stamp)),
                    Err(e) => broken.push((rel, self.relative_message(&e.to_string()))),
                },
                None => continue,
            }
            on_progress(ScanProgress { done: done + 1, total });
        }

        // Read before the clear, like the nodes above and for the same reason:
        // a rebuild that emptied the asset table and then failed would leave a
        // library that is entirely intact looking empty.
        let blobs = assets::scan(&self.root);
        let generation_records = generations::scan(&self.root);

        cancel.check()?;
        // A file a sync client mangled is left on disk exactly as it is, and
        // recorded so the navigator can say so. The clear and every refill are
        // one transaction, so a malformed row or SQLite failure restores the
        // previous complete read model rather than exposing a partial rebuild.
        self.index.rebuild_from_scan(&blobs, &generation_records, &fresh, &broken)?;
        on_progress(ScanProgress { done: total, total });
        Ok(())
    }

    /// Fold external edits (Obsidian, git pull, a collaborator on the share)
    /// into the index. Returns true if anything changed.
    ///
    /// Only files whose `(mtime, size)` moved are re-read: listing a directory
    /// over SMB is cheap, re-reading hundreds of small files is not.
    pub fn reconcile(&mut self) -> Result<bool> {
        for _ in 0..3 {
            let observation = self.reconcile_plan()?.observe()?;
            if !observation.revalidate()? {
                continue;
            }
            // No other index writer can interleave with this synchronous API,
            // so an index-stale baseline would be an internal invariant
            // violation. Retrying is still safer than applying it.
            if let Some(changed) = self.apply_reconcile(observation)? {
                return Ok(changed);
            }
        }
        // A continuously changing folder will be observed again on the next
        // watcher tick. Do not let it monopolise a synchronous caller forever.
        Ok(false)
    }

    /// Capture the index baseline for a full filesystem observation.
    pub fn reconcile_plan(&self) -> Result<ReconcilePlan> {
        Ok(ReconcilePlan {
            root: self.root.clone(),
            project_id: self.id(),
            node_stamps: self.index.all_stamps()?,
            corrupt: self.index.corrupt_paths()?.into_iter().collect(),
            assets: self.index.asset_paths()?,
            generations: self.index.generation_paths()?,
        })
    }

    /// Apply a previously gathered observation using index operations only.
    ///
    /// `None` means another reconcile/save changed the index after the plan was
    /// captured. Nothing from the stale observation is applied; the next poll
    /// starts from the newer baseline.
    pub fn apply_reconcile(&mut self, observation: ReconcileObservation) -> Result<Option<bool>> {
        let ReconcileObservation {
            plan,
            nodes,
            seen_nodes,
            seen_node_stamps: _,
            assets,
            seen_assets,
            generations,
            seen_generations,
            mut generation_ledger_changed,
        } = observation;

        if self.id() != plan.project_id
            || self.root != plan.root
            || self.index.all_stamps()? != plan.node_stamps
            || self.index.corrupt_paths()?.into_iter().collect::<HashSet<_>>() != plan.corrupt
            || self.index.asset_paths()? != plan.assets
            || self.index.generation_paths()? != plan.generations
        {
            return Ok(None);
        }

        let mut changed = false;
        for observed in nodes {
            match observed {
                ObservedNode::Valid { rel, node, stamp } => {
                    self.index.upsert_node(&node, &rel, &stamp)?;
                    if plan.corrupt.contains(&rel) {
                        self.index.clear_corrupt(&rel)?;
                    }
                    changed = true;
                }
                ObservedNode::Corrupt { rel, error, .. } => {
                    self.index.mark_corrupt(&rel, &error)?;
                    if !plan.corrupt.contains(&rel) {
                        changed = true;
                    }
                }
            }
        }
        for rel in plan.node_stamps.keys().filter(|rel| !seen_nodes.contains(*rel)) {
            self.index.remove_by_rel_path(rel)?;
            changed = true;
        }
        for rel in plan.corrupt.iter().filter(|rel| !seen_nodes.contains(*rel)) {
            self.index.clear_corrupt(rel)?;
            changed = true;
        }

        for asset in assets {
            self.index.upsert_asset(&asset)?;
            changed = true;
        }
        for rel in plan.assets.iter().filter(|rel| !seen_assets.contains(*rel)) {
            self.index.remove_asset_by_rel_path(rel)?;
            changed = true;
        }

        for (generation, rel, stamp) in generations {
            self.index.upsert_generation(&generation, &rel, &stamp)?;
            changed = true;
        }
        for rel in plan.generations.iter().filter(|rel| !seen_generations.contains(*rel)) {
            self.index.remove_generation_by_rel_path(rel)?;
            changed = true;
            generation_ledger_changed = true;
        }
        if generation_ledger_changed {
        }
        Ok(Some(changed))
    }

    /// Reconcile only local node paths reported by the OS watcher.
    pub fn reconcile_paths(&mut self, changed_paths: &[PathBuf]) -> Result<bool> {
        if !self.is_present() {
            return Err(Error::Disconnected);
        }

        let known = self.index.all_stamps()?;
        let was_corrupt: HashSet<String> = self.index.corrupt_paths()?.into_iter().collect();
        let mut targets = HashSet::new();

        for changed in changed_paths {
            let absolute =
                if changed.is_absolute() { changed.clone() } else { self.root.join(changed) };
            let Ok(relative) = absolute.strip_prefix(&self.root) else { continue };
            let rel = paths::to_rel_string(relative);
            if rel != NODES_DIR && !rel.starts_with(&format!("{NODES_DIR}/")) {
                continue;
            }

            if absolute.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
                if !is_conflict_path(&absolute) {
                    targets.insert(rel);
                }
                continue;
            }

            let prefix = format!("{}/", rel.trim_end_matches('/'));
            targets.extend(
                known
                    .keys()
                    .chain(was_corrupt.iter())
                    .filter(|known_rel| known_rel.starts_with(&prefix))
                    .cloned(),
            );
            if absolute.is_dir() {
                targets.extend(
                    markdown_files_under(&self.root, &absolute)
                        .into_iter()
                        .filter(|(_, path)| !is_conflict_path(path))
                        .map(|(rel, _)| rel),
                );
            }
        }

        let mut changed = false;
        for rel in targets {
            let path = paths::from_rel_string(&self.root, &rel);
            let Some((mtime, size)) = atomic::peek(&path)? else {
                if known.contains_key(&rel) {
                    self.index.remove_by_rel_path(&rel)?;
                    changed = true;
                }
                if was_corrupt.contains(&rel) {
                    self.index.clear_corrupt(&rel)?;
                    changed = true;
                }
                continue;
            };
            if known.get(&rel) == Some(&(mtime, size)) {
                continue;
            }
            let Some((text, stamp)) = atomic::read_stamped(&path)? else { continue };
            match markdown::from_markdown(&text, &path) {
                Ok(node) => {
                    self.index.upsert_node(&node, &rel, &stamp)?;
                    if was_corrupt.contains(&rel) {
                        self.index.clear_corrupt(&rel)?;
                    }
                    changed = true;
                }
                Err(error) => {
                    self.index.mark_corrupt(&rel, &self.relative_message(&error.to_string()))?;
                    if !was_corrupt.contains(&rel) {
                        changed = true;
                    }
                }
            }
        }
        Ok(changed)
    }

    /// Every Markdown file under `nodes/`, as `(relative path, absolute path)`.
    ///
    /// Includes conflict siblings; the two callers below split them apart. One
    /// walk rather than two so the node list and the conflict list can never
    /// disagree about which files exist, which they would the moment somebody
    /// changed a depth limit in one of them.
    pub(super) fn markdown_files(&self) -> Vec<(String, PathBuf)> {
        markdown_files_at(&self.root)
    }

    /// Every node Markdown file, as `(relative path, absolute path)`.
    pub(super) fn node_files(&self) -> Vec<(String, PathBuf)> {
        self.markdown_files().into_iter().filter(|(_, path)| !is_conflict_path(path)).collect()
    }

    /// The other half: files `guarded_write` parked, which are never nodes.
    pub(super) fn conflict_files(&self) -> Vec<(String, PathBuf)> {
        self.markdown_files().into_iter().filter(|(_, path)| is_conflict_path(path)).collect()
    }
}
