//! A project is a self-contained folder.
//!
//! Nothing canonical is stored outside it — no global application database, no
//! absolute paths, no secrets. Copy the folder to a USB stick and it opens
//! somewhere else; delete the local index and nothing is lost. See
//! `docs/02-data-model.md`.

// One module per thing a project folder holds. Each contributes its own
// `impl Project`, so the type stays one type and the file it lives in does not
// have to be one file.
pub use self::assets::{AssetUsage, AssetUsageRole};
pub use self::reconcile::{ReconcileObservation, ReconcilePlan};

mod assets;
mod generations;
mod nodes;
mod peers;
mod reconcile;
mod thumbs;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::{Generation, Id, Node, SCHEMA_VERSION, kind_registry};

use crate::atomic::{self};
use crate::error::{Error, Result};
use crate::index::Index;
use crate::paths;
use crate::scan::{Cancel, ScanProgress};

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
/// keys live in machine-local credential storage, because project folders get shared.
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
        let receipts = crate::generations::read_all_strict(path)?;
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

    /// Whether the folder this project was opened from is still reachable.
    ///
    /// Cheap, and checked on the write path rather than cached, because the
    /// whole failure mode is that it changes underneath a running session.
    pub fn is_present(&self) -> bool {
        paths::project_is_present(&self.root)
    }

    pub(super) fn ensure_writable(&self) -> Result<()> {
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

    /// Recheck the write preconditions immediately before a staged commit.
    pub fn verify_writable(&self) -> Result<()> {
        self.ensure_writable()
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
}
