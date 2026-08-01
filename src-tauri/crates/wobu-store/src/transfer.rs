//! Portable style and subtree transfer between project folders.
//!
//! A transfer is deliberately split into three phases. [`preview`] is read-only,
//! [`stage`] reads and validates every referenced blob into memory, and only
//! [`Project::apply_transfer`](crate::Project::apply_transfer) touches the open
//! destination. This keeps a slow or half-synchronised source share outside the
//! destination's lock and makes a missing reference a refusal, not a dangling
//! link written into a second world.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wobu_core::{AssetKind, Id, Node, NodeKind, kind_def};

use crate::error::{Error, Result};
use crate::{Project, atomic, paths};

pub const TRANSFER_VERSION: u32 = 1;

/// One root the source project can export. Selecting it includes descendants
/// through `parent_id`; influence links do not pull unrelated entities in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferCandidate {
    pub root_id: Id,
    pub kind: NodeKind,
    pub name: String,
    pub node_count: usize,
    pub reference_count: usize,
    pub external_link_count: usize,
    pub missing_asset_count: usize,
    pub replaces_singleton: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferPreview {
    pub version: u32,
    pub source_project_id: Id,
    pub source_project_name: String,
    pub default_root_id: Option<Id>,
    pub candidates: Vec<TransferCandidate>,
    /// Reserved in the versioned envelope for project-pinned LoRAs. No such
    /// field exists in today's project schema, so this is honestly empty.
    pub pinned_loras: Vec<String>,
    pub lora_note: String,
}

/// A recoverable account of an apply. `completed == false` means an unexpected
/// concurrent or filesystem failure happened after at least one safe write;
/// the UI must leave this report visible instead of presenting an atomic
/// success. Content-addressed asset copies may remain as harmless orphans.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOutcome {
    pub completed: bool,
    pub root_id: Id,
    pub imported_root_id: Id,
    pub planned_node_count: usize,
    pub applied_node_ids: Vec<Id>,
    pub pending_node_ids: Vec<Id>,
    pub reference_count: usize,
    pub deduped_reference_count: usize,
    pub dropped_external_link_count: usize,
    pub replaced_singleton: bool,
    pub conflict_paths: Vec<String>,
    pub failure: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StagedAsset {
    pub(crate) id: Id,
    pub(crate) kind: AssetKind,
    pub(crate) bytes: Vec<u8>,
}

/// Fully self-contained input to the destination apply. Public because the
/// shell stages it off the destination lock, but constructible only here.
#[derive(Debug, Clone)]
pub struct TransferBundle {
    pub(crate) source_root: PathBuf,
    pub(crate) source_project_id: Id,
    pub(crate) root_id: Id,
    pub(crate) nodes: Vec<Node>,
    pub(crate) assets: Vec<StagedAsset>,
    pub(crate) external_link_count: usize,
    pub(crate) replaces_singleton: bool,
}

impl TransferBundle {
    pub fn source_project_id(&self) -> Id {
        self.source_project_id
    }
}

pub fn preview(source: &Path) -> Result<TransferPreview> {
    with_source(source, |project| {
        let nodes = project.world_nodes()?.to_vec();
        let assets = project.list_assets()?;
        let available: HashSet<Id> = assets
            .iter()
            .filter(|asset| paths::from_rel_string(project.root(), &asset.rel_path).is_file())
            .map(|asset| asset.id)
            .collect();

        let candidates = nodes
            .iter()
            .map(|root| candidate(root, &nodes, &available))
            .collect();
        let default_root_id = nodes
            .iter()
            .find(|node| node.kind == NodeKind::StyleGuide)
            .map(|node| node.id);

        Ok(TransferPreview {
            version: TRANSFER_VERSION,
            source_project_id: project.id(),
            source_project_name: project.meta().name.clone(),
            default_root_id,
            candidates,
            pinned_loras: Vec::new(),
            lora_note: "ComfyUI LoRAs are installed on this computer, not stored in a Wobu project, so they are not copied.".to_string(),
        })
    })
}

