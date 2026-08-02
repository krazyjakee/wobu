//! A project is a self-contained folder.
//!
//! Nothing canonical is stored outside it — no global application database, no
//! absolute paths, no secrets. Copy the folder to a USB stick and it opens
//! somewhere else; delete the local index and nothing is lost. See
//! `docs/02-data-model.md`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::asset::AssetRef;
use wobu_core::{
    Asset, AssetKind, AssetRole, Description, DescriptionState, EnhanceStamp, Generation, Id, Link,
    LinkEdge, LinkRole, Node, NodeKind, NodeSummary, SCHEMA_VERSION, SourceStamp, kind_def,
    kind_registry,
};

use crate::apply;
use crate::assets::{self, ImportedAsset};
use crate::atomic::{self, Stamp, WriteOutcome};
use crate::conflict::{self, Conflict, Keep, Resolved};
use crate::error::{Error, Result};
use crate::generations;
use crate::index::{CorruptFile, Index, Touched};
use crate::markdown;
use crate::paths;
use crate::scan::{Cancel, ScanProgress};
use crate::transfer::{TransferBundle, TransferOutcome};

const PROJECT_FILE: &str = "project.json";
const PROJECT_META_RECOVERY: &str = "project.json.recovery";
const NODES_DIR: &str = "nodes";

/// New and pre-ceiling projects start with the same modest shared guardrail.
/// Stored as integer USD micros: no floating-point drift in an admission check.
pub const DEFAULT_SPEND_CEILING_USD_MICROS: u64 = 10_000_000;

fn default_spend_ceiling() -> Option<u64> {
    Some(DEFAULT_SPEND_CEILING_USD_MICROS)
}

/// `project.json`. Records *which* provider a project prefers, never a key —
/// keys live in the OS keychain, because project folders get shared.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    pub id: Id,
    pub name: String,
    pub schema_version: u32,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub providers: serde_json::Map<String, serde_json::Value>,
    /// Shared because it authorises spend by this project, not by one machine.
    #[serde(default = "default_spend_ceiling")]
    pub spend_ceiling_usd_micros: Option<u64>,
}

/// What the launcher and title bar bind to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: Id,
    pub name: String,
    /// Absolute, and therefore *only* ever held in memory or in the local
    /// recents file — never written into the project folder.
    pub path: String,
    pub on_network_share: bool,
    pub read_only: bool,
    pub last_opened_at: Option<DateTime<Utc>>,
}

/// One role an asset plays on one node in the Assets library detail panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetUsageRole {
    pub role: AssetRole,
    pub weight: f32,
    pub enabled: bool,
}

/// Every reason one node keeps an asset from being an orphan.
///
/// Grouped per node so the frontend can show the name and tags once even when
/// the same image is both a palette and full reference. `cover` is independent
/// of roles in the node model and still counts as use for safe deletion.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetUsage {
    pub asset_id: Id,
    pub node_id: Id,
    pub node_name: String,
    pub node_kind: NodeKind,
    pub node_tags: Vec<String>,
    pub roles: Vec<AssetUsageRole>,
    pub cover: bool,
}

#[derive(Debug)]
pub enum SaveOutcome {
    Saved(Box<Node>),
    /// Someone else changed the file since we loaded it. Ours was written
    /// alongside; the UI raises a diff rather than merging prose.
    Conflict {
        conflict_path: String,
    },
}

/// What happened to an enhanced description on its way to disk.
///
/// A type of its own rather than a third [`SaveOutcome`] variant, because
/// [`RefusedEdit`](Enhanced::RefusedEdit) is not a saving problem — nothing
/// raced, nothing is at risk on the share — and folding it in would make every
/// ordinary save site match on a case that can never happen to it.
#[derive(Debug)]
pub enum Enhanced {
    Saved(Box<Node>),
    /// The description on disk was written by hand, and the enhance was not
    /// told to overwrite it. The node comes back untouched so the UI can show
    /// the user what it is about to replace and ask.
    RefusedEdit(Box<Node>),
    Conflict {
        conflict_path: String,
    },
}

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

pub struct Project {
    root: PathBuf,
    meta: ProjectMeta,
    index: Index,
    /// Where [`index`](Project::index) actually is. Held rather than derived
    /// from the ULID because [`open_at_index`](Project::open_at_index) can put
    /// it somewhere else, and a caller that deletes the file to force a rebuild
    /// must be told the truth about which file that is.
    index_path: PathBuf,
    on_network_share: bool,
    read_only: bool,
    /// Who we are, fixed at open. See [`crate::peer`] — it is a short alias for
    /// this installation's ed25519 key, and it replaces the `$USER` that used to
    /// make two collaborators on default installs into the same person.
    ///
    /// Held on the project rather than read at each use so that a session cannot
    /// stamp two conflict siblings in one folder under two different names,
    /// which is a folder nobody can read back.
    peer: String,
    /// Every node, whole, for the influence engine. Empty until something asks —
    /// see [`world_nodes`](Project::world_nodes), which is also where the cost
    /// of holding this is argued.
    world: Vec<Node>,
}

