//! A project is a self-contained folder.
//!
//! Nothing canonical is stored outside it — no global application database, no
//! absolute paths, no secrets. Copy the folder to a USB stick and it opens
//! somewhere else; delete the local index and nothing is lost. See
//! `docs/02-data-model.md`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::asset::AssetRef;
use wobu_core::{
    Asset, AssetKind, AssetRole, Id, Node, NodeKind, NodeSummary, SCHEMA_VERSION, kind_def,
    kind_registry,
};

use crate::assets::{self, ImportedAsset};
use crate::atomic::{self, WriteOutcome};
use crate::conflict::{self, Conflict, Keep, Resolved};
use crate::error::{Error, Result};
use crate::index::{CorruptFile, Index, Touched};
use crate::markdown;
use crate::paths;
use crate::scan::{Cancel, ScanProgress};

const PROJECT_FILE: &str = "project.json";
const NODES_DIR: &str = "nodes";

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
    Conflict { conflict_path: String },
}

pub struct Project {
    root: PathBuf,
    meta: ProjectMeta,
    index: Index,
    on_network_share: bool,
    read_only: bool,
    user: String,
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
        };
        std::fs::write(root.join(PROJECT_FILE), serde_json::to_string_pretty(&meta)?)
            .map_err(|e| Error::io(root.join(PROJECT_FILE), e))?;

        let index = Index::open_for(&meta.id)?;
        index.clear()?;
        let mut project = Project {
            root,
            meta,
            index,
            on_network_share: false,
            read_only: false,
            user: current_user(),
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
        let root = path.to_path_buf();
        let meta_path = root.join(PROJECT_FILE);
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

        let index = Index::open_for(&meta.id)?;
        let mut project = Project {
            root,
            meta,
            index,
            on_network_share,
            read_only,
            user: current_user(),
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
            SaveOutcome::Conflict { conflict_path } => Err(Error::AlreadyExists(
                paths::from_rel_string(&self.root, &conflict_path),
            )),
        }
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

        match atomic::guarded_write(&self.root, &path, &text, expected, &self.user)? {
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
    pub fn import_asset(&mut self, bytes: &[u8], kind: AssetKind) -> Result<ImportedAsset> {
        self.ensure_writable()?;
        let imported = assets::import(&self.root, bytes, kind)?;
        self.index.upsert_asset(&imported.asset)?;
        Ok(imported)
    }

    /// The same, for a file the user dropped or picked rather than bytes the
    /// webview already holds.
    pub fn import_asset_file(&mut self, path: &Path, kind: AssetKind) -> Result<ImportedAsset> {
        // Read whole rather than streamed: the hash needs every byte anyway,
        // and it has to be hashed before we know where the file goes.
        let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        self.import_asset(&bytes, kind)
    }

    pub fn list_assets(&self) -> Result<Vec<Asset>> {
        self.index.list_assets()
    }

    pub fn get_asset(&self, id: Id) -> Result<Option<Asset>> {
        self.index.asset(id)
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
    pub fn unlink_asset(&mut self, node_id: Id, asset_id: Id, role: AssetRole) -> Result<SaveOutcome> {
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
        let Some(link) = node.asset_links.iter_mut().find(|l| l.asset_id == asset_id && l.role == role)
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

    /// Fold blobs that appeared or vanished into the index.
    ///
    /// A collaborator importing a reference on the far side of a share produces
    /// no event on this machine at all, so a directory listing is the only
    /// signal there will ever be that their file exists.
    fn reconcile_assets(&mut self) -> Result<bool> {
        let known = self.index.asset_paths()?;
        let mut seen = std::collections::HashSet::new();
        let mut changed = false;

        for (rel, path) in assets::list_paths(&self.root) {
            seen.insert(rel.clone());
            // Blobs are immutable — the path *is* the content — so a path we
            // have already described can never need describing again. That is
            // what keeps this cheap enough to run on every watcher tick.
            if known.contains(&rel) {
                continue;
            }
            if let Some(asset) = assets::describe_at(&self.root, &path) {
                self.index.upsert_asset(&asset)?;
                changed = true;
            }
        }

        for rel in known.iter().filter(|rel| !seen.contains(*rel)) {
            self.index.remove_asset_by_rel_path(rel)?;
            changed = true;
        }
        Ok(changed)
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
                mine: name.user.as_deref() == Some(self.user.as_str()),
                rel_path: rel,
                node_rel_path: target_rel,
                node_id: node.as_ref().map(|(id, _)| *id),
                node_name: node.map(|(_, name)| name),
                user: name.user,
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
                self.remove_sibling(&sibling)?;
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
                    &self.user,
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

    /// Read every node file and rebuild the index from scratch.
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

        cancel.check()?;
        self.index.clear()?;
        for asset in &blobs {
            self.index.upsert_asset(asset)?;
        }
        for (node, rel, stamp) in &fresh {
            self.index.upsert_node(node, rel, stamp)?;
            self.index.clear_corrupt(rel)?;
        }
        for (rel, why) in &broken {
            // A file a sync client mangled is left on disk exactly as it is, and
            // recorded so the navigator can say so. Skipping it silently —
            // which is what this used to do — leaves the user with an entity
            // that quietly stopped existing.
            self.index.mark_corrupt(rel, why)?;
        }
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
        paths::index_path(&self.meta.id)
    }

    /// Fold external edits (Obsidian, git pull, a collaborator on the share)
    /// into the index. Returns true if anything changed.
    ///
    /// Only files whose `(mtime, size)` moved are re-read: listing a directory
    /// over SMB is cheap, re-reading hundreds of small files is not.
    pub fn reconcile(&mut self) -> Result<bool> {
        // Nothing below can tell "the folder is empty" from "the folder is not
        // there" — `node_files` walks a missing tree and yields zero entries
        // either way, and the deletion sweep at the bottom then removes every
        // node from the index. That index is the only copy of the world still
        // readable while a share is away, so emptying it turns a recoverable
        // disconnection into what looks to the user like total data loss.
        if !self.is_present() {
            return Err(Error::Disconnected);
        }

        let known = self.index.all_stamps()?;
        // Captured up front so the loop below can tell a file that has just
        // broken from one that was already broken — only the former is a
        // change worth waking the UI for.
        let was_corrupt: std::collections::HashSet<String> =
            self.index.corrupt_paths()?.into_iter().collect();
        let mut seen = std::collections::HashSet::new();
        let mut changed = false;

        for (rel, path) in self.node_files() {
            seen.insert(rel.clone());
            let Some((mtime, size)) = atomic::peek(&path)? else { continue };
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
                Err(e) => {
                    // Note what is *not* here: no `upsert_node`, so the row
                    // this file used to have keeps its last good contents, and
                    // no removal, so the entity stays in the navigator. A live
                    // row beside a broken file is how the user finds their
                    // data again.
                    let why = self.relative_message(&e.to_string());
                    self.index.mark_corrupt(&rel, &why)?;
                    if !was_corrupt.contains(&rel) {
                        changed = true;
                    }
                }
            }
        }

        for rel in known.keys().filter(|rel| !seen.contains(*rel)) {
            self.index.remove_by_rel_path(rel)?;
            changed = true;
        }

        // A corrupt file that has since been deleted or repaired-by-rename is
        // no longer corrupt; without this the banner would never clear.
        for rel in was_corrupt.iter().filter(|rel| !seen.contains(*rel)) {
            self.index.clear_corrupt(rel)?;
            changed = true;
        }

        changed |= self.reconcile_assets()?;
        Ok(changed)
    }

    /// Every Markdown file under `nodes/`, as `(relative path, absolute path)`.
    ///
    /// Includes conflict siblings; the two callers below split them apart. One
    /// walk rather than two so the node list and the conflict list can never
    /// disagree about which files exist, which they would the moment somebody
    /// changed a depth limit in one of them.
    fn markdown_files(&self) -> Vec<(String, PathBuf)> {
        let nodes_root = self.root.join(NODES_DIR);
        walkdir::WalkDir::new(&nodes_root)
            .max_depth(3)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("md")))
            .filter_map(|e| {
                let rel = e.path().strip_prefix(&self.root).ok()?;
                Some((paths::to_rel_string(rel), e.path().to_path_buf()))
            })
            .collect()
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

/// Conflict siblings are for a human to resolve. Indexing one would put a ghost
/// duplicate of the node in the navigator, and — far worse — make it a save
/// target, so that resolving the conflict could start a new one.
fn is_conflict_path(path: &Path) -> bool {
    path.file_name().is_some_and(|n| conflict::is_sibling(&n.to_string_lossy()))
}

/// The name a conflict sibling is stamped with, and the name a collaborator
/// sees in the presence list. Shared so the two cannot disagree about who a
/// person is.
pub(crate) fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for expected in
            ["project.json", "nodes", "assets/originals", "assets/thumbs", ".wobu/tmp", ".wobu/sessions"]
        {
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
        assert!(matches!(
            Project::create(dir.path(), "Ashfall"),
            Err(Error::AlreadyExists(_))
        ));
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

        let text = std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Vashk-Prime");
        // Push mtime forward; a same-second write can otherwise look unchanged.
        std::fs::write(&path, text).unwrap();
        filetime_bump(&path);

        assert!(project.reconcile().unwrap());
        let names: Vec<_> = project.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
        assert!(names.contains(&"Vashk-Prime".to_string()), "{names:?}");
        assert_eq!(project.get_node(node.id).unwrap().name, "Vashk-Prime");
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
        let (a, b, tmp) =
            (dir.join("vashk.md"), dir.join("sunborn.md"), dir.join("swap.tmp"));
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
        let theirs = std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Nadia's Vashk");
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

    /// Nudge mtime forward so a write inside the same filesystem timestamp
    /// granularity is still visible to the `(mtime, size)` pre-filter.
    fn filetime_bump(path: &Path) {
        let meta = std::fs::metadata(path).unwrap();
        let later = meta.modified().unwrap() + std::time::Duration::from_secs(2);
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(later).unwrap();
    }
}
