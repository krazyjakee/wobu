//! Images and meshes in the project folder, and what each one is for.
//!
//! An asset's bytes are content-addressed and shared; what makes two links to
//! the same file different is the role recorded on each. Deleting therefore
//! asks how many *canonical* users are left rather than how many links exist —
//! a conflict copy referencing an image must not keep it alive, and must not
//! be what removes it either.

use std::path::{Path, PathBuf};

use wobu_core::asset::AssetRef;
use wobu_core::{Asset, AssetKind, AssetRole, Id, NodeKind};

use super::*;
use crate::assets::{self, ImportedAsset};
use crate::atomic::{self};
use crate::error::{Error, Result};
use crate::markdown;
use crate::paths;
use crate::scan::Cancel;

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

impl Project {
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

        self.remove_asset_blob(&asset)
    }

    /// The destructive half of a deletion, once something has decided it is
    /// allowed. Every caller does its own permission thinking first.
    pub(super) fn remove_asset_blob(&mut self, asset: &Asset) -> Result<()> {
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
    pub(super) fn canonical_asset_users(&self, id: Id) -> Result<usize> {
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
    pub(super) fn require_asset(&self, asset_id: Id) -> Result<()> {
        if self.index.asset(asset_id)?.is_none() {
            return Err(Error::NoSuchAsset(asset_id.to_string()));
        }
        Ok(())
    }
}

fn remove_asset_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path, error)),
    }
}
