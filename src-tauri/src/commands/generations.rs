//! Reading back what was generated.
//!
//! Receipts, their pages, and the turnaround views a mesh is reconstructed
//! from. Nothing here generates: `crate::generate` produces these rows and
//! `commands::mesh` consumes them, and this is the surface in between.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::{Generation, Id, MeshAsset};
use wobu_imagine::View as MeshView;
use wobu_store::{GenerationPage, GenerationPageRequest, Project};

use super::{absolute, blocking};
use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

/// One bounded page of lightweight Concepts history, newest first in SQLite.
#[tauri::command]
pub fn generation_list(
    state: State<'_, AppState>,
    node_id: Id,
    offset: u32,
    limit: u32,
) -> CommandResult<GenerationPage> {
    state.with(|project| {
        generation_page(project, GenerationPageRequest { node_id: Some(node_id), offset, limit })
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundView {
    generation_id: Id,
    view_type: String,
    asset_id: Id,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshConcept {
    generation_id: Id,
    created_at: chrono::DateTime<chrono::Utc>,
    backend: String,
    model: String,
    asset: MeshAsset,
    turnaround: Vec<TurnaroundView>,
}

/// Lightweight 3D history. The directory scan reads fixed GLB headers only;
/// the complete mesh is not read or hashed until `mesh_asset_path` below.
#[tauri::command]
pub fn mesh_concepts(state: State<'_, AppState>, node_id: Id) -> CommandResult<Vec<MeshConcept>> {
    state.with(|project| {
        let generations = project.list_generations(node_id)?;
        let by_id: HashMap<_, _> = generations.iter().map(|item| (item.id, item)).collect();
        let meshes: HashMap<_, _> =
            project.list_meshes().into_iter().map(|mesh| (mesh.id, mesh)).collect();
        Ok(generations
            .iter()
            .filter_map(|generation| {
                let output = generation.mesh_output()?;
                let asset = meshes.get(&output.asset_id)?.clone();
                Some(MeshConcept {
                    generation_id: generation.id,
                    created_at: generation.created_at,
                    backend: generation.backend.clone(),
                    model: generation.model.clone(),
                    asset,
                    turnaround: turnaround_views(&output.turnaround_generation_ids, &by_id),
                })
            })
            .collect())
    })
}

fn turnaround_views(ids: &[Id], generations: &HashMap<Id, &Generation>) -> Vec<TurnaroundView> {
    if ids.len() != MeshView::ALL.len() {
        return Vec::new();
    }
    let views: Option<Vec<_>> = ids
        .iter()
        .map(|id| {
            let generation = generations.get(id)?;
            Some(TurnaroundView {
                generation_id: *id,
                view_type: generation.view_type.clone()?,
                asset_id: *generation.output_asset_ids.first()?,
            })
        })
        .collect();
    // A partial sheet is not "the sheet that produced this mesh". If even one
    // immutable source receipt is missing, show the explicit unavailable state.
    let views = views.unwrap_or_default();
    let distinct: HashSet<_> =
        views.iter().filter_map(|view| MeshView::parse(&view.view_type)).collect();
    if distinct.len() == MeshView::ALL.len() { views } else { Vec::new() }
}

/// Validate and expose one complete GLB. Async because this is the first full
/// mesh read and it may cross a slow share.
#[tauri::command]
pub async fn mesh_asset_path(
    state: State<'_, AppState>,
    asset_id: Id,
) -> CommandResult<Option<String>> {
    let (project_id, root) =
        state.with(|project| Ok((project.id(), project.root().to_path_buf())))?;
    let checked_root = root.clone();
    let mesh = blocking("The mesh validation thread stopped unexpectedly.", move || {
        wobu_store::assets::cached_mesh(&checked_root, project_id, asset_id)
    })
    .await??;
    let Some((_mesh, cached)) = mesh else { return Ok(None) };
    Ok(state
        .peek(|project| project.is_some_and(|project| project.id() == project_id))
        .then(|| cached.to_string_lossy().into_owned()))
}

/// Canonical project path for Finder/Explorer. Unlike the viewer path this is
/// not a local cache, and unlike loading it does not read the GLB body.
#[tauri::command]
pub fn mesh_source_path(state: State<'_, AppState>, asset_id: Id) -> CommandResult<Option<String>> {
    state.with(|project| {
        Ok(project
            .list_meshes()
            .into_iter()
            .find(|mesh| mesh.id == asset_id)
            .and_then(|mesh| absolute(project, &mesh.rel_path)))
    })
}

/// Copy a validated GLB to the location chosen by the modeller.
#[tauri::command]
pub async fn mesh_export(
    state: State<'_, AppState>,
    asset_id: Id,
    destination: String,
) -> CommandResult<()> {
    if destination.trim().is_empty() {
        return Err(WobuError::new(Code::Invalid, "Choose where to export the GLB."));
    }
    let destination = PathBuf::from(destination);
    let (project_id, root) =
        state.with(|project| Ok((project.id(), project.root().to_path_buf())))?;
    blocking("The mesh export thread stopped unexpectedly.", move || {
        let (_mesh, cached) = wobu_store::assets::cached_mesh(&root, project_id, asset_id)?
            .ok_or_else(|| wobu_store::Error::NoSuchAsset(asset_id.to_string()))?;
        std::fs::copy(&cached, &destination)
            .map(|_| ())
            .map_err(|error| wobu_store::Error::io(&destination, error))
    })
    .await??;
    Ok(())
}

/// The full immutable receipt for the one tile a person opened.
#[tauri::command]
pub fn generation_get(
    state: State<'_, AppState>,
    generation_id: Id,
) -> CommandResult<Option<Generation>> {
    state.with(|project| Ok(project.get_generation(generation_id)?))
}

/// Remove a generation from Concepts without erasing its spend record.
#[tauri::command]
pub fn generation_delete(state: State<'_, AppState>, generation_id: Id) -> CommandResult<()> {
    state.with(|project| Ok(project.delete_generation(generation_id)?))
}

fn generation_page(
    project: &Project,
    request: GenerationPageRequest,
) -> CommandResult<GenerationPage> {
    let mut page = project.generation_page(&request)?;
    for item in &mut page.items {
        if let Some(relative) = item.thumbnail_path.as_deref() {
            item.thumbnail_path = Some(
                wobu_store::paths::from_rel_string(project.root(), relative)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    Ok(page)
}