impl Project {
    pub fn create(parent_dir: &Path, name: &str) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Core(wobu_core::Error::EmptyName));
        }
        let folder = format!("{}.wobu", wobu_core::slugify(name)?);
        let root = parent_dir.join(folder);
        if root.exists() {
            return Err(Error::AlreadyExists(root));
        }

        for dir in [
            root.join(NODES_DIR),
            root.join("assets/originals"),
            root.join("assets/thumbs"),
            root.join("generations"),
            root.join(".wobu/tmp"),
            root.join(".wobu/sessions"),
        ] {
            paths::ensure_dir(&dir)?;
        }

        let meta = ProjectMeta {
            id: wobu_core::new_id(),
            name: name.to_string(),
            schema_version: SCHEMA_VERSION,
            created_at: Utc::now(),
            providers: serde_json::Map::new(),
            spend_ceiling_usd_micros: default_spend_ceiling(),
        };
        std::fs::write(root.join(PROJECT_FILE), serde_json::to_string_pretty(&meta)?)
            .map_err(|e| Error::io(root.join(PROJECT_FILE), e))?;

        let index_path = paths::index_path(&meta.id);
        let index = Index::open_for(&meta.id)?;
        index.clear()?;
        let mut project = Project {
            root,
            meta,
            index,
            index_path,
            on_network_share: false,
            read_only: false,
            peer: crate::peer::alias().to_owned(),
            world: Vec::new(),
        };

        // Every influence stack is rooted in these two, so a project without
        // them is not openable in a meaningful sense.
        for def in kind_registry().iter().filter(|d| d.singleton) {
            project.create_node(def.kind, def.label, None)?;
        }
        Ok(project)
    }

    pub fn open(path: &Path) -> Result<Project> {
        Project::open_with(path, &Cancel::new(), &mut |_| {})
    }

    /// `open`, reporting progress and stoppable part-way.
    ///
    /// Only the first open of a project pays the full scan; after that the
    /// index is warm and `reconcile` only reads what moved. But the first open
    /// is exactly when the user has no idea whether the app is working, so it
    /// is the one that needs both a number and a way out.
    pub fn open_with(
        path: &Path,
        cancel: &Cancel,
        on_progress: &mut impl FnMut(ScanProgress),
    ) -> Result<Project> {
        Project::open_inner(path, None, cancel, on_progress)
    }

    /// `open`, with the index at a path the caller picks.
    ///
    /// [`paths::index_path`] keys the database by project ULID under app data,
    /// which is what the app wants — the index has to outlive the share being
    /// remounted somewhere else. A test suite wants the opposite: several tests
    /// opening one copied fixture in parallel would meet in a single SQLite
    /// file and drop each other's tables mid-query when the schema version has
    /// moved. Tests and read-only transfer scans call this with a disposable
    /// path; ordinary project opens must keep using the project-id index.
    pub fn open_at_index(path: &Path, index_path: &Path) -> Result<Project> {
        Project::open_inner(path, Some(index_path), &Cancel::new(), &mut |_| {})
    }

    fn open_inner(
        path: &Path,
        index_path: Option<&Path>,
        cancel: &Cancel,
        on_progress: &mut impl FnMut(ScanProgress),
    ) -> Result<Project> {
        let root = path.to_path_buf();
        let meta_path = root.join(PROJECT_FILE);
        atomic::recover_replace(&root, &meta_path, PROJECT_META_RECOVERY)?;
        if !meta_path.is_file() {
            return Err(Error::NotAProject(root));
        }

        let raw = std::fs::read_to_string(&meta_path).map_err(|e| Error::io(&meta_path, e))?;
        let meta: ProjectMeta = serde_json::from_str(&raw)?;
        if meta.schema_version > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                found: meta.schema_version,
                supported: SCHEMA_VERSION,
            });
        }

        let on_network_share = paths::is_network_path(&root);
        // Probed rather than inferred: a read-only share must be detected on
        // open so the UI can say so, not on the first failed save.
        let read_only = !paths::is_writable(&root);

        let (index, index_path) = match index_path {
            Some(path) => (Index::open_at(path)?, path.to_path_buf()),
            None => (Index::open_for(&meta.id)?, paths::index_path(&meta.id)),
        };
        let mut project = Project {
            root,
            meta,
            index,
            index_path,
            on_network_share,
            read_only,
            peer: crate::peer::alias().to_owned(),
            world: Vec::new(),
        };

        // Before anything reads the folder: a write that was interrupted between
        // staging and rename left a full copy of a node file behind.
        project.sweep_staging();

        if project.index.is_empty()? {
            project.rescan_with(cancel, on_progress)?;
        } else {
            // The warm path. No progress reported because there is nothing slow
            // to report: this only reads files whose stamp moved, which on a
            // folder nobody has touched is none of them.
            project.reconcile()?;
        }
        Ok(project)
    }

    /// How long a `.part` file has to sit before we believe nobody is using it.
    ///
    /// Absurdly generous on purpose. Staging normally lives for a few
    /// milliseconds, so anything approaching this is certainly abandoned —
    /// while a file that *is* mid-write belongs to another Wobu on the share,
    /// and deleting it would make that user's rename fail and lose the save we
    /// were trying to protect. Being slow to tidy costs a few kilobytes; being
    /// quick costs someone their edit.
    const STAGING_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

    /// Remove staging files left behind by an interrupted write.
    ///
    /// A crash or a kill between `stage_and_rename`'s write and its rename
    /// leaves a `.part` in `.wobu/tmp`. Each one is a full copy of a node file,
    /// they accumulate silently, and on a synced share they replicate to
    /// everyone. Nothing ever reads them — the target was never touched — so
    /// they are pure litter.
    ///
    /// Deliberately infallible: this is housekeeping, and a project that opens
    /// fine except for a tidy-up is a project that should open.
    fn sweep_staging(&self) {
        let tmp = self.root.join(".wobu").join("tmp");
        let Ok(entries) = std::fs::read_dir(&tmp) else { return };
        let now = std::time::SystemTime::now();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("part") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(modified) = meta.modified() else { continue };
            // `duration_since` errors on a file stamped in the future — a clock
            // skew between machines on a share. Treat that as "recent", which
            // is the cautious reading.
            let Ok(age) = now.duration_since(modified) else { continue };
            if age >= Self::STAGING_GRACE {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn id(&self) -> Id {
        self.meta.id
    }

    /// The open project's `project.json`, already parsed.
    ///
    /// Exists so the shell can read the provider selection without re-opening
    /// and re-parsing the file on every Enhance. It is safe to share and holds
    /// no key — `docs/08-providers.md` is explicit that only the *selection*
    /// lives here, because project folders get put on shares.
    pub fn meta(&self) -> &ProjectMeta {
        &self.meta
    }

    /// Set the shared spend guardrail while preserving metadata fields this
    /// build does not understand. The full JSON is staged and published through
    /// the store's crash-recoverable metadata replacement path.
    pub fn set_spend_ceiling(&mut self, ceiling_usd_micros: Option<u64>) -> Result<()> {
        self.ensure_writable()?;
        let path = self.root.join(PROJECT_FILE);
        let bytes = std::fs::read(&path).map_err(|error| Error::io(&path, error))?;
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let object = value.as_object_mut().ok_or_else(|| {
            Error::Json(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "project.json must contain a JSON object",
            )))
        })?;
        object.insert(
            "spendCeilingUsdMicros".into(),
            ceiling_usd_micros.map_or(serde_json::Value::Null, |amount| amount.into()),
        );
        let encoded = serde_json::to_vec_pretty(&value)?;
        atomic::replace_metadata(&self.root, &path, PROJECT_META_RECOVERY, &encoded)?;
        self.meta.spend_ceiling_usd_micros = ceiling_usd_micros;
        Ok(())
    }

    /// Read the shared ceiling without opening the receipt ledger or a second
    /// local SQLite index. Display polling uses this O(1) half while admission
    /// uses [`spend_ledger`](Self::spend_ledger) below.
    pub fn spend_ceiling(path: &Path) -> Result<Option<u64>> {
        let meta_path = path.join(PROJECT_FILE);
        atomic::recover_replace(path, &meta_path, PROJECT_META_RECOVERY)?;
        if !meta_path.is_file() {
            return Err(Error::NotAProject(path.to_path_buf()));
        }
        let raw =
            std::fs::read_to_string(&meta_path).map_err(|error| Error::io(&meta_path, error))?;
        let meta: ProjectMeta = serde_json::from_str(&raw)?;
        Ok(meta.spend_ceiling_usd_micros)
    }

    /// Read the shared ceiling and every canonical receipt, without opening a
    /// second local SQLite index. Spend admission calls this while holding its
    /// cross-process ledger lock, and a network scan must not create two
    /// independent `Project` owners for one index merely to sum JSON files.
    pub fn spend_ledger(path: &Path) -> Result<(Option<u64>, Vec<Generation>)> {
        let ceiling = Self::spend_ceiling(path)?;
        let receipts = generations::read_all_strict(path)?;
        Ok((ceiling, receipts))
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn on_network_share(&self) -> bool {
        self.on_network_share
    }

    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            id: self.meta.id,
            name: self.meta.name.clone(),
            path: self.root.to_string_lossy().into_owned(),
            on_network_share: self.on_network_share,
            read_only: self.read_only,
            last_opened_at: Some(Utc::now()),
        }
    }

    // ── reading ──────────────────────────────────────────────────────────

    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>> {
        self.index.list_nodes()
    }

    /// Every explicit influence edge, read from the local derived index.
    pub fn node_links(&self) -> Result<Vec<LinkEdge>> {
        self.index.links()
    }

    /// A parser message with the project root taken out of it.
    ///
    /// `wobu-core` and the YAML parser both name the file by absolute path,
    /// which is right for a log and wrong for anything shown to a person: the
    /// leading half of it is the user's home directory, and this string ends up
    /// on screen and, sooner or later, pasted into a bug report. Everything
    /// else about the message is kept verbatim, because "expected a mapping at
    /// line 4" is the part that says what to fix.
    fn relative_message(&self, message: &str) -> String {
        let root = self.root.to_string_lossy();
        let stripped = message.replace(&format!("{root}/"), "");
        // Windows renders the same path with backslashes.
        stripped.replace(&format!("{}\\", root.replace('/', "\\")), "")
    }

    /// Node files that are on disk and cannot be parsed.
    ///
    /// Read alongside [`list_nodes`](Self::list_nodes) rather than folded into
    /// it: a truncated file may never have had a node row, so there is nothing
    /// to fold it onto.
    pub fn corrupt_files(&self) -> Result<Vec<CorruptFile>> {
        self.index.corrupt_files()
    }

    pub fn get_node(&self, id: Id) -> Result<Node> {
        let rel = self.index.rel_path_of(id)?.ok_or_else(|| Error::NoSuchNode(id.to_string()))?;
        let path = paths::from_rel_string(&self.root, &rel);
        let Some((text, _)) = atomic::read_stamped(&path)? else {
            // The index says this node exists and the file says otherwise. If
            // the whole folder has gone, believe the index: telling the user
            // their character does not exist, when it is sitting safely on a
            // NAS that happens to be unplugged, is both wrong and alarming.
            return Err(if self.is_present() {
                Error::NoSuchNode(id.to_string())
            } else {
                Error::Disconnected
            });
        };
        markdown::from_markdown(&text, &path)
    }

    /// The exact node version a long-running local task read before it started.
    pub fn node_stamp(&self, id: Id) -> Result<Option<atomic::Stamp>> {
        self.index.stamp_of(id)
    }

    /// Every node in the project, whole, for the influence engine.
    ///
    /// `wobu-influence` is pure by design: it borrows already-loaded `Node`s and
    /// does no IO, so that `prompt_compile` — which runs on every drag of a
    /// weight slider — stays sub-millisecond. Somebody has to hold those nodes,
    /// and this is the only place that can. The shell holds the `Project` under
    /// a mutex it must not do file IO beneath (`state.rs`), and a cache anywhere
    /// else would have to be told when a project closes and when a different one
    /// opens; here it is a field of the thing that *is* the open project, so a
    /// close drops it and no other project can ever be served from it.
    ///
    /// **Nothing here reads the project folder.** The nodes are rehydrated from
    /// the index's `doc` column, which lives in local app data, so the answer
    /// costs the same whether the world is on an SSD or on an SMB share that is
    /// currently unplugged — and the Inspector has to keep working in the second
    /// case, which is the whole promise of `docs/07-file-shares.md`. The index
    /// being one reconcile behind the folder is the same staleness the navigator
    /// already renders with, and the same event clears it.
    ///
    /// Built in full once, then patched: every writer of a node row records the
    /// id it touched, so a save, or a collaborator's edit arriving through
    /// `reconcile`, re-reads one row rather than the world. The full build is the
    /// state after an open, a rescan or an index rebuild — see `index::Touched`.
    ///
    /// The cost is real and worth stating: this holds the whole world in memory
    /// for as long as the project is open, at roughly the size of the Markdown
    /// it came from. A world of two thousand entities with a couple of kilobytes
    /// of prose each is a few megabytes. That is the price of an Inspector that
    /// does not stutter, and it is paid only by projects that open the panel.
    pub fn world_nodes(&mut self) -> Result<&[Node]> {
        match self.index.take_touched() {
            Touched::Everything => self.world = self.index.nodes()?,
            Touched::These(ids) => {
                for id in ids {
                    // Kept sorted by id, which is the order `Index::nodes`
                    // returns and the order `World` needs — it picks the Style
                    // Guide by lowest id, and a project must not resolve
                    // differently depending on which node was saved last.
                    let at = self.world.binary_search_by_key(&id, |n| n.id);
                    match (self.index.node(id)?, at) {
                        (Some(node), Ok(at)) => self.world[at] = node,
                        (Some(node), Err(at)) => self.world.insert(at, node),
                        (None, Ok(at)) => drop(self.world.remove(at)),
                        // Touched and gone and never held: a node created and
                        // deleted between two reads of this.
                        (None, Err(_)) => {}
                    }
                }
            }
        }
        Ok(&self.world)
    }

    /// Clone one complete, reconciled view for the static wiki renderer.
    ///
    /// The caller performs `reconcile` first so it can emit `world:changed` if
    /// the export noticed an external edit. Rendering and image copies happen
    /// after this snapshot releases the shell's project lock.
    pub fn wiki_snapshot(&mut self) -> Result<crate::wiki::WikiSnapshot> {
        let corrupt = self.corrupt_files()?.len();
        let conflicts = self.conflicts()?.len();
        if corrupt > 0 || conflicts > 0 {
            return Err(Error::ExportBlocked { corrupt, conflicts });
        }
        let nodes = self.world_nodes()?.to_vec();
        let assets = self.list_assets()?;
        Ok(crate::wiki::WikiSnapshot::new(self.root.clone(), self.meta.name.clone(), nodes, assets))
    }

    // ── writing ──────────────────────────────────────────────────────────

    pub fn create_node(
        &mut self,
        kind: NodeKind,
        name: &str,
        parent_id: Option<Id>,
    ) -> Result<Node> {
        self.ensure_writable()?;

        let def = kind_def(kind);
        if def.singleton && self.index.singleton_of(kind)?.is_some() {
            return Err(Error::Core(wobu_core::Error::DuplicateSingleton { kind: kind.as_str() }));
        }

        let mut node = Node::new(kind, name)?;
        node.parent_id = parent_id;

        // Two nodes of a kind may share a display name, but not a filename.
        let taken = self.index.slugs_in_kind(kind)?;
        node.slug = wobu_core::unique_slug(&node.slug, &|s| taken.iter().any(|t| t == s));

        node.validate()?;
        self.validate_parent(&node, parent_id)?;

        match self.write_node(&node, None)? {
            SaveOutcome::Saved(saved) => Ok(*saved),
            SaveOutcome::Conflict { conflict_path } => {
                Err(Error::AlreadyExists(paths::from_rel_string(&self.root, &conflict_path)))
            }
        }
    }

    /// Apply a fully staged style/subtree transfer to this project.
    ///
    /// The complete target graph, filenames and Markdown are validated before
    /// any node is written. Asset copies happen first and are content-addressed,
    /// so an unexpected failure can leave only harmless reusable blobs before
    /// the first entity. Once node publication starts, every guarded write is
    /// accounted for in the returned report; a conflict is never flattened
    /// into an apparent all-or-nothing success.
    pub fn apply_transfer(&mut self, bundle: TransferBundle) -> Result<TransferOutcome> {
        self.apply_transfer_with(bundle, |_| {})
    }

    fn apply_transfer_with(
        &mut self,
        bundle: TransferBundle,
        after_preflight: impl FnOnce(&mut Project),
    ) -> Result<TransferOutcome> {
        self.ensure_writable()?;
        let destination_root =
            std::fs::canonicalize(&self.root).map_err(|error| Error::io(&self.root, error))?;
        if destination_root == bundle.source_root || self.id() == bundle.source_project_id {
            return Err(Error::TransferSameProject);
        }

        let selected: std::collections::HashSet<Id> =
            bundle.nodes.iter().map(|node| node.id).collect();
        if !bundle.nodes.iter().any(|node| node.id == bundle.root_id) {
            return Err(Error::NoSuchNode(bundle.root_id.to_string()));
        }

        // Identity and slug allocation is an in-memory preflight. Singleton
        // identity is kept exactly so existing destination backlinks survive;
        // ordinary nodes receive fresh ids and collision-free paths.
        let existing = self.list_nodes()?;
        let mut taken: std::collections::HashMap<NodeKind, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for node in &existing {
            taken.entry(node.kind).or_default().insert(node.slug.clone());
        }
        let mut identities: std::collections::HashMap<Id, Node> = std::collections::HashMap::new();
        for source in &bundle.nodes {
            if kind_def(source.kind).singleton {
                let summary = existing
                    .iter()
                    .find(|node| node.kind == source.kind)
                    .ok_or_else(|| Error::NoSuchNode(source.kind.as_str().to_string()))?;
                identities.insert(source.id, self.get_node(summary.id)?);
                continue;
            }

            let mut identity = Node::new(source.kind, &source.name)?;
            while self.index.node(identity.id)?.is_some()
                || identities.values().any(|node| node.id == identity.id)
            {
                identity.id = wobu_core::new_id();
            }
            let kind_taken = taken.entry(source.kind).or_default();
            identity.slug = wobu_core::unique_slug(&identity.slug, &|slug| {
                kind_taken.contains(slug)
                    || self
                        .root
                        .join(format!("{NODES_DIR}/{}/{}.md", source.kind.dir(), slug))
                        .exists()
            });
            kind_taken.insert(identity.slug.clone());
            identities.insert(source.id, identity);
        }

        let id_map: std::collections::HashMap<Id, Id> =
            identities.iter().map(|(source, target)| (*source, target.id)).collect();
        let mut planned = Vec::with_capacity(bundle.nodes.len());
        for source in &bundle.nodes {
            let identity = identities
                .get(&source.id)
                .ok_or_else(|| Error::NoSuchNode(source.id.to_string()))?;
            let mut target = source.clone();
            target.id = identity.id;
            target.slug = identity.slug.clone();
            target.created_at = identity.created_at;
            target.parent_id = source.parent_id.and_then(|parent| id_map.get(&parent).copied());
            target.links.retain(|link| selected.contains(&link.to_id));
            for link in &mut target.links {
                link.to_id = id_map[&link.to_id];
            }
            target.enhanced_from = None;
            target.description_state = match &target.description {
                Some(description) if !description.is_empty() => DescriptionState::Edited,
                _ => DescriptionState::None,
            };
            target.touch();
            planned.push(target);
        }

        let planned_lookup: std::collections::HashMap<Id, (NodeKind, Option<Id>)> =
            planned.iter().map(|node| (node.id, (node.kind, node.parent_id))).collect();
        for node in &planned {
            node.validate()?;
            let lookup = |id: Id| {
                planned_lookup
                    .get(&id)
                    .copied()
                    .or_else(|| self.index.kind_and_parent(id).ok().flatten())
            };
            wobu_core::validate_parent(node, node.parent_id, &lookup)?;
            // Serialisation belongs in preflight too: a value that cannot be
            // represented in frontmatter must not be discovered half-way in.
            let _ = markdown::to_markdown(node)?;
        }
        for asset in &bundle.assets {
            crate::assets::validate_import(&asset.bytes)?;
            let hash = atomic::hash_bytes(&asset.bytes);
            if crate::assets::asset_id(&hash) != Some(asset.id) {
                return Err(Error::NoSuchAsset(asset.id.to_string()));
            }
        }
        for lora in &bundle.loras {
            crate::lora::validate(&lora.bytes)?;
            if atomic::hash_bytes(&lora.bytes) != lora.hash {
                return Err(Error::InvalidLora(
                    "a staged transfer weight no longer matches its content hash".into(),
                ));
            }
        }

        // Parents publish first. This also makes a singleton root's children
        // point at its preserved destination id before their own write.
        planned.sort_by_key(|node| {
            let mut depth = 0usize;
            let mut parent = node.parent_id;
            while let Some(id) = parent {
                let Some((_, next)) = planned_lookup.get(&id) else { break };
                depth += 1;
                parent = *next;
            }
            depth
        });
        let expected_stamps: std::collections::HashMap<Id, Option<atomic::Stamp>> = planned
            .iter()
            .map(|node| {
                let stamp = if kind_def(node.kind).singleton {
                    self.index.stamp_of(node.id)
                } else {
                    Ok(None)
                }?;
                Ok((node.id, stamp))
            })
            .collect::<Result<_>>()?;

        let imported_root_id = id_map[&bundle.root_id];
        let mut outcome = TransferOutcome {
            completed: false,
            root_id: bundle.root_id,
            imported_root_id,
            planned_node_count: planned.len(),
            applied_node_ids: Vec::new(),
            pending_node_ids: planned.iter().map(|node| node.id).collect(),
            reference_count: bundle.assets.len(),
            deduped_reference_count: 0,
            lora_count: bundle.loras.len(),
            deduped_lora_count: 0,
            dropped_external_link_count: bundle.external_link_count,
            replaced_singleton: bundle.replaces_singleton,
            conflict_paths: Vec::new(),
            failure: None,
        };

        // Production passes a no-op. The seam makes the race contract
        // deterministic in a unit test: a collaborator can win after every
        // byte/path preflight but before the first guarded publication.
        after_preflight(self);

        for asset in &bundle.assets {
            match self.import_asset(&asset.bytes, asset.kind) {
                Ok(imported) => outcome.deduped_reference_count += usize::from(imported.deduped),
                Err(error) => {
                    outcome.failure = Some(error.to_string());
                    return Ok(outcome);
                }
            }
        }

        for lora in &bundle.loras {
            match crate::lora::publish(&self.root, &lora.hash, &lora.bytes) {
                Ok((_, deduped)) => outcome.deduped_lora_count += usize::from(deduped),
                Err(error) => {
                    outcome.failure = Some(error.to_string());
                    return Ok(outcome);
                }
            }
        }

        for node in planned {
            let expected = expected_stamps.get(&node.id).and_then(Option::as_ref);
            match self.write_node(&node, expected) {
                Ok(SaveOutcome::Saved(saved)) => {
                    outcome.applied_node_ids.push(saved.id);
                    outcome.pending_node_ids.retain(|id| *id != saved.id);
                }
                Ok(SaveOutcome::Conflict { conflict_path }) => {
                    outcome.conflict_paths.push(conflict_path.clone());
                    outcome.failure = Some(format!(
                        "A destination node changed during transfer; the incoming version was parked as {conflict_path}."
                    ));
                    return Ok(outcome);
                }
                Err(error) => {
                    outcome.failure = Some(error.to_string());
                    return Ok(outcome);
                }
            }
        }
        outcome.completed = true;
        Ok(outcome)
    }

    /// Save an edited node, refusing to clobber a concurrent change.
    pub fn save_node(&mut self, mut node: Node) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        node.validate()?;
        self.validate_parent(&node, node.parent_id)?;
        node.touch();

        let expected = self.index.stamp_of(node.id)?;
        self.write_node(&node, expected.as_ref())
    }

    /// Publish a long-running task's result only if the node is still the
    /// version the task began from. The incoming version is parked as an
    /// ordinary conflict when somebody edited during the run.
    pub fn save_node_expected(
        &mut self,
        mut node: Node,
        expected: &atomic::Stamp,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        node.validate()?;
        self.validate_parent(&node, node.parent_id)?;
        node.touch();
        self.write_node(&node, Some(expected))
    }

    /// Persist the entity identity seed through the same guarded node write as
    /// every other shared edit. `None` explicitly clears the lock.
    pub fn set_locked_seed(&mut self, id: Id, seed: Option<u64>) -> Result<SaveOutcome> {
        let mut node = self.get_node(id)?;
        node.locked_seed = seed;
        self.save_node(node)
    }

    /// Land the result of an enhance, stamping what it was enhanced from.
    ///
    /// The stamp is the point. Without it there is nothing in the file that
    /// says what the model was shown, and "is this description still current"
    /// has no answer at all — so this is the only supported way to write a
    /// machine description, and it is a method on `Project` rather than
    /// something the caller assembles so that a stamp cannot be forgotten.
    ///
    /// `sources` is `wobu_influence::resolve`'s answer for this node, passed
    /// straight through. Taking it rather than recomputing it is what keeps
    /// staleness and prompt compilation talking about the same graph: there is
    /// one definition of "upstream" in Wobu and it is the walk that builds the
    /// prompt. The subject is dropped from it here rather than at the call
    /// site, because `resolve` includes the subject in its own stack and
    /// stamping a node against its own description would make it stale the
    /// instant it was written.
    ///
    /// **A hand-edited description is never overwritten silently.** `force` is
    /// the user answering the question the UI raised, not a default.
    pub fn accept_enhanced(
        &mut self,
        id: Id,
        description: Description,
        sources: &[Id],
        force: bool,
    ) -> Result<Enhanced> {
        self.ensure_writable()?;

        let mut node = self.get_node(id)?;
        if node.description_is_hand_written() && !force {
            return Ok(Enhanced::RefusedEdit(Box::new(node)));
        }

        let description = description.normalised_for(node.kind);
        let empty = description.is_empty();
        node.description = (!empty).then_some(description);
        // An enhance that produced nothing is not a fresh description, and
        // recording one would hide the failure behind a state that says the
        // node has been described.
        node.description_state =
            if empty { DescriptionState::None } else { DescriptionState::Fresh };

        let mut stamp = EnhanceStamp::default();
        for source in sources.iter().filter(|s| **s != id) {
            // A source the index cannot produce is one deleted between the
            // resolve and this call. Stamping a version we never read would
            // claim the description saw something it did not.
            if let Some(source_node) = self.index.node(*source)? {
                stamp.sources.push(SourceStamp {
                    node: *source,
                    version: crate::index::source_version(&source_node),
                });
            }
        }
        stamp.subject = crate::index::subject_version(&node);
        node.enhanced_from = Some(stamp);

        Ok(match self.save_node(node)? {
            SaveOutcome::Saved(node) => Enhanced::Saved(node),
            SaveOutcome::Conflict { conflict_path } => Enhanced::Conflict { conflict_path },
        })
    }

    pub fn move_node(&mut self, id: Id, new_parent_id: Option<Id>) -> Result<()> {
        let mut node = self.get_node(id)?;
        if node.parent_id == new_parent_id {
            return Ok(());
        }
        node.parent_id = new_parent_id;
        // save_node re-validates, which is where the cycle check happens.
        match self.save_node(node)? {
            SaveOutcome::Saved(_) => Ok(()),
            SaveOutcome::Conflict { conflict_path } => {
                Err(Error::AlreadyExists(paths::from_rel_string(&self.root, &conflict_path)))
            }
        }
    }

    // ── node links ───────────────────────────────────────────────────────

    /// Add an explicit influence edge, or replace the same `(target, role)`.
    ///
    /// The registry is checked here as well as in the picker. A webview can be
    /// stale after an app update, and an edge the current kind does not declare
    /// would otherwise be writable but impossible to add again after removal.
    pub fn add_node_link(
        &mut self,
        node_id: Id,
        to_id: Id,
        role: LinkRole,
        weight: Option<f32>,
        enabled: Option<bool>,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        if self.index.node(to_id)?.is_none() {
            return Err(Error::NoSuchNode(to_id.to_string()));
        }

        let mut node = self.get_node(node_id)?;
        self.require_link_role(&node, role)?;
        let mut link = Link::new(to_id, role);
        if let Some(weight) = weight {
            link.weight = weight;
        }
        if let Some(enabled) = enabled {
            link.enabled = enabled;
        }
        let link = link.clamped();

        match node.links.iter_mut().find(|item| item.to_id == to_id && item.role == role) {
            Some(existing) => *existing = link,
            None => node.links.push(link),
        }
        self.save_node(node)
    }

    /// Remove exactly one explicit edge. `parent_id` is deliberately not
    /// reachable here: it is an implicit relationship edited by `move_node`.
    pub fn remove_node_link(
        &mut self,
        node_id: Id,
        to_id: Id,
        role: LinkRole,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        let mut node = self.get_node(node_id)?;
        let before = node.links.len();
        node.links.retain(|item| item.to_id != to_id || item.role != role);
        if node.links.len() == before {
            return Err(Error::NoSuchNodeLink {
                target: to_id.to_string(),
                role: role.as_str().to_string(),
            });
        }
        self.save_node(node)
    }

    /// Re-weight or mute one explicit edge without replacing its other state.
    pub fn update_node_link(
        &mut self,
        node_id: Id,
        to_id: Id,
        role: LinkRole,
        weight: Option<f32>,
        enabled: Option<bool>,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        let mut node = self.get_node(node_id)?;
        let Some(link) =
            node.links.iter_mut().find(|item| item.to_id == to_id && item.role == role)
        else {
            return Err(Error::NoSuchNodeLink {
                target: to_id.to_string(),
                role: role.as_str().to_string(),
            });
        };
        if let Some(weight) = weight {
            link.weight = weight.clamp(0.0, 1.0);
        }
        if let Some(enabled) = enabled {
            link.enabled = enabled;
        }
        self.save_node(node)
    }

    /// Everything explicitly pointing at this node, from the local index.
    pub fn node_backlinks(&self, id: Id) -> Result<Vec<LinkEdge>> {
        if self.index.node(id)?.is_none() {
            return Err(Error::NoSuchNode(id.to_string()));
        }
        self.index.backlinks(id)
    }

    fn require_link_role(&self, node: &Node, role: LinkRole) -> Result<()> {
        if !kind_def(node.kind).default_link_roles.contains(&role) {
            return Err(Error::InvalidNodeLinkRole {
                kind: node.kind.as_str().to_string(),
                role: role.as_str().to_string(),
            });
        }
        Ok(())
    }

    /// Delete a node, promoting any children to its parent and stripping the
    /// influence edges that pointed at it.
    ///
    /// Deleting a Region should not silently take its Cities with it, and
    /// refusing outright would make the user delete a subtree leaf by leaf.
    ///
    /// The inbound links matter just as much: ULIDs are never reused, so a link
    /// left pointing at a deleted node is dead weight in someone's frontmatter
    /// forever, and the influence engine would resolve it into an empty layer
    /// card rather than nothing at all.
    pub fn delete_node(&mut self, id: Id) -> Result<()> {
        self.ensure_writable()?;

        let node = self.get_node(id)?;
        if kind_def(node.kind).singleton {
            return Err(Error::Core(wobu_core::Error::DuplicateSingleton {
                kind: node.kind.as_str(),
            }));
        }

        for child_id in self.index.children_of(id)? {
            let mut child = self.get_node(child_id)?;
            child.parent_id = node.parent_id;
            self.save_node(child)?;
        }

        // Collected first: each save_node below rewrites the index, and holding
        // a borrow across that would be reading a table we are mutating.
        let referrers: Vec<Id> =
            self.index.backlinks(id)?.into_iter().map(|edge| edge.from_id).collect();
        for from_id in referrers {
            // A referrer that is itself already gone is not an error — deleting
            // two linked nodes in either order must work.
            let Ok(mut referrer) = self.get_node(from_id) else { continue };
            referrer.links.retain(|link| link.to_id != id);
            self.save_node(referrer)?;
        }

        if let Some(rel) = self.index.rel_path_of(id)? {
            let path = paths::from_rel_string(&self.root, &rel);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(&path, e)),
            }
        }
        self.index.remove_node(id)?;
        Ok(())
    }

    fn write_node(&mut self, node: &Node, expected: Option<&atomic::Stamp>) -> Result<SaveOutcome> {
        let rel = self.rel_path(node);
        let path = paths::from_rel_string(&self.root, &rel);
        let text = markdown::to_markdown(node)?;

        // Two people saving the same words is not a conflict — nobody's text is
        // at risk — but every save re-stamps `updated_at`, so their bytes never
        // match. `guarded_write` can only compare bytes, so left to itself it
        // parks a `.conflict-` sibling whose sole difference from the winner is
        // a timestamp. That is worse than useless: it teaches people that
        // conflict files are noise, right up until one of them matters.
        if let Some(expected) = expected
            && let Some((theirs, stamp)) = self.same_words_on_disk(node, &path, expected)?
        {
            self.index.upsert_node(&theirs, &rel, &stamp)?;
            return Ok(SaveOutcome::Saved(Box::new(theirs)));
        }

        match atomic::guarded_write(&self.root, &path, &text, expected, &self.peer)? {
            WriteOutcome::Written(stamp) => {
                self.index.upsert_node(node, &rel, &stamp)?;
                Ok(SaveOutcome::Saved(Box::new(node.clone())))
            }
            WriteOutcome::Conflict { conflict_path, .. } => {
                // Pull the winner's version into the index so the UI shows what
                // is actually on disk while the conflict card is open.
                if let Ok(Some((text, stamp))) = atomic::read_stamped(&path)
                    && let Ok(theirs) = markdown::from_markdown(&text, &path)
                {
                    self.index.upsert_node(&theirs, &rel, &stamp)?;
                }
                let rel_conflict = conflict_path
                    .strip_prefix(&self.root)
                    .map(paths::to_rel_string)
                    .unwrap_or_else(|_| paths::to_rel_string(&conflict_path));
                Ok(SaveOutcome::Conflict { conflict_path: rel_conflict })
            }
        }
    }

    /// The file changed under us, but it says exactly what we were about to say.
    ///
    /// Returns the on-disk node and its stamp when it matches ours in every
    /// field but `updated_at`. The caller adopts it: the user's words are on
    /// disk, so the save has effectively happened, and there is nothing for a
    /// conflict card to offer a choice between.
    ///
    /// Comparison is by re-serialising our node with *their* timestamp and
    /// requiring the bytes to match theirs exactly. That is deliberately strict
    /// — a file hand-edited into different formatting falls through to the
    /// normal conflict path, which is the safe direction to be wrong in.
    fn same_words_on_disk(
        &self,
        node: &Node,
        path: &Path,
        expected: &atomic::Stamp,
    ) -> Result<Option<(Node, atomic::Stamp)>> {
        // The cheap filter first, so the common case — nothing changed — costs
        // one `stat` rather than a read and a parse.
        match atomic::peek(path)? {
            Some((mtime, size)) if mtime == expected.mtime_ms && size == expected.size => {
                return Ok(None);
            }
            // Deleted under us. `guarded_write` recreates, which is right.
            None => return Ok(None),
            _ => {}
        }

        let Some((text, stamp)) = atomic::read_stamped(path)? else { return Ok(None) };
        if stamp.hash == expected.hash {
            return Ok(None);
        }
        let Ok(theirs) = markdown::from_markdown(&text, path) else { return Ok(None) };

        let mut ours = node.clone();
        ours.updated_at = theirs.updated_at;
        match markdown::to_markdown(&ours) {
            Ok(rendered) if rendered == text => Ok(Some((theirs, stamp))),
            _ => Ok(None),
        }
    }

    fn rel_path(&self, node: &Node) -> String {
        format!("{NODES_DIR}/{}/{}.md", node.kind.dir(), node.slug)
    }

    fn validate_parent(&self, node: &Node, parent_id: Option<Id>) -> Result<()> {
        let lookup = |id: Id| self.index.kind_and_parent(id).ok().flatten();
        wobu_core::validate_parent(node, parent_id, &lookup)?;
        Ok(())
    }

    /// Whether the folder this project was opened from is still reachable.
    ///
    /// Cheap, and checked on the write path rather than cached, because the
    /// whole failure mode is that it changes underneath a running session.
    pub fn is_present(&self) -> bool {
        paths::project_is_present(&self.root)
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        // Checked before every write, not just reported after one fails. A
        // write into an unmounted share's leftover mountpoint *succeeds* — it
        // lands on the local disk under the mount, invisible to everyone and
        // destined to be shadowed the moment the share comes back. Refusing up
        // front is the only way that edit stays recoverable.
        if !self.is_present() {
            return Err(Error::Disconnected);
        }
        Ok(())
    }

    // ── assets ───────────────────────────────────────────────────────────

    /// Bring an image into `assets/originals/` and index it.
    ///
    /// Goes through `ensure_writable` like every other write, for the same
    /// reason: an import into an unmounted share's leftover mountpoint would
    /// *succeed*, landing on local disk under the mount where nobody — the
    /// importer included, once the share returns — will ever find it.
    ///
    /// Content addressing means this cannot conflict, so unlike `save_node`
    /// there is no outcome enum: it either lands or it fails. What it does
    /// report is whether the bytes were already there.
    ///
    /// **No thumbnail is made here**, and that is deliberate rather than an
    /// omission. Nothing in this method decodes a pixel — that is the whole of
    /// why an import can accept a file a sync client has not finished copying —
    /// and a decode is both the expensive step and the one that can fail on a
    /// blob that is otherwise perfectly storable. Folding it in would let a
    /// half-copied file turn a successful import into a failed one. Callers
    /// follow up with [`ensure_thumb`](Self::ensure_thumb), off whichever thread
    /// they are not drawing the window with; see `crate::thumbs`.
    pub fn import_asset(&mut self, bytes: &[u8], kind: AssetKind) -> Result<ImportedAsset> {
        self.import_asset_with(bytes, kind, &Cancel::new())
    }

    /// `import_asset`, stoppable — see [`assets::import_with`].
    pub fn import_asset_with(
        &mut self,
        bytes: &[u8],
        kind: AssetKind,
        cancel: &Cancel,
    ) -> Result<ImportedAsset> {
        let root = self.asset_import_root()?;
        let imported = assets::import_with(&root, bytes, kind, cancel)?;
        self.record_import(&imported)?;
        Ok(imported)
    }

    /// Validate an import while the shell holds its project mutex, then hand
    /// the root out by value for the expensive read/hash/write phase.
    pub fn asset_import_root(&self) -> Result<PathBuf> {
        self.verify_writable()?;
        Ok(self.root.clone())
    }

    /// Recheck the write preconditions immediately before a staged commit.
    pub fn verify_writable(&self) -> Result<()> {
        self.ensure_writable()
    }

    /// Commit an already-published import to the machine-local index.
    ///
    /// The import worker has just published the content-addressed blob and the
    /// shell verifies its project ticket before calling this method. The one
    /// bounded project-presence check here catches a share that disconnected
    /// during that work; source/blob I/O remains outside the global mutex.
    pub fn record_import(&mut self, imported: &ImportedAsset) -> Result<()> {
        self.verify_writable()?;
        self.index.upsert_asset(&imported.asset)
    }

    /// The same, for a file the user dropped or picked rather than bytes the
    /// webview already holds.
    pub fn import_asset_file(&mut self, path: &Path, kind: AssetKind) -> Result<ImportedAsset> {
        self.import_asset_file_with(path, kind, &Cancel::new())
    }

    /// `import_asset_file`, stoppable.
    ///
    /// The one import that can take minutes rather than milliseconds: a 300 MB
    /// scan on a share is read a chunk at a time from a mount that may be
    /// wedged, and #30 asks for a way out of that. Nothing here makes the *UI*
    /// responsive — the call still blocks whichever thread it is on — so a
    /// caller that must not stall has to run it off the one drawing the window
    /// and hold the token; the token is what makes doing so worth anything.
    pub fn import_asset_file_with(
        &mut self,
        path: &Path,
        kind: AssetKind,
        cancel: &Cancel,
    ) -> Result<ImportedAsset> {
        // Read whole rather than streamed: the hash needs every byte anyway,
        // and it has to be hashed before we know where the file goes.
        let bytes = assets::read_cancellable(path, cancel)?;
        self.import_asset_with(&bytes, kind, cancel)
    }

    pub fn list_assets(&self) -> Result<Vec<Asset>> {
        self.index.list_assets()
    }

    /// Mesh metadata is deliberately absent from the eager image index. This
    /// listing is called only when the 3D tab opens and reads no mesh body.
    pub fn list_meshes(&self) -> Vec<wobu_core::MeshAsset> {
        assets::scan_meshes(&self.root)
    }

    /// Store the GLB half of a future mesh job without coupling persistence to
    /// any provider adapter. The returned id is what `params.meshOutput` records.
    pub fn store_mesh_glb(&mut self, bytes: &[u8]) -> Result<crate::StoredMesh> {
        self.ensure_writable()?;
        assets::store_mesh_glb(&self.root, bytes)
    }

    /// Project-wide reference/cover usage for filtering and orphan discovery.
    ///
    /// The full nodes come from the local index-backed world cache, never from
    /// opening every Markdown file over the project share. Tags deliberately
    /// come from the linked node: assets have no mutable metadata document of
    /// their own, while node tags are canonical and already shared.
    pub fn asset_usages(&mut self) -> Result<Vec<AssetUsage>> {
        let mut out = Vec::new();
        for node in self.world_nodes()? {
            let mut by_asset: std::collections::BTreeMap<Id, Vec<AssetUsageRole>> =
                std::collections::BTreeMap::new();
            for link in &node.asset_links {
                by_asset.entry(link.asset_id).or_default().push(AssetUsageRole {
                    role: link.role,
                    weight: link.weight,
                    enabled: link.enabled,
                });
            }
            if let Some(cover) = node.cover_asset_id {
                by_asset.entry(cover).or_default();
            }
            for (asset_id, roles) in by_asset {
                out.push(AssetUsage {
                    asset_id,
                    node_id: node.id,
                    node_name: node.name.clone(),
                    node_kind: node.kind,
                    node_tags: node.tags.clone(),
                    roles,
                    cover: node.cover_asset_id == Some(asset_id),
                });
            }
        }
        Ok(out)
    }

    /// Permanently remove one unreferenced blob and its derived thumbnail.
    ///
    /// A UI confirmation is necessary but not sufficient: the reference check
    /// lives here and reads canonical Markdown afresh so a stale local index
    /// cannot authorise deletion. There is no cross-process transaction over a
    /// shared folder, so another editor could still write after that read; the
    /// destructive path makes the safety window as narrow as the filesystem
    /// permits and never relies on the long-lived world cache.
    ///
    /// Generation receipts deliberately do not block deletion. Issue #28
    /// defines an orphan as an asset with no node use; generated images are
    /// disposable, while their immutable receipt remains as an honest record
    /// whose output is now missing. The confirmation sheet says that plainly.
    /// Missing files are accepted because reconcile may have observed their
    /// disappearance after the index snapshot; dropping the stale row still
    /// achieves the requested end state.
    pub fn delete_asset(&mut self, id: Id) -> Result<()> {
        self.ensure_writable()?;
        let asset = self.get_asset(id)?.ok_or_else(|| Error::NoSuchAsset(id.to_string()))?;
        let users = self.canonical_asset_users(id)?;
        if users > 0 {
            return Err(Error::AssetInUse { asset: id.to_string(), nodes: users });
        }

        if let Some(thumb) = &asset.thumb_path {
            remove_asset_file(&paths::from_rel_string(&self.root, thumb))?;
        }
        remove_asset_file(&paths::from_rel_string(&self.root, &asset.rel_path))?;
        self.index.remove_asset_by_rel_path(&asset.rel_path)?;
        Ok(())
    }

    /// Count use from the source-of-truth files, bypassing every index stamp
    /// shortcut. A malformed file makes deletion fail closed: until it can be
    /// read, we cannot prove that it does not contain the asset id.
    fn canonical_asset_users(&self, id: Id) -> Result<usize> {
        let mut users = 0;
        // Conflict siblings are included. They are not active nodes today, but
        // the user may resolve one by keeping it tomorrow; deleting an asset it
        // names would make that resolution manufacture a dangling reference.
        for (_, path) in self.markdown_files() {
            let Some((text, _)) = atomic::read_stamped(&path)? else { continue };
            let node = markdown::from_markdown(&text, &path)?;
            if node.cover_asset_id == Some(id)
                || node.asset_links.iter().any(|link| link.asset_id == id)
            {
                users += 1;
            }
        }
        Ok(users)
    }

    pub fn get_asset(&self, id: Id) -> Result<Option<Asset>> {
        self.index.asset(id)
    }

    // ── generations ──────────────────────────────────────────────────────

    /// Append one immutable generation record to the project.
    ///
    /// The JSON lands before the index row. If local SQLite is unavailable at
    /// that point the result is still safely recorded, and the next open or
    /// rebuild discovers it from the folder. There is deliberately no update
    /// counterpart: a retry or another collaborator's attempt gets a new ULID.
    pub fn record_generation(&mut self, generation: Generation) -> Result<Generation> {
        self.ensure_writable()?;
        if self.index.node(generation.node_id)?.is_none() {
            return Err(Error::NoSuchNode(generation.node_id.to_string()));
        }
        self.append_generation(generation)
    }

    /// Append a replay receipt even when its historical subject has since been
    /// deleted. Ordinary generation still requires a live node; this archival
    /// path is allowed only when `replayOf` names an immutable receipt already
    /// in this project.
    pub fn record_replay_generation(&mut self, generation: Generation) -> Result<Generation> {
        self.ensure_writable()?;
        let source = generation
            .params
            .get("replayOf")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Id::from_string(value).ok())
            .ok_or_else(|| Error::MalformedGeneration {
                path: PathBuf::from(generation.rel_path()),
                reason: "replay receipt has no valid replayOf id".to_string(),
            })?;
        if self.get_generation(source)?.is_none() {
            return Err(Error::NoSuchNode(format!("replay source {source}")));
        }
        self.append_generation(generation)
    }

    fn append_generation(&mut self, generation: Generation) -> Result<Generation> {
        for asset_id in &generation.output_asset_ids {
            self.require_asset(*asset_id)?;
        }

        let (rel, stamp) = generations::write(&self.root, &generation)?;
        self.index.upsert_generation(&generation, &rel, &stamp)?;
        Ok(generation)
    }

    /// A node's generation history for the Concepts grid, newest first.
    pub fn list_generations(&self, node_id: Id) -> Result<Vec<Generation>> {
        self.index.generations_for_node(node_id)
    }

    /// Every immutable receipt for the project-wide History browser, newest
    /// first. Unlike spend reconstruction this is a UI read and can use the
    /// reconciled disposable index rather than reopening every month shard.
    pub fn generation_history(&self) -> Result<Vec<Generation>> {
        self.index.generations_all()
    }

    /// Every immutable receipt, for reconstructing project spend.
    pub fn list_all_generations(&self) -> Result<Vec<Generation>> {
        generations::read_all_strict(&self.root)
    }

    /// One indexed generation, without reading across the project share.
    pub fn get_generation(&self, id: Id) -> Result<Option<Generation>> {
        self.index.generation(id)
    }

    /// Attach an asset to a node in a role.
    ///
    /// Goes through `save_node` like any other edit to the node, which is the
    /// point: attaching a reference is an edit to the node's Markdown, so it
    /// has to lose the same save races and park the same conflict sibling as
    /// typing in the notes field. A private write path here would be a way to
    /// clobber a collaborator that the editor does not have.
    ///
    /// Linking the same asset in the same role twice replaces the weight rather
    /// than adding a second link — the user dropped the same picture on the
    /// same shelf, and two identical rows is not something they can then
    /// distinguish in the UI to remove one of.
    pub fn link_asset(
        &mut self,
        node_id: Id,
        asset_id: Id,
        role: AssetRole,
        weight: Option<f32>,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        self.require_asset(asset_id)?;

        let mut node = self.get_node(node_id)?;
        let mut link = AssetRef::new(asset_id, role);
        if let Some(weight) = weight {
            link.weight = weight;
        }
        let link = link.clamped();

        match node.asset_links.iter_mut().find(|l| l.asset_id == asset_id && l.role == role) {
            Some(existing) => existing.weight = link.weight,
            None => node.asset_links.push(link),
        }
        self.save_node(node)
    }

    /// Detach an asset from a node.
    ///
    /// **Never touches the blob.** Assets are content-addressed and shared, so
    /// the file behind this link may be the cover of another node, a reference
    /// on three more, and identical to the one a collaborator imported last
    /// week. Removing the last link is not evidence that anybody wants the
    /// picture gone, and deleting it would be unrecoverable in a way that
    /// unlinking is not.
    pub fn unlink_asset(
        &mut self,
        node_id: Id,
        asset_id: Id,
        role: AssetRole,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;

        let mut node = self.get_node(node_id)?;
        let before = node.asset_links.len();
        node.asset_links.retain(|l| !(l.asset_id == asset_id && l.role == role));
        if node.asset_links.len() == before {
            return Err(Error::NoSuchAssetLink {
                asset: asset_id.to_string(),
                role: role.as_str().to_owned(),
            });
        }
        self.save_node(node)
    }

    /// Change a link's weight, its enabled flag, or both.
    ///
    /// Both arguments are optional and `None` means "leave it": the slider and
    /// the mute toggle are separate controls, and sending the whole link back
    /// from either would let a stale copy of the other overwrite what the user
    /// just did with it.
    pub fn update_asset_link(
        &mut self,
        node_id: Id,
        asset_id: Id,
        role: AssetRole,
        weight: Option<f32>,
        enabled: Option<bool>,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;

        let mut node = self.get_node(node_id)?;
        let Some(link) =
            node.asset_links.iter_mut().find(|l| l.asset_id == asset_id && l.role == role)
        else {
            return Err(Error::NoSuchAssetLink {
                asset: asset_id.to_string(),
                role: role.as_str().to_owned(),
            });
        };

        let mut updated = link.clone();
        if let Some(weight) = weight {
            updated.weight = weight;
        }
        if let Some(enabled) = enabled {
            updated.enabled = enabled;
        }
        *link = updated.clamped();
        self.save_node(node)
    }

    /// Choose (or clear) the image that represents a node.
    ///
    /// Deliberately independent of the links: a cover is what a card shows, and
    /// making it imply a link would mean choosing a thumbnail quietly changed
    /// what the influence engine sends to a backend.
    pub fn set_cover_asset(&mut self, node_id: Id, asset_id: Option<Id>) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        if let Some(asset_id) = asset_id {
            self.require_asset(asset_id)?;
        }

        let mut node = self.get_node(node_id)?;
        node.cover_asset_id = asset_id;
        self.save_node(node)
    }

    /// Refuse an asset id the project does not have.
    ///
    /// Read from the index, which is a complete description of
    /// `assets/originals/` after any open or reconcile — and, unlike the
    /// folder, answers without a stat over the share on every keystroke of a
    /// weight slider.
    fn require_asset(&self, asset_id: Id) -> Result<()> {
        if self.index.asset(asset_id)?.is_none() {
            return Err(Error::NoSuchAsset(asset_id.to_string()));
        }
        Ok(())
    }

    // ── thumbnails ───────────────────────────────────────────────────────
    //
    // The argument for all of it — why the files are in the project folder
    // rather than in a local cache, and why two builds may disagree about their
    // contents without anything being lost — is in `crate::thumbs`.

    /// Every blob the index has no thumbnail recorded against.
    ///
    /// What a project that arrived over sync hands to
    /// [`thumbs::ensure_all`](crate::thumbs::ensure_all). Read out in one go and
    /// returned by value on purpose: the pass that follows takes seconds to
    /// minutes and must run with nothing of this project's held, which is the
    /// same rule the shell's project mutex is built on.
    ///
    /// Filtered from the index rather than from a directory listing, because
    /// `assets::describe_at` already stats the thumbnail when it describes a
    /// blob — so this costs one local SQLite read where the honest-looking
    /// version costs one round trip per asset over SMB.
    pub fn missing_thumbs(&self) -> Result<Vec<crate::thumbs::ThumbTarget>> {
        Ok(self
            .index
            .list_assets()?
            .into_iter()
            .filter(|asset| asset.thumb_path.is_none())
            .map(|asset| crate::thumbs::ThumbTarget {
                asset_id: asset.id,
                hash: asset.hash,
                rel_path: asset.rel_path,
            })
            .collect())
    }

    /// The immutable facts needed to draw one asset's thumbnail.
    ///
    /// Returned by value so source I/O and pixel work can happen after the
    /// project mutex has been released.
    pub fn thumb_target(&self, id: Id) -> Result<Option<crate::thumbs::ThumbTarget>> {
        Ok(self.index.asset(id)?.map(|asset| crate::thumbs::ThumbTarget {
            asset_id: asset.id,
            hash: asset.hash,
            rel_path: asset.rel_path,
        }))
    }

    /// Whether a thumbnail missing from disk may be written right now.
    ///
    /// Existing thumbnails remain usable in read-only projects, so callers
    /// carry this answer beside the target rather than rejecting the request.
    pub fn can_write_thumb(&self) -> bool {
        self.ensure_writable().is_ok()
    }

    /// Record thumbnail results already proved present outside the shell lock.
    ///
    /// Asset identity is rechecked in the local index before mutation. There is
    /// intentionally no filesystem access here: checking immediately before
    /// this call offers no stronger guarantee against a collaborator deleting
    /// a disposable thumbnail immediately after it, and would block every
    /// index query on a share stat for each result.
    pub fn record_thumb_targets(&mut self, targets: &[crate::thumbs::ThumbTarget]) -> Result<()> {
        for target in targets {
            let Some(mut asset) = self.index.asset(target.asset_id)? else { continue };
            if asset.hash != target.hash || asset.rel_path != target.rel_path {
                continue;
            }
            let rel = crate::thumbs::rel_path(&asset.hash);
            if asset.thumb_path.as_deref() == Some(rel.as_str()) {
                continue;
            }
            asset.thumb_path = Some(rel);
            self.index.upsert_asset(&asset)?;
        }
        Ok(())
    }

    /// Make one blob's thumbnail if the folder has not got one, and record it.
    ///
    /// The lazy half of #25, and the one a grid tile reaches for. `Ok(None)`
    /// covers all three ways there is legitimately no thumbnail and never will
    /// be one right now — no such asset, a folder nothing can be written into,
    /// and a blob whose pixels will not decode — because each of those is a tile
    /// that falls back to a placeholder rather than an error the user can act
    /// on. Anything else is a real failure and is reported.
    pub fn ensure_thumb(&mut self, id: Id, cancel: &Cancel) -> Result<Option<String>> {
        let Some(asset) = self.index.asset(id)? else { return Ok(None) };

        // Only when one has to be *made*. A read-only share that already holds
        // thumbnails — the ordinary way a folder is published — still serves
        // them, and refusing here would blank the grid on exactly that project.
        if !crate::thumbs::exists(&self.root, &asset.hash) && self.ensure_writable().is_err() {
            return Ok(None);
        }

        match crate::thumbs::ensure(&self.root, &asset.hash, &asset.rel_path, cancel) {
            Ok(thumb) => {
                self.record_thumbs(&[id])?;
                Ok(Some(thumb.rel_path))
            }
            Err(Error::Undecodable { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Note that these blobs now have thumbnails in the folder.
    ///
    /// Every id is re-checked against the disk rather than trusted, because the
    /// caller of the bulk pass is holding a list assembled before it ran and the
    /// index is the thing the UI reads: a row claiming a thumbnail that is not
    /// there is a broken image in the grid, which is worse than the null it
    /// replaced.
    pub fn record_thumbs(&mut self, ids: &[Id]) -> Result<()> {
        for id in ids {
            let Some(mut asset) = self.index.asset(*id)? else { continue };
            let rel = crate::thumbs::rel_path(&asset.hash);
            if !crate::thumbs::exists(&self.root, &asset.hash) {
                continue;
            }
            if asset.thumb_path.as_deref() == Some(rel.as_str()) {
                continue;
            }
            asset.thumb_path = Some(rel);
            self.index.upsert_asset(&asset)?;
        }
        Ok(())
    }

    // ── replication ──────────────────────────────────────────────────────
    //
    // The store's half of M3. Every method here is about one peer, named by the
    // 64-hex `EndpointId` `wobu-sync` gets back from TLS — never by an alias,
    // which is a display name and may not decide anything. Nothing below opens a
    // socket, and `wobu-store` must never depend on `wobu-sync`: a project has to
    // open on a machine that will never sync, and the index has to be readable by
    // a build with no transport in it at all.
    //
    // The argument for all of it is in `crate::apply`.

    /// Fold a peer's version of some nodes into the folder.
    ///
    /// The one entry point that lets a remote machine change this project, and
    /// it is bounded: a node file is only ever *written* when our copy is
    /// byte-identical to what the two of us last agreed on, so the write cannot
    /// destroy anything that is not also on the peer's disk. Everything else
    /// parks beside ours. See [`crate::apply`] for the table and the argument.
    ///
    /// Refuses while the folder is unreachable, through the same check every
    /// other write takes. That is not a formality here: a write into an
    /// unmounted share's leftover mountpoint succeeds, landing on the local disk
    /// under the mount where nobody will ever see it — and a sync is the one
    /// writer that runs without a person watching, so it is the one most likely
    /// to do it a hundred times before anyone notices.
    ///
    /// The caller emits `world:changed` once per batch when
    /// [`ApplyReport::changed_the_folder`] says so. This crate cannot emit and
    /// must not learn how.
    ///
    /// [`ApplyReport::changed_the_folder`]: crate::apply::ApplyReport::changed_the_folder
    pub fn apply_from_peer(
        &mut self,
        peer_id: &str,
        incoming: &[apply::Incoming],
    ) -> Result<apply::ApplyReport> {
        self.ensure_writable()?;
        apply::apply(&self.root, &self.index, peer_id, incoming)
    }

    /// What a peer's manifest implies, without touching anything.
    ///
    /// `&self` rather than `&mut self`, and that is load-bearing rather than
    /// tidy: a plan is a proposal, and the folder is not entitled to change
    /// because two machines exchanged a list of hashes.
    pub fn plan_against_peer(&self, peer_id: &str, remote: &[(Id, String)]) -> Result<apply::Plan> {
        apply::plan(&self.index, peer_id, remote)
    }

    /// This project's half of a manifest: every node it holds, and its hash.
    ///
    /// Sorted by id so two runs against an unchanged folder produce identical
    /// bytes on the wire, which is what lets a peer skip a comparison it has
    /// already made.
    pub fn manifest(&self) -> Result<Vec<(Id, String)>> {
        let mut out: Vec<(Id, String)> = self.index.node_hashes()?.into_iter().collect();
        out.sort_unstable_by_key(|(id, _)| *id);
        Ok(out)
    }

    /// One node packaged the way a peer will receive it, straight off the disk.
    ///
    /// The bytes are read from the file rather than re-rendered from the index,
    /// so what a peer is sent is what a person would see if they opened the
    /// folder. Re-rendering would be a second definition of the file's contents
    /// and the two would eventually disagree — at which point the hash in the
    /// manifest describes one of them and the payload is the other, and the
    /// receiving end fast-forwards onto a version nobody has.
    ///
    /// `Ok(None)` means the index does not know the node, or its file is not
    /// there. Neither is an error: a node deleted between a manifest and a
    /// request is ordinary, and there is nothing to send.
    pub fn outgoing(&self, id: Id) -> Result<Option<apply::Outgoing>> {
        let Some(rel) = self.index.rel_path_of(id)? else { return Ok(None) };
        let path = paths::from_rel_string(&self.root, &rel);
        let Some((text, stamp)) = atomic::read_stamped(&path)? else { return Ok(None) };
        let slug = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Some(apply::Outgoing { node_id: id, slug, text, hash: stamp.hash }))
    }

    /// Record that this project and a peer now hold the same bytes for these
    /// nodes.
    ///
    /// **Only ever called for an acknowledgement, never for a send.** A base is
    /// a claim that a specific machine holds specific bytes, and the next sync
    /// will fast-forward on that claim without asking anybody. Moving it when we
    /// put a node on the wire, rather than when the peer said it landed, would
    /// make a dropped connection into a base that describes a file the peer
    /// never received — and the edit they make next would then read as a
    /// one-sided change and overwrite ours.
    ///
    /// One transaction, so a crash leaves the bases as of the end of a sync
    /// rather than some prefix of them.
    pub fn record_agreed(&self, peer_id: &str, agreed: &[(Id, String)]) -> Result<()> {
        self.index.record_bases(peer_id, agreed)
    }

    /// Forget everything agreed *and everything refused* with a peer: a share
    /// revoked, a ticket rotated, an identity that will not be dialled again.
    ///
    /// Cheap to be wrong about in one direction only. Forgetting too much costs
    /// a re-compare and some conflict cards; forgetting too little leaves a base
    /// attributed to a machine nobody has any reason to still trust, or a
    /// refusal that would quietly withhold a version from a machine that has
    /// since been re-admitted. Reach for it whenever unsure.
    pub fn forget_peer(&self, peer_id: &str) -> Result<()> {
        self.index.forget_peer(peer_id)
    }

    // ── conflicts ────────────────────────────────────────────────────────

    /// Every unresolved conflict sibling in the folder, newest first.
    ///
    /// Recomputed from the directory rather than cached in the index, and that
    /// is deliberate. A sibling can arrive from a *different machine* — a
    /// collaborator's Wobu parking their loser onto the share — so there is no
    /// event on this side that could keep a cached list honest. The walk is one
    /// directory listing, which `docs/07-file-shares.md` establishes as the
    /// cheap half; the reads are two files per conflict, and a folder with no
    /// conflicts, which is almost all of them, pays nothing at all.
    pub fn conflicts(&self) -> Result<Vec<Conflict>> {
        let mut out = Vec::new();

        for (rel, path) in self.conflict_files() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(conflict::parse)
            else {
                continue;
            };
            // Unreadable bytes rather than a missing file: a sibling half-copied
            // by a sync client is exactly the thing not to drop off the list.
            let Some((parked, _)) = atomic::read_stamped(&path)? else { continue };

            let target = path.with_file_name(name.target_file_name());
            let target_rel = target
                .strip_prefix(&self.root)
                .map(paths::to_rel_string)
                .unwrap_or_else(|_| paths::to_rel_string(&target));
            // A node file that has since been deleted leaves the sibling as the
            // only surviving copy, so it is listed with an empty other side
            // rather than hidden.
            let (current, current_hash) = match atomic::read_stamped(&target)? {
                Some((text, stamp)) => (text, stamp.hash),
                None => (String::new(), String::new()),
            };
            let node = self.index.node_at_rel_path(&target_rel)?;

            out.push(Conflict {
                mine: name.peer.as_deref() == Some(self.peer.as_str()),
                rel_path: rel,
                node_rel_path: target_rel,
                node_id: node.as_ref().map(|(id, _)| *id),
                node_name: node.map(|(_, name)| name),
                user: name.peer,
                saved_at: name.saved_at,
                parked,
                current,
                current_hash,
            });
        }

        // Newest first, so the card a user is most likely to be looking for is
        // the one at the top. Unlabelled siblings sort last rather than first —
        // `None` would otherwise win a descending sort and put the ones we know
        // least about in front.
        out.sort_by(|a, b| match (b.saved_at, a.saved_at) {
            (Some(b), Some(a)) => b.cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => a.rel_path.cmp(&b.rel_path),
        });
        Ok(out)
    }

    /// Carry out the user's decision about one conflict sibling.
    ///
    /// **This is the only code path in Wobu that deletes a conflict sibling**,
    /// and it is reachable only from a button. Nothing on a timer, on a scan or
    /// on a rebuild may ever grow a second one — a sibling is by construction
    /// the last surviving copy of a paragraph somebody typed.
    ///
    /// `expected_hash` is the hash of the node file as the card rendered it. A
    /// mismatch means a third writer moved the file while the user was reading
    /// the diff, so the question they answered is not the question on disk:
    /// [`Resolved::Stale`] comes back and *neither* file is touched. Without
    /// that check, "keep theirs" would quietly throw away the parked version in
    /// favour of text the user never saw, which is the same silent loss the
    /// whole conflict machinery exists to prevent.
    ///
    /// [`Keep::Current`] additionally writes the refusal down, so that the next
    /// sync round does not park the same bytes again and re-ask a question that
    /// has been answered (#89). [`Keep::Parked`] needs no such bookkeeping and
    /// deliberately does none: taking their version makes their bytes the local
    /// file, so the next compare sees `local == remote` and reads `Converged`,
    /// which is not a conflict and never consults a refusal. A refusal can only
    /// ever suppress a `Conflict`, and a `Conflict` requires the two sides to
    /// differ — so any row left over from an earlier decision about this node
    /// simply stops matching, with nothing to clear. See `record_refusal` below
    /// for how a sibling's alias is turned into the peer id a refusal is keyed
    /// on, and for the two cases where it honestly cannot be.
    pub fn resolve_conflict(
        &mut self,
        rel_path: &str,
        keep: Keep,
        expected_hash: &str,
    ) -> Result<Resolved> {
        self.ensure_writable()?;

        let sibling = paths::from_rel_string(&self.root, rel_path);
        // Checked before anything else, and checked against the *filename*
        // rather than against the list above: this function removes a file, and
        // the argument comes over a bridge. A caller that got the path wrong
        // must fail here rather than delete a node.
        let name = sibling
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(conflict::parse)
            .ok_or_else(|| Error::NotAConflict(sibling.clone()))?;
        if !sibling.is_file() {
            return Err(Error::NotAConflict(sibling));
        }

        let target = sibling.with_file_name(name.target_file_name());
        let target_rel = target
            .strip_prefix(&self.root)
            .map(paths::to_rel_string)
            .unwrap_or_else(|_| paths::to_rel_string(&target));

        // `None` covers the node file having been deleted since the card was
        // drawn, which is a change like any other and gets the same answer.
        let current = atomic::read_stamped(&target)?;
        let current_hash = current.as_ref().map(|(_, s)| s.hash.as_str()).unwrap_or("");
        if current_hash != expected_hash {
            return Ok(Resolved::Stale);
        }

        match keep {
            // Nothing is written: the winner is already the file on disk. The
            // guard above is what makes this safe — it is the reason a bare
            // delete here cannot discard a version the user did not reject.
            Keep::Current => {
                // Read before the delete, obviously, and kept even if the
                // recording below finds nobody to attribute it to: the hash of
                // the bytes being refused is the whole content of the decision.
                let refused = atomic::read_stamped(&sibling)?.map(|(_, stamp)| stamp.hash);
                self.remove_sibling(&sibling)?;
                // After the removal, not before. A refusal recorded for a card
                // that is still on screen would be a row describing a decision
                // that did not land — and the order that matters is the same one
                // `Keep::Parked` uses below: the index only ever learns about a
                // change the folder already made.
                if let Some(refused) = refused {
                    self.record_refusal(&name, &target_rel, &refused)?;
                }
                Ok(Resolved::Done)
            }
            Keep::Parked => {
                let Some((parked, _)) = atomic::read_stamped(&sibling)? else {
                    return Err(Error::NotAConflict(sibling));
                };
                let expected = current.map(|(_, stamp)| stamp);

                // Back through `guarded_write` rather than a plain write. The
                // hash check above closed the window the user spent reading;
                // this closes the microseconds between that check and the
                // rename, which on a share is a real window and the one place a
                // resolution could still clobber.
                match atomic::guarded_write(
                    &self.root,
                    &target,
                    &parked,
                    expected.as_ref(),
                    &self.peer,
                )? {
                    WriteOutcome::Written(stamp) => {
                        match markdown::from_markdown(&parked, &target) {
                            Ok(node) => {
                                self.index.upsert_node(&node, &target_rel, &stamp)?;
                                self.index.clear_corrupt(&target_rel)?;
                            }
                            // The user chose a version that does not parse —
                            // a sibling a sync client truncated, most likely.
                            // It is still their choice, so it lands; the
                            // navigator says so rather than the app refusing.
                            Err(e) => {
                                let why = self.relative_message(&e.to_string());
                                self.index.mark_corrupt(&target_rel, &why)?;
                            }
                        }
                        // Only after the winner is safely on disk. The other
                        // order loses everything if the write then fails.
                        self.remove_sibling(&sibling)?;
                        Ok(Resolved::Done)
                    }
                    WriteOutcome::Conflict { conflict_path, .. } => {
                        let rel = conflict_path
                            .strip_prefix(&self.root)
                            .map(paths::to_rel_string)
                            .unwrap_or_else(|_| paths::to_rel_string(&conflict_path));
                        Ok(Resolved::Conflict { conflict_path: rel })
                    }
                }
            }
        }
    }

    /// Write down that a person refused these exact bytes, so the next sync does
    /// not park them again.
    ///
    /// A conflict does not move the base — deliberately, and #80's argument for
    /// that stands — so the same disagreement is rediscovered every round and the
    /// card the user has just dismissed comes straight back (#89). This is the
    /// other half: `sync_rejected` holds the refusal, and
    /// [`crate::apply::already_refused`] reads it.
    ///
    /// ## Finding the peer, when all we have is an alias
    ///
    /// The refusal is keyed on a full 64-hex `EndpointId`, because an alias is
    /// twenty-eight bits of a public key and this codebase's standing rule is
    /// that an alias is displayed and never decides anything. But a conflict
    /// sibling only *carries* an alias — that is the whole of what its filename
    /// can carry, since the name is a contract with other machines that must read
    /// the same on all of them — and there is no table mapping one back to an id,
    /// by design (#76).
    ///
    /// So the alias is inverted the only honest way: re-derive it, with the same
    /// pure function, over every peer id this project already knows, and take the
    /// answer only if **exactly one** matches. The decision is then keyed on the
    /// full id, so the alias has narrowed a candidate set rather than authorised
    /// anything.
    ///
    /// Both failure modes are deliberately the same failure, and it is the
    /// harmless one — nothing is recorded, and the card comes back exactly as it
    /// does today:
    ///
    /// - **No candidate matches.** The sibling came from a peer this project has
    ///   never settled or refused anything with, or from a build with a different
    ///   derivation, or the name was hand-made. Guessing here would attach a
    ///   person's decision to a machine that had nothing to do with it.
    /// - **Several candidates match.** Two ids sharing an alias is a thirty-two
    ///   bit coincidence, or somebody grinding keypairs. Recording for all of
    ///   them would let a ground key suppress one specific version of one node
    ///   from an honest peer, which is a lost edit — small, targeted, and silent.
    ///   Recording for none costs one redundant card.
    ///
    /// The node also has to be one the index knows, because a refusal is keyed by
    /// node id and a sibling beside an unindexed file has no id to key on. That
    /// is the same shrug for the same reason.
    ///
    /// A caller that *does* hold a real peer id — `wobu-sync`, or a shell command
    /// wired to a card that carries one — should use [`reject_from_peer`] and
    /// skip all of the above.
    ///
    /// [`reject_from_peer`]: Project::reject_from_peer
    fn record_refusal(
        &self,
        name: &conflict::SiblingName,
        target_rel: &str,
        refused_hash: &str,
    ) -> Result<()> {
        let Some(alias) = name.peer.as_deref() else { return Ok(()) };
        let Some((node_id, _)) = self.index.node_at_rel_path(target_rel)? else { return Ok(()) };

        let mut matches = self
            .index
            .known_peers()?
            .into_iter()
            .filter(|peer_id| apply::alias_of(peer_id) == alias);
        let (Some(peer_id), None) = (matches.next(), matches.next()) else { return Ok(()) };

        self.index.record_rejection(&peer_id, node_id, refused_hash)
    }

    /// Refuse one version of one node from a peer named by its id.
    ///
    /// The seam without the guesswork, for a caller that knows which machine sent
    /// the bytes. [`resolve_conflict`] cannot know — it is handed a filename, and
    /// a filename holds an alias — so it re-derives; this takes the id straight.
    ///
    /// `hash` is the full BLAKE3 hex of the version being refused, as
    /// [`crate::apply::Outgoing::hash`] and `Stamp` spell it. A truncated
    /// `source_version` here would suppress a later rename from the same peer,
    /// because a version is blind to names.
    ///
    /// Nothing is written to the folder and nothing is deleted. A refusal only
    /// ever turns a future conflict into "do nothing" — it can never authorise a
    /// write — so this is safe to call for a decision that turns out to be
    /// redundant, and idempotent for one made twice.
    ///
    /// [`resolve_conflict`]: Project::resolve_conflict
    pub fn reject_from_peer(&self, peer_id: &str, node_id: Id, hash: &str) -> Result<()> {
        self.index.record_rejection(peer_id, node_id, hash)
    }

    /// The single `remove_file` call that may name a conflict sibling.
    ///
    /// Trivial on purpose: keeping it as one named function means a grep for
    /// callers is a complete audit of what can delete one, which is the only
    /// way to keep that promise true as the crate grows.
    fn remove_sibling(&self, sibling: &Path) -> Result<()> {
        match std::fs::remove_file(sibling) {
            Ok(()) => Ok(()),
            // Already gone — a collaborator resolved the same card. The user
            // asked for it to not be there and it is not there.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(sibling, e)),
        }
    }

    // ── reconciliation ───────────────────────────────────────────────────

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
        generations::invalidate_spend_aggregate(&self.root);
        on_progress(ScanProgress { done: total, total });
        Ok(())
    }

    /// Throw the index away and build it again from the Markdown.
    ///
    /// Offered to the user as a support action, and safe to offer because the
    /// index holds no canonical data — every fact is in the folder. It is the
    /// answer to "the navigator is showing something that isn't there", which
    /// is otherwise unfixable from inside the app.
    ///
    /// Refuses while the folder is unreachable. Rebuilding from a share that is
    /// not mounted would faithfully record that the world is empty, which is
    /// the one way this operation can actually lose something a user cares
    /// about — the last readable copy of their world.
    pub fn rebuild_index(&mut self) -> Result<()> {
        if !self.is_present() {
            return Err(Error::Disconnected);
        }
        self.rescan()?;
        self.index.vacuum()
    }

    /// Where the index file for this project lives.
    pub fn index_path(&self) -> PathBuf {
        self.index_path.clone()
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
            generations::invalidate_spend_aggregate(&self.root);
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
    fn markdown_files(&self) -> Vec<(String, PathBuf)> {
        markdown_files_at(&self.root)
    }

    /// Every node Markdown file, as `(relative path, absolute path)`.
    fn node_files(&self) -> Vec<(String, PathBuf)> {
        self.markdown_files().into_iter().filter(|(_, path)| !is_conflict_path(path)).collect()
    }

    /// The other half: files `guarded_write` parked, which are never nodes.
    fn conflict_files(&self) -> Vec<(String, PathBuf)> {
        self.markdown_files().into_iter().filter(|(_, path)| is_conflict_path(path)).collect()
    }
}

fn markdown_files_at(root: &Path) -> Vec<(String, PathBuf)> {
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

fn remove_asset_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path, error)),
    }
}