/// Read every selected node and every blob it names before the destination is
/// allowed to change. The asset id is re-derived from the bytes: a truncated or
/// incorrectly synchronised source blob therefore cannot masquerade as the
/// content its filename claims to be.
pub fn stage(source: &Path, root_id: Id) -> Result<TransferBundle> {
    let source_root = canonical(source)?;
    with_source(&source_root, |project| {
        let all = project.world_nodes()?.to_vec();
        let Some(root) = all.iter().find(|node| node.id == root_id) else {
            return Err(Error::NoSuchNode(root_id.to_string()));
        };
        let replaces_singleton = kind_def(root.kind).singleton;
        let selected = selected_ids(root_id, &all);
        let nodes: Vec<Node> = all
            .into_iter()
            .filter(|node| selected.contains(&node.id))
            .collect();
        let referenced = referenced_assets(&nodes);
        let indexed: HashMap<Id, _> = project
            .list_assets()?
            .into_iter()
            .map(|asset| (asset.id, asset))
            .collect();
        let mut assets = Vec::with_capacity(referenced.len());
        for id in referenced {
            let asset = indexed.get(&id).ok_or_else(|| Error::NoSuchAsset(id.to_string()))?;
            let path = paths::from_rel_string(project.root(), &asset.rel_path);
            let bytes = std::fs::read(&path).map_err(|error| Error::io(&path, error))?;
            let hash = atomic::hash_bytes(&bytes);
            if crate::assets::asset_id(&hash) != Some(id) {
                return Err(Error::NoSuchAsset(id.to_string()));
            }
            crate::assets::validate_import(&bytes)?;
            assets.push(StagedAsset { id, kind: asset.kind, bytes });
        }

        let external_link_count = nodes
            .iter()
            .flat_map(|node| &node.links)
            .filter(|link| !selected.contains(&link.to_id))
            .count();
        Ok(TransferBundle {
            source_root: source_root.clone(),
            source_project_id: project.id(),
            root_id,
            nodes,
            assets,
            external_link_count,
            replaces_singleton,
        })
    })
}

fn candidate(root: &Node, all: &[Node], available: &HashSet<Id>) -> TransferCandidate {
    let selected = selected_ids(root.id, all);
    let nodes: Vec<&Node> = all.iter().filter(|node| selected.contains(&node.id)).collect();
    let referenced = referenced_assets_ref(&nodes);
    TransferCandidate {
        root_id: root.id,
        kind: root.kind,
        name: root.name.clone(),
        node_count: nodes.len(),
        reference_count: referenced.len(),
        external_link_count: nodes
            .iter()
            .flat_map(|node| &node.links)
            .filter(|link| !selected.contains(&link.to_id))
            .count(),
        missing_asset_count: referenced.difference(available).count(),
        replaces_singleton: kind_def(root.kind).singleton,
    }
}

fn selected_ids(root: Id, all: &[Node]) -> HashSet<Id> {
    let mut selected = HashSet::from([root]);
    loop {
        let before = selected.len();
        for node in all {
            if node.parent_id.is_some_and(|parent| selected.contains(&parent)) {
                selected.insert(node.id);
            }
        }
        if selected.len() == before {
            return selected;
        }
    }
}

fn referenced_assets(nodes: &[Node]) -> HashSet<Id> {
    referenced_assets_ref(&nodes.iter().collect::<Vec<_>>())
}

fn referenced_assets_ref(nodes: &[&Node]) -> HashSet<Id> {
    let mut ids = HashSet::new();
    for node in nodes {
        ids.extend(node.asset_links.iter().map(|link| link.asset_id));
        ids.extend(node.cover_asset_id);
    }
    ids
}

fn with_source<T>(source: &Path, f: impl FnOnce(&mut Project) -> Result<T>) -> Result<T> {
    let canonical = canonical(source)?;
    let scratch = std::env::temp_dir().join(format!("wobu-transfer-{}.sqlite", wobu_core::new_id()));
    let result = Project::open_at_index(&canonical, &scratch).and_then(|mut project| f(&mut project));
    for path in [scratch.clone(), scratch.with_extension("sqlite-wal"), scratch.with_extension("sqlite-shm")] {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn canonical(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).map_err(|error| Error::io(path, error))
}
