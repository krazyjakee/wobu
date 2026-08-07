//! The sheet of views a mesh is reconstructed from.
//!
//! Read-only: it groups the generations already on a node into the batches and
//! slots the review UI draws. Which take is selected per slot is the user's
//! decision and lives in the receipt, not here.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::State;
use wobu_core::{Generation, Id};
use wobu_imagine::View;

use crate::error::CommandResult;
use crate::state::AppState;

/// One rendered take of one view.
///
/// A *take* rather than a cell of a batch, because a per-view reroll is by
/// definition a second answer for one position: the Turnaround preset locks one
/// seed across all eight views, so re-rolling the back view has to use a
/// different seed and would fall outside its own batch if batches were the only
/// unit here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundTake {
    pub generation_id: Id,
    pub asset_id: Id,
    pub seed: u64,
    pub created_at: DateTime<Utc>,
    pub backend: String,
    pub model: String,
}

/// Every take for one of the eight views, newest first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundSlot {
    pub view_type: String,
    pub takes: Vec<TurnaroundTake>,
}

/// One complete eight-view run, identified by the seed the preset locked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundBatch {
    pub seed: u64,
    pub created_at: DateTime<Utc>,
    /// In `View::ALL` order, which is the order the mesh request sends them.
    pub generation_ids: Vec<Id>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnaroundSheet {
    /// Exactly eight, in `View::ALL` order, whether or not they have takes.
    pub views: Vec<TurnaroundSlot>,
    /// Complete runs, newest first. Empty is the ordinary state before the
    /// Turnaround preset has ever been generated for this entity.
    pub batches: Vec<TurnaroundBatch>,
    /// View names with nothing rendered yet, in `View::ALL` order.
    pub missing: Vec<String>,
}

/// What this entity has rendered towards a mesh.
///
/// Reads receipts only — no image bytes and no mesh bytes — so opening the 3D
/// tab over a share stays as cheap as `mesh_concepts` already is.
#[tauri::command]
pub fn turnaround_sheet(state: State<'_, AppState>, node_id: Id) -> CommandResult<TurnaroundSheet> {
    state.with(|project| Ok(sheet(&project.list_generations(node_id)?)))
}

pub(super) fn sheet(generations: &[Generation]) -> TurnaroundSheet {
    let mut slots: Vec<TurnaroundSlot> = View::ALL
        .iter()
        .map(|view| TurnaroundSlot { view_type: view.to_string(), takes: Vec::new() })
        .collect();
    // Seed → the take chosen for each view of that run. A run that generated a
    // view twice (which the preset cannot do, but a hand-edited project can)
    // keeps the newest, because that is the one the sheet shows.
    let mut by_seed: HashMap<u64, HashMap<View, TurnaroundTake>> = HashMap::new();

    for generation in generations {
        let Some(view) = generation.view_type.as_deref().and_then(View::parse) else { continue };
        let Some(asset_id) = generation.output_asset_ids.first().copied() else { continue };
        let take = TurnaroundTake {
            generation_id: generation.id,
            asset_id,
            seed: generation.seed,
            created_at: generation.created_at,
            backend: generation.backend.clone(),
            model: generation.model.clone(),
        };
        let run = by_seed.entry(generation.seed).or_default();
        match run.get(&view) {
            Some(existing) if existing.created_at >= take.created_at => {}
            _ => {
                run.insert(view, take.clone());
            }
        }
        let index = View::ALL.iter().position(|candidate| *candidate == view).unwrap_or_default();
        slots[index].takes.push(take);
    }

    for slot in &mut slots {
        slot.takes.sort_by(|left, right| {
            let newest = right.created_at.cmp(&left.created_at);
            newest.then(right.generation_id.cmp(&left.generation_id))
        });
    }

    let mut batches: Vec<TurnaroundBatch> = by_seed
        .into_iter()
        .filter(|(_, run)| run.len() == View::ALL.len())
        .map(|(seed, run)| TurnaroundBatch {
            seed,
            created_at: run.values().map(|take| take.created_at).max().unwrap_or_else(Utc::now),
            generation_ids: View::ALL.iter().map(|view| run[view].generation_id).collect(),
        })
        .collect();
    batches.sort_by(|left, right| {
        right.created_at.cmp(&left.created_at).then(right.seed.cmp(&left.seed))
    });

    let missing = slots
        .iter()
        .filter(|slot| slot.takes.is_empty())
        .map(|slot| slot.view_type.clone())
        .collect();

    TurnaroundSheet { views: slots, batches, missing }
}

/* ── what the mesh backend will take ──────────────────────────────────────── */