/// Conflict siblings are for a human to resolve. Indexing one would put a ghost
/// duplicate of the node in the navigator, and — far worse — make it a save
/// target, so that resolving the conflict could start a new one.
fn is_conflict_path(path: &Path) -> bool {
    path.file_name().is_some_and(|n| conflict::is_sibling(&n.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_core::new_id;

    fn new_project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(dir.path(), "Ashfall").unwrap();
        (dir, project)
    }

    #[test]
    fn create_lays_out_a_self_contained_folder() {
        let (dir, project) = new_project();
        let root = dir.path().join("ashfall.wobu");
        assert_eq!(project.root(), root);
        for expected in [
            "project.json",
            "nodes",
            "assets/originals",
            "assets/thumbs",
            "generations",
            ".wobu/tmp",
            ".wobu/sessions",
        ] {
            assert!(root.join(expected).exists(), "missing {expected}");
        }
    }

    #[test]
    fn create_seeds_the_two_singletons() {
        let (_dir, project) = new_project();
        let nodes = project.list_nodes().unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::StyleGuide));
        assert!(kinds.contains(&NodeKind::WorldBible));
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn singletons_cannot_be_duplicated_or_deleted() {
        let (_dir, mut project) = new_project();
        assert!(project.create_node(NodeKind::StyleGuide, "Another Style", None).is_err());

        let style = project
            .list_nodes()
            .unwrap()
            .into_iter()
            .find(|n| n.kind == NodeKind::StyleGuide)
            .unwrap();
        assert!(project.delete_node(style.id).is_err());
    }

    #[test]
    fn creating_a_project_twice_in_the_same_place_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        Project::create(dir.path(), "Ashfall").unwrap();
        assert!(matches!(Project::create(dir.path(), "Ashfall"), Err(Error::AlreadyExists(_))));
    }

    #[test]
    fn a_node_round_trips_through_disk() {
        let (_dir, mut project) = new_project();
        let created = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();

        let mut edited = project.get_node(created.id).unwrap();
        assert_eq!(edited, created);

        edited.notes_raw = "scarred, ex-guild".into();
        let SaveOutcome::Saved(_) = project.save_node(edited).unwrap() else {
            panic!("expected a clean save")
        };

        let reloaded = project.get_node(created.id).unwrap();
        assert_eq!(reloaded.notes_raw, "scarred, ex-guild");
    }

    #[test]
    fn generations_are_append_only_and_indexed_per_node() {
        let (_dir, mut project) = new_project();
        let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
        let make = |id: &str, prompt: &str| Generation {
            id: Id::from_string(id).unwrap(),
            node_id: node.id,
            created_at: "2026-07-31T14:22:11Z".parse().unwrap(),
            preset: "character_sheet".into(),
            view_type: None,
            user_prompt: "at dusk".into(),
            compiled_prompt: prompt.into(),
            negative_prompt: "text, watermark".into(),
            backend: "gemini".into(),
            model: "gemini-2.5-flash-image".into(),
            seed: 42,
            params: Default::default(),
            output_asset_ids: vec![],
            influence_snapshot: wobu_core::InfluenceSnapshot { layers: vec![] },
        };
        let first = make("01ARZ3NDEKTSV4RRFFQ69G5FAV", "first compiled prompt");
        let second = make("01ARZ3NDEKTSV4RRFFQ69G5FAW", "second compiled prompt");

        project.record_generation(first.clone()).unwrap();
        project.record_generation(second.clone()).unwrap();
        let history = project.list_generations(node.id).unwrap();
        assert_eq!(history.len(), 2, "one node may have concurrent generation attempts");
        assert!(history.contains(&first));
        assert!(history.contains(&second));

        let changed = make("01ARZ3NDEKTSV4RRFFQ69G5FAV", "must not replace the first");
        assert!(matches!(project.record_generation(changed), Err(Error::AlreadyExists(_))));
        assert_eq!(project.get_generation(first.id).unwrap(), Some(first));

        project.index.clear().unwrap();
        assert!(project.list_generations(node.id).unwrap().is_empty());
        project.rescan().unwrap();
        assert_eq!(project.list_generations(node.id).unwrap().len(), 2);
    }

    #[test]
    fn replay_receipt_may_outlive_its_original_node() {
        let (_dir, mut project) = new_project();
        let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
        let original = Generation {
            id: new_id(),
            node_id: node.id,
            created_at: "2026-07-31T14:22:11Z".parse().unwrap(),
            preset: "portrait".into(),
            view_type: None,
            user_prompt: String::new(),
            compiled_prompt: "Kael".into(),
            negative_prompt: String::new(),
            backend: "comfyui".into(),
            model: "local".into(),
            seed: 42,
            params: Default::default(),
            output_asset_ids: vec![],
            influence_snapshot: wobu_core::InfluenceSnapshot { layers: vec![] },
        };
        project.record_generation(original.clone()).unwrap();
        project.delete_node(node.id).unwrap();
        let mut replay = original.clone();
        replay.id = new_id();
        replay.params.insert("replayOf".into(), serde_json::json!(original.id));
        project.record_replay_generation(replay.clone()).unwrap();
        assert_eq!(project.get_generation(replay.id).unwrap(), Some(replay));
    }

    #[test]
    fn nodes_land_at_the_documented_path() {
        let (dir, mut project) = new_project();
        project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
        assert!(dir.path().join("ashfall.wobu/nodes/character/kael-vantris.md").is_file());
    }

    #[test]
    fn duplicate_names_get_distinct_filenames() {
        let (_dir, mut project) = new_project();
        let a = project.create_node(NodeKind::Character, "Kael", None).unwrap();
        let b = project.create_node(NodeKind::Character, "Kael", None).unwrap();
        assert_ne!(a.slug, b.slug);
        assert_eq!(a.name, b.name);
    }

    #[test]
    fn renaming_a_node_does_not_move_its_file() {
        // Moving the file out from under a collaborator is worse than a stale slug.
        let (_dir, mut project) = new_project();
        let mut node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
        node.name = "Kael the Ashbound".into();
        project.save_node(node.clone()).unwrap();

        let reloaded = project.get_node(node.id).unwrap();
        assert_eq!(reloaded.slug, "kael-vantris");
        assert_eq!(reloaded.name, "Kael the Ashbound");
    }

    #[test]
    fn deleting_a_parent_promotes_its_children() {
        let (_dir, mut project) = new_project();
        let region = project.create_node(NodeKind::Setting, "Ember Coast", None).unwrap();
        let city = project.create_node(NodeKind::Setting, "Cinder Bay", Some(region.id)).unwrap();

        project.delete_node(region.id).unwrap();

        let survivor = project.get_node(city.id).unwrap();
        assert_eq!(survivor.parent_id, None, "the city must not vanish with the region");
        assert!(project.get_node(region.id).is_err());
    }

    #[test]
    fn explicit_node_links_add_update_remove_and_answer_backlinks() {
        let (_dir, mut project) = new_project();
        let guild = project.create_node(NodeKind::Culture, "Ember Guild", None).unwrap();
        let kael = project.create_node(NodeKind::Character, "Kael", None).unwrap();

        let SaveOutcome::Saved(saved) = project
            .add_node_link(kael.id, guild.id, wobu_core::LinkRole::MemberOf, Some(2.0), Some(false))
            .unwrap()
        else {
            panic!("expected a clean add")
        };
        assert_eq!(saved.links.len(), 1);
        assert_eq!(saved.links[0].weight, 1.0, "command weights are clamped");
        assert!(!saved.links[0].enabled);
        let incoming = project.node_backlinks(guild.id).unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].from_id, kael.id);

        let SaveOutcome::Saved(saved) = project
            .update_node_link(
                kael.id,
                guild.id,
                wobu_core::LinkRole::MemberOf,
                Some(0.4),
                Some(true),
            )
            .unwrap()
        else {
            panic!("expected a clean update")
        };
        assert_eq!(saved.links[0].weight, 0.4);
        assert!(saved.links[0].enabled);

        let SaveOutcome::Saved(saved) =
            project.remove_node_link(kael.id, guild.id, wobu_core::LinkRole::MemberOf).unwrap()
        else {
            panic!("expected a clean removal")
        };
        assert!(saved.links.is_empty());
        assert!(project.node_backlinks(guild.id).unwrap().is_empty());
    }

    #[test]
    fn node_link_add_obeys_the_source_kinds_registry_roles() {
        let (_dir, mut project) = new_project();
        let style = project
            .list_nodes()
            .unwrap()
            .into_iter()
            .find(|node| node.kind == NodeKind::StyleGuide)
            .unwrap();
        let kael = project.create_node(NodeKind::Character, "Kael", None).unwrap();

        let result =
            project.add_node_link(kael.id, style.id, wobu_core::LinkRole::StyledBy, None, None);
        assert!(matches!(result, Err(Error::InvalidNodeLinkRole { .. })));
        assert!(project.get_node(kael.id).unwrap().links.is_empty());
    }

    #[test]
    fn deleting_a_node_strips_the_links_that_pointed_at_it() {
        let (_dir, mut project) = new_project();
        let guild = project.create_node(NodeKind::Culture, "Ember Guild", None).unwrap();
        let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
        kael.links.push(wobu_core::Link::new(guild.id, wobu_core::LinkRole::MemberOf));
        let SaveOutcome::Saved(kael) = project.save_node(kael).unwrap() else {
            panic!("expected a clean save")
        };

        project.delete_node(guild.id).unwrap();

        // Not just in the index — in the Markdown, which is the source of truth.
        let reread = project.get_node(kael.id).unwrap();
        assert!(reread.links.is_empty(), "dangling link survived: {:?}", reread.links);
        assert!(project.index().backlinks(guild.id).unwrap().is_empty());
    }

    #[test]
    fn deleting_two_linked_nodes_works_in_either_order() {
        let (_dir, mut project) = new_project();
        let guild = project.create_node(NodeKind::Culture, "Ember Guild", None).unwrap();
        let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
        kael.links.push(wobu_core::Link::new(guild.id, wobu_core::LinkRole::MemberOf));
        let SaveOutcome::Saved(kael) = project.save_node(kael).unwrap() else {
            panic!("expected a clean save")
        };

        project.delete_node(kael.id).unwrap();
        project.delete_node(guild.id).unwrap();
        assert!(project.list_nodes().unwrap().iter().all(|n| n.id != guild.id));
    }

    #[test]
    fn a_move_that_would_make_a_cycle_is_refused() {
        let (_dir, mut project) = new_project();
        let region = project.create_node(NodeKind::Setting, "Ember Coast", None).unwrap();
        let city = project.create_node(NodeKind::Setting, "Cinder Bay", Some(region.id)).unwrap();

        assert!(project.move_node(region.id, Some(city.id)).is_err());
        assert_eq!(project.get_node(region.id).unwrap().parent_id, None);
    }

    #[test]
    fn reopening_reads_the_world_back_off_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = {
            let mut project = Project::create(dir.path(), "Ashfall").unwrap();
            project.create_node(NodeKind::Species, "Vashk", None).unwrap();
            project.root().to_path_buf()
        };

        let reopened = Project::open(&root).unwrap();
        let names: Vec<_> = reopened.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
        assert!(names.contains(&"Vashk".to_string()));
    }

    #[test]
    fn deleting_the_index_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (root, id) = {
            let mut project = Project::create(dir.path(), "Ashfall").unwrap();
            project.create_node(NodeKind::Species, "Vashk", None).unwrap();
            (project.root().to_path_buf(), project.id())
        };

        std::fs::remove_file(paths::index_path(&id)).ok();
        let reopened = Project::open(&root).unwrap();
        assert_eq!(reopened.list_nodes().unwrap().len(), 3, "2 singletons + Vashk");
    }

    #[test]
    fn reconcile_picks_up_an_external_edit() {
        // The Obsidian / git-pull / collaborator case.
        let (_dir, mut project) = new_project();
        let node = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");

        let text =
            std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Vashk-Prime");
        // Push mtime forward; a same-second write can otherwise look unchanged.
        std::fs::write(&path, text).unwrap();
        filetime_bump(&path);

        assert!(project.reconcile().unwrap());
        let names: Vec<_> = project.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
        assert!(names.contains(&"Vashk-Prime".to_string()), "{names:?}");
        assert_eq!(project.get_node(node.id).unwrap().name, "Vashk-Prime");
    }

    #[test]
    fn local_reconcile_only_reads_the_reported_node_path() {
        let (_dir, mut project) = new_project();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let sunborn = project.create_node(NodeKind::Species, "Sunborn", None).unwrap();
        let vashk_path = project.root().join("nodes/species/vashk.md");
        let sunborn_path = project.root().join("nodes/species/sunborn.md");

        for (path, before, after) in [
            (&vashk_path, "name: Vashk", "name: Vashk-Prime"),
            (&sunborn_path, "name: Sunborn", "name: Sunborn-Prime"),
        ] {
            let text = std::fs::read_to_string(path).unwrap().replace(before, after);
            std::fs::write(path, text).unwrap();
            filetime_bump(path);
        }

        assert!(project.reconcile_paths(std::slice::from_ref(&vashk_path)).unwrap());
        let indexed = project.list_nodes().unwrap();
        assert_eq!(indexed.iter().find(|node| node.id == vashk.id).unwrap().name, "Vashk-Prime");
        assert_eq!(
            indexed.iter().find(|node| node.id == sunborn.id).unwrap().name,
            "Sunborn",
            "an unrelated external edit must wait for its own event"
        );

        assert!(project.reconcile().unwrap());
        let indexed = project.list_nodes().unwrap();
        assert_eq!(
            indexed.iter().find(|node| node.id == sunborn.id).unwrap().name,
            "Sunborn-Prime"
        );
    }

    #[test]
    fn a_full_observation_is_rejected_if_a_file_moves_before_apply() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");

        let first = std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: First");
        std::fs::write(&path, first).unwrap();
        filetime_bump(&path);
        let observation = project.reconcile_plan().unwrap().observe().unwrap();

        let second = std::fs::read_to_string(&path).unwrap().replace("name: First", "name: Second");
        std::fs::write(&path, second).unwrap();
        filetime_bump(&path);

        assert!(!observation.revalidate().unwrap());
        assert!(project.reconcile().unwrap());
        assert!(project.list_nodes().unwrap().iter().any(|node| node.name == "Second"));
    }

    #[test]
    fn revalidation_notices_a_file_that_was_unchanged_during_observation() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        let observation = project.reconcile_plan().unwrap().observe().unwrap();

        let edited =
            std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Vashk-Prime");
        std::fs::write(&path, edited).unwrap();
        filetime_bump(&path);

        assert!(!observation.revalidate().unwrap());
        assert!(project.reconcile().unwrap());
    }

    #[test]
    fn markdown_walk_does_not_descend_below_the_node_depth_limit() {
        let (_dir, project) = new_project();
        let deep = project.root().join("nodes/species/deep/deeper");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("hidden.md"), "---\nname: Hidden\n---\n").unwrap();

        assert!(
            markdown_files_at(project.root()).iter().all(|(rel, _)| !rel.ends_with("hidden.md"))
        );
    }

    #[test]
    fn reconcile_survives_two_files_swapping_names() {
        // Renaming files around in Obsidian is normal, and a swap is the case
        // where a node arrives at a path the index still believes belongs to
        // someone else. Getting this wrong makes the project fail to open.
        let (_dir, mut project) = new_project();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let sunborn = project.create_node(NodeKind::Species, "Sunborn", None).unwrap();

        let dir = project.root().join("nodes/species");
        let (a, b, tmp) = (dir.join("vashk.md"), dir.join("sunborn.md"), dir.join("swap.tmp"));
        std::fs::rename(&a, &tmp).unwrap();
        std::fs::rename(&b, &a).unwrap();
        std::fs::rename(&tmp, &b).unwrap();
        filetime_bump(&a);
        filetime_bump(&b);

        project.reconcile().unwrap();

        // The slug follows the filename, so they have traded places.
        assert_eq!(project.get_node(vashk.id).unwrap().slug, "sunborn");
        assert_eq!(project.get_node(sunborn.id).unwrap().slug, "vashk");
    }

    #[test]
    fn reconcile_notices_an_externally_deleted_file() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        std::fs::remove_file(project.root().join("nodes/species/vashk.md")).unwrap();

        assert!(project.reconcile().unwrap());
        assert_eq!(project.list_nodes().unwrap().len(), 2, "only the singletons remain");
    }

    /// A truncated YAML frontmatter block — the shape Dropbox and OneDrive
    /// actually produce when they copy a half-written file.
    fn truncate_frontmatter(path: &Path) {
        let text = std::fs::read_to_string(path).unwrap();
        let cut = text.len() / 3;
        std::fs::write(path, &text[..cut]).unwrap();
        filetime_bump(path);
    }

    #[test]
    fn a_mangled_file_is_recorded_rather_than_dropped() {
        let (_dir, mut project) = new_project();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        truncate_frontmatter(&path);

        assert!(project.reconcile().unwrap());

        let corrupt = project.corrupt_files().unwrap();
        assert_eq!(corrupt.len(), 1, "{corrupt:?}");
        assert_eq!(corrupt[0].rel_path, "nodes/species/vashk.md");
        assert_eq!(corrupt[0].node_id, Some(vashk.id), "the broken file is tied to its entity");
        assert!(!corrupt[0].error.is_empty(), "the parse error is what tells the user what to fix");

        // The parser names the file by absolute path. That is right for a log
        // and wrong for a string the UI shows and a user pastes into a bug
        // report — the leading half of it is their home directory.
        let root = project.root().to_string_lossy().into_owned();
        assert!(!corrupt[0].error.contains(&root), "leaked an absolute path: {}", corrupt[0].error);
        assert!(
            corrupt[0].error.contains("nodes/species/vashk.md"),
            "the file should still be named, relatively: {}",
            corrupt[0].error,
        );
    }

    #[test]
    fn a_mangled_file_keeps_its_node_row_and_its_bytes() {
        let (_dir, mut project) = new_project();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        truncate_frontmatter(&path);
        let on_disk = std::fs::read(&path).unwrap();

        project.reconcile().unwrap();

        // The row survives — this is the whole point. A live node beside a
        // broken file is how the user finds their data again; dropping it
        // makes the entity silently cease to exist.
        let listed = project.list_nodes().unwrap();
        assert!(listed.iter().any(|n| n.id == vashk.id), "the node vanished from the navigator");
        assert_eq!(std::fs::read(&path).unwrap(), on_disk, "the file was modified");
    }

    #[test]
    fn a_mangled_file_is_never_written_over() {
        let (_dir, mut project) = new_project();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        truncate_frontmatter(&path);
        let on_disk = std::fs::read(&path).unwrap();
        project.reconcile().unwrap();

        // Saving the last-known-good node over the mangled file would destroy
        // whatever the sync client left behind — including, possibly, the only
        // copy of an edit made on another machine.
        let outcome = project.save_node(vashk).unwrap();
        assert!(matches!(outcome, SaveOutcome::Conflict { .. }), "{outcome:?}");
        assert_eq!(std::fs::read(&path).unwrap(), on_disk, "the mangled file was overwritten");
    }

    #[test]
    fn a_repaired_file_stops_being_corrupt() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        let good = std::fs::read_to_string(&path).unwrap();

        truncate_frontmatter(&path);
        project.reconcile().unwrap();
        assert_eq!(project.corrupt_files().unwrap().len(), 1);

        // The user restored it from a backup, or the sync client finished.
        std::fs::write(&path, &good).unwrap();
        filetime_bump(&path);
        project.reconcile().unwrap();

        assert!(project.corrupt_files().unwrap().is_empty(), "the broken state stuck around");
        assert_eq!(project.list_nodes().unwrap().len(), 3);
    }

    #[test]
    fn deleting_a_mangled_file_clears_it() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        truncate_frontmatter(&path);
        project.reconcile().unwrap();
        assert_eq!(project.corrupt_files().unwrap().len(), 1);

        // Giving up on it is a legitimate resolution, and the banner has to go.
        std::fs::remove_file(&path).unwrap();
        project.reconcile().unwrap();
        assert!(project.corrupt_files().unwrap().is_empty());
    }

    /// The counterpart to the test above, and the distinction the whole
    /// unmount story rests on: a deleted *file* is a real deletion, a missing
    /// *folder* is not evidence of anything.
    #[test]
    fn reconcile_refuses_to_read_a_vanished_share_as_mass_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let before = project.list_nodes().unwrap().len();

        // What an unmount leaves behind: the mountpoint is still a directory,
        // so `root.is_dir()` is true and walking it yields nothing.
        std::fs::remove_dir_all(project.root()).unwrap();
        std::fs::create_dir_all(project.root()).unwrap();
        assert!(project.root().is_dir(), "the mountpoint should still look like a directory");

        assert!(matches!(project.reconcile(), Err(Error::Disconnected)));
        assert_eq!(
            project.list_nodes().unwrap().len(),
            before,
            "the index is the only readable copy of the world while the share is away",
        );
    }

    /// The index holds summaries, not bodies, so a node that was never opened
    /// before the share went cannot be read. What it must not do is claim the
    /// node does not exist.
    #[test]
    fn an_unreadable_node_blames_the_share_rather_than_the_node() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();

        std::fs::remove_dir_all(project.root()).unwrap();
        std::fs::create_dir_all(project.root()).unwrap();

        assert!(matches!(project.get_node(vashk.id), Err(Error::Disconnected)));
        // Still listed, because that comes from the index.
        assert!(project.list_nodes().unwrap().iter().any(|n| n.id == vashk.id));
    }

    #[test]
    fn a_genuinely_missing_node_still_says_so() {
        let (_dir, mut project) = new_project();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        std::fs::remove_file(project.root().join("nodes/species/vashk.md")).unwrap();

        // The folder is fine, so the file being gone is real information.
        assert!(matches!(project.get_node(vashk.id), Err(Error::NoSuchNode(_))));
    }

    #[test]
    fn writes_are_refused_while_the_share_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();

        std::fs::remove_dir_all(project.root()).unwrap();
        std::fs::create_dir_all(project.root()).unwrap();

        // Left to itself this write would *succeed*, landing on the local disk
        // under the empty mountpoint — invisible to everyone else and shadowed
        // the moment the share returns.
        assert!(matches!(project.save_node(vashk), Err(Error::Disconnected)));
        assert!(matches!(
            project.create_node(NodeKind::Species, "Sunborn", None),
            Err(Error::Disconnected)
        ));
    }

    #[test]
    fn a_share_that_comes_back_reconciles_normally() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();

        let stashed = dir.path().join("stash");
        std::fs::rename(project.root(), &stashed).unwrap();
        assert!(matches!(project.reconcile(), Err(Error::Disconnected)));

        std::fs::rename(&stashed, project.root()).unwrap();
        assert!(project.is_present());
        project.reconcile().unwrap();
        assert_eq!(project.list_nodes().unwrap().len(), 3, "singletons plus Vashk");
    }

    #[test]
    fn conflict_siblings_are_never_indexed_as_nodes() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let original = project.root().join("nodes/species/vashk.md");
        let sibling = project.root().join("nodes/species/vashk.conflict-nadia-20260731T142211Z.md");
        std::fs::copy(&original, &sibling).unwrap();

        project.rescan().unwrap();
        let vashk: Vec<_> =
            project.list_nodes().unwrap().into_iter().filter(|n| n.name == "Vashk").collect();
        assert_eq!(vashk.len(), 1, "the conflict copy must not appear in the navigator");
    }

    #[test]
    fn a_corrupt_file_is_skipped_rather_than_overwritten() {
        let (_dir, mut project) = new_project();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");
        std::fs::write(&path, "this is not a node file").unwrap();

        project.rescan().unwrap();
        assert_eq!(project.list_nodes().unwrap().len(), 2, "corrupt file drops out of the index");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "this is not a node file",
            "but is left exactly as it was on disk"
        );
    }

    #[test]
    fn a_concurrent_edit_produces_a_conflict_not_a_clobber() {
        let (_dir, mut project) = new_project();
        let node = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        let path = project.root().join("nodes/species/vashk.md");

        // Nadia saves while we hold an older copy.
        let theirs =
            std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Nadia's Vashk");
        std::fs::write(&path, theirs).unwrap();
        filetime_bump(&path);

        let mut mine = node.clone();
        mine.notes_raw = "my edit".into();
        let outcome = project.save_node(mine).unwrap();

        let SaveOutcome::Conflict { conflict_path } = outcome else {
            panic!("expected a conflict, got {outcome:?}")
        };
        assert!(conflict_path.contains(".conflict-"), "{conflict_path}");
        assert!(std::fs::read_to_string(&path).unwrap().contains("Nadia's Vashk"));
        assert!(project.root().join(&conflict_path).is_file());
    }

    #[test]
    fn a_read_only_project_refuses_writes_rather_than_failing_late() {
        let (_dir, mut project) = new_project();
        project.read_only = true;
        assert!(matches!(
            project.create_node(NodeKind::Species, "Vashk", None),
            Err(Error::ReadOnly)
        ));
    }

    #[test]
    fn a_read_only_project_refuses_an_import_rather_than_failing_late() {
        // An import is a write like any other, and the chip in the title bar
        // has already told the user this folder cannot take one.
        let (_dir, mut project) = new_project();
        project.read_only = true;
        assert!(matches!(
            project.import_asset(&[0x89, b'P', b'N', b'G'], AssetKind::Reference),
            Err(Error::ReadOnly)
        ));
    }

    #[test]
    fn a_read_only_project_refuses_an_asset_link_rather_than_failing_late() {
        // Attaching a reference is an edit to the node file, so the chip in the
        // title bar has already told the user this cannot work.
        let (_dir, mut project) = new_project();
        let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
        let asset = wobu_core::new_id();
        project.read_only = true;

        assert!(matches!(
            project.link_asset(node.id, asset, AssetRole::Pose, None),
            Err(Error::ReadOnly)
        ));
        assert!(matches!(project.set_cover_asset(node.id, Some(asset)), Err(Error::ReadOnly)));
        // Read-only is reported ahead of the asset not existing: the folder is
        // the reason nothing can happen here, and it is the one the user can do
        // something about.
        assert!(matches!(
            project.unlink_asset(node.id, asset, AssetRole::Pose),
            Err(Error::ReadOnly)
        ));
    }

    #[test]
    fn an_import_is_refused_while_the_share_is_away() {
        // The same trap as a node save: writing into an unmounted share's
        // leftover mountpoint succeeds, lands on local disk, and is shadowed
        // the moment the share comes back — taking the reference with it.
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        std::fs::remove_dir_all(project.root()).unwrap();
        std::fs::create_dir_all(project.root()).unwrap();

        assert!(matches!(
            project.import_asset(&[0x89, b'P', b'N', b'G'], AssetKind::Reference),
            Err(Error::Disconnected)
        ));
    }

    #[test]
    fn opening_a_plain_folder_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Project::open(dir.path()), Err(Error::NotAProject(_))));
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_misread() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(dir.path(), "Ashfall").unwrap();
        let meta_path = project.root().join(PROJECT_FILE);
        let raw = std::fs::read_to_string(&meta_path).unwrap();
        std::fs::write(&meta_path, raw.replace("\"schemaVersion\": 1", "\"schemaVersion\": 99"))
            .unwrap();

        assert!(matches!(
            Project::open(project.root()),
            Err(Error::SchemaTooNew { found: 99, .. })
        ));
    }

    #[test]
    fn nothing_absolute_is_written_into_the_folder() {
        // The same share is /Volumes/art/… on one machine and Z:\art\… on another.
        let (dir, mut project) = new_project();
        project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
        let root_string = dir.path().to_string_lossy().into_owned();

        for (_, path) in project.node_files() {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains(&root_string), "{} leaked an absolute path", path.display());
        }
        let meta = std::fs::read_to_string(project.root().join(PROJECT_FILE)).unwrap();
        assert!(!meta.contains(&root_string));
    }

    #[test]
    fn spend_ceiling_is_shared_and_preserves_unknown_metadata() {
        let (_dir, mut project) = new_project();
        assert_eq!(project.meta().spend_ceiling_usd_micros, Some(DEFAULT_SPEND_CEILING_USD_MICROS));
        let path = project.root().join(PROJECT_FILE);
        let mut meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        meta.as_object_mut()
            .unwrap()
            .insert("futureMetadata".into(), serde_json::json!({ "kept": true }));
        std::fs::write(&path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();

        project.set_spend_ceiling(Some(2_500_000)).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["spendCeilingUsdMicros"], 2_500_000);
        assert_eq!(saved["futureMetadata"]["kept"], true);
        assert_eq!(project.meta().spend_ceiling_usd_micros, Some(2_500_000));

        project.set_spend_ceiling(None).unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
                .unwrap()["spendCeilingUsdMicros"]
                .is_null()
        );
    }

    #[test]
    fn project_without_a_ceiling_gets_the_default_guardrail() {
        let (_dir, project) = new_project();
        let path = project.root().join(PROJECT_FILE);
        let mut meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        meta.as_object_mut().unwrap().remove("spendCeilingUsdMicros");
        std::fs::write(&path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
        let root = project.root().to_path_buf();
        drop(project);
        let reopened = Project::open(&root).unwrap();
        assert_eq!(
            reopened.meta().spend_ceiling_usd_micros,
            Some(DEFAULT_SPEND_CEILING_USD_MICROS)
        );
    }

    #[test]
    fn transfer_reports_a_guarded_write_race_with_pending_ids() {
        let source_dir = tempfile::tempdir().unwrap();
        let mut source = Project::create(source_dir.path(), "Source").unwrap();
        let source_style = source
            .list_nodes()
            .unwrap()
            .into_iter()
            .find(|node| node.kind == NodeKind::StyleGuide)
            .unwrap();
        let mut style = source.get_node(source_style.id).unwrap();
        style.notes_raw = "the incoming house style".to_string();
        assert!(matches!(source.save_node(style).unwrap(), SaveOutcome::Saved(_)));
        let source_root = source.root().to_path_buf();
        drop(source);

        let bundle = crate::transfer::stage(&source_root, source_style.id).unwrap();
        let (_destination_dir, mut destination) = new_project();
        let destination_style = destination
            .list_nodes()
            .unwrap()
            .into_iter()
            .find(|node| node.kind == NodeKind::StyleGuide)
            .unwrap();

        let outcome = destination
            .apply_transfer_with(bundle, |project| {
                let mut changed = project.get_node(destination_style.id).unwrap();
                changed.notes_raw = "a collaborator won the race".to_string();
                assert!(matches!(project.save_node(changed).unwrap(), SaveOutcome::Saved(_)));
            })
            .unwrap();

        assert!(!outcome.completed);
        assert!(outcome.applied_node_ids.is_empty());
        assert_eq!(outcome.pending_node_ids, vec![destination_style.id]);
        assert_eq!(outcome.conflict_paths.len(), 1);
        assert!(outcome.failure.as_deref().unwrap().contains("parked"));
        assert_eq!(
            destination.get_node(destination_style.id).unwrap().notes_raw,
            "a collaborator won the race"
        );
    }

    /// Nudge mtime forward so a write inside the same filesystem timestamp
    /// granularity is still visible to the `(mtime, size)` pre-filter.
    fn filetime_bump(path: &Path) {
        let meta = std::fs::metadata(path).unwrap();
        let later = meta.modified().unwrap() + std::time::Duration::from_secs(2);
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(later).unwrap();
    }
}
