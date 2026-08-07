//! Thumbnails, and the batching that keeps a grid scrolling.
//!
//! Every command here is bounded by [`super::THUMB_BATCH_LIMIT`]: the caller
//! sends the window it is about to draw. Generating one is idempotent and
//! cached on disk, so a second ask for a thumbnail that exists is a path
//! lookup rather than a decode.

use std::collections::{HashMap, HashSet};

use tauri::{AppHandle, Emitter, State};
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::Id;
use wobu_store::{GenerationPageRequest, Project};

use super::assets::ensure_thumb_unlocked;
use super::{THUMB_BATCH_LIMIT, blocking};
use crate::error::{Code, CommandResult, WobuError};
use crate::state::{AppState, WORLD_CHANGED};

/// Emitted while the library's missing thumbnails are being drawn. Payload is
/// `ScanProgress`, the same shape `project:open-progress` carries.
pub const THUMB_PROGRESS: &str = "assets:thumb-progress";

/// The absolute path of one blob's thumbnail, drawing it if the folder has not
/// got one.
///
/// **This is what a grid tile binds to, and it is the only thing it binds to.**
/// The path comes back for `convertFileSrc`, so the webview loads a ~30 KB WebP
/// over the asset protocol instead of being handed a base64 copy of a 40 MB
/// scan for every tile on screen. Full-resolution originals are `asset_original`
/// and are fetched one at a time, when an image is actually opened.
///
/// `async` with the work on a blocking thread, because drawing one is a decode
/// and a resize: cheap for a screenshot, a few hundred milliseconds for a
/// 6000px scan, and neither belongs on the thread painting the window.
///
/// `null` rather than an error for the three cases where there is legitimately
/// no thumbnail — no such asset, a read-only or unreachable folder, a blob whose
/// pixels will not decode. A tile draws a placeholder for all three; none of
/// them is something the user can act on.
#[tauri::command]
pub async fn asset_thumb(
    state: State<'_, AppState>,
    asset_id: Id,
) -> CommandResult<Option<String>> {
    let handle = state.handle();
    let prepared =
        handle.ticket(|project| Ok((project.thumb_target(asset_id)?, project.can_write_thumb())));
    let (project, (target, can_write)) = match prepared {
        Ok(prepared) => prepared,
        Err(error) if error.code == Code::ShareUnmounted => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(target) = target else { return Ok(None) };
    let root = project.root().to_path_buf();
    let work = target.clone();
    let thumbnail = blocking("The thumbnail thread stopped unexpectedly.", move || {
        ensure_thumb_unlocked(&root, &work, can_write)
    })
    .await??;
    let Some(thumbnail) = thumbnail else { return Ok(None) };

    // Pure local-index mutation under the mutex. The path was proved present
    // by `ensure` above; joining it does not touch the filesystem.
    let commit = handle.with_ticket(&project, |project| {
        if thumbnail.generated {
            project.verify_writable()?;
        }
        Ok(project.record_thumb_targets(std::slice::from_ref(&target))?)
    });
    if let Err(error) = commit {
        if error.code == Code::ShareUnmounted || error.code == Code::ReadOnly {
            return Ok(None);
        }
        return Err(error);
    }
    Ok(Some(
        wobu_store::paths::from_rel_string(project.root(), &thumbnail.rel_path)
            .to_string_lossy()
            .into_owned(),
    ))
}

/// Resolve/generate the thumbnails for one bounded history page in one IPC.
#[tauri::command]
pub async fn asset_thumb_batch(
    state: State<'_, AppState>,
    asset_ids: Vec<Id>,
) -> CommandResult<HashMap<String, String>> {
    if asset_ids.len() > THUMB_BATCH_LIMIT {
        return Err(WobuError::new(
            Code::Invalid,
            "A thumbnail page may contain at most 100 assets.",
        ));
    }
    thumb_paths(state.handle(), &asset_ids).await
}

/// One thumbnail per *node*, for every list that draws entities rather than blobs.
///
/// The navigator, the palette, the relation lists and the influence stack all
/// show entities, and an entity is not an asset — so the webview cannot use
/// `asset_thumb_batch` for them without first learning which picture stands for
/// which node, which is a question only the index can answer. Doing that here
/// keeps it at one IPC for a whole visible window rather than one per row,
/// which is the entire point (#146, and #97 before it).
///
/// Nodes with no picture are simply absent from the map rather than present
/// with a null: "no thumbnail" and "thumbnail not drawn yet" are the same thing
/// to a caller that draws a fallback for both, and an absent key is cheaper to
/// send for the common case of a text-only world.
#[tauri::command]
pub async fn node_thumb_batch(
    state: State<'_, AppState>,
    node_ids: Vec<Id>,
) -> CommandResult<HashMap<String, String>> {
    if node_ids.len() > THUMB_BATCH_LIMIT {
        return Err(WobuError::new(
            Code::Invalid,
            "A thumbnail page may contain at most 100 nodes.",
        ));
    }
    let pairs = match state.with(|project| node_thumb_assets(project, &node_ids)) {
        Ok(pairs) => pairs,
        // A closed or unmounted project draws placeholders, exactly as one
        // whose blobs will not decode does. Neither is actionable from a row.
        Err(error) if error.code == Code::NoProjectOpen || error.code == Code::ShareUnmounted => {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(error),
    };
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut asset_ids: Vec<Id> = Vec::with_capacity(pairs.len());
    let mut seen = HashSet::new();
    for (_, asset_id) in &pairs {
        if seen.insert(*asset_id) {
            asset_ids.push(*asset_id);
        }
    }
    let by_asset = thumb_paths(state.handle(), &asset_ids).await?;
    Ok(pairs
        .into_iter()
        .filter_map(|(node_id, asset_id)| {
            by_asset.get(&asset_id.to_string()).map(|path| (node_id.to_string(), path.clone()))
        })
        .collect())
}

/// The picture that stands for each node, in the order the caller asked.
///
/// Read from the local index rather than from `world_nodes`: the navigator asks
/// for this on the first paint, and materialising the whole world to answer it
/// would move a cost that today is paid only by projects that open the
/// Inspector onto every project that opens at all.
pub(super) fn node_thumb_assets(
    project: &Project,
    node_ids: &[Id],
) -> CommandResult<Vec<(Id, Id)>> {
    let mut pairs = Vec::new();
    let mut seen = HashSet::new();
    for node_id in node_ids {
        if !seen.insert(*node_id) {
            continue;
        }
        if let Some(asset_id) = node_thumb_asset(project.index(), *node_id)? {
            pairs.push((*node_id, asset_id));
        }
    }
    Ok(pairs)
}

/// Cover first, then the first live reference, then the newest concept output.
///
/// That order is the user's own: a cover is an explicit choice about how this
/// entity should be shown, so nothing may override it. A disabled reference is
/// still preferred over nothing, because `enabled` says whether a picture is
/// *sent to a backend* — it was never a statement about display.
pub(super) fn node_thumb_asset(
    index: &wobu_store::Index,
    node_id: Id,
) -> CommandResult<Option<Id>> {
    if let Some(cover) = index.cover_asset_of(node_id)? {
        return Ok(Some(cover));
    }
    let links = index.asset_links_of(node_id)?;
    if let Some(link) = links.iter().find(|link| link.enabled).or_else(|| links.first()) {
        return Ok(Some(link.asset_id));
    }
    let page = index.generation_page(&GenerationPageRequest {
        node_id: Some(node_id),
        offset: 0,
        limit: 1,
    })?;
    Ok(page.items.first().and_then(|item| item.first_asset_id))
}

/// The shared body of both thumbnail batches: asset ids in, absolute paths out.
async fn thumb_paths(handle: AppState, asset_ids: &[Id]) -> CommandResult<HashMap<String, String>> {
    let prepared = handle
        .ticket(|project| Ok((project.thumb_targets(asset_ids)?, project.can_write_thumb())))?;
    let (project, (targets, can_write)) = prepared;
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    let root = project.root().to_path_buf();
    let work = targets.clone();
    let completed = blocking("The thumbnail batch thread stopped unexpectedly.", move || {
        if !can_write {
            return Ok(work
                .iter()
                .filter(|target| wobu_store::thumbs::exists(&root, &target.hash))
                .map(|target| target.asset_id)
                .collect());
        }
        wobu_store::thumbs::ensure_all(&root, &work, &wobu_store::Cancel::new(), &mut |_| {})
    })
    .await??;
    let completed: HashSet<_> = completed.into_iter().collect();
    let completed_targets: Vec<_> =
        targets.iter().filter(|target| completed.contains(&target.asset_id)).cloned().collect();
    handle.with_ticket(&project, |project| {
        if can_write {
            project.verify_writable()?;
        }
        Ok(project.record_thumb_targets(&completed_targets)?)
    })?;
    Ok(completed_targets
        .into_iter()
        .map(|target| {
            let path = wobu_store::paths::from_rel_string(
                project.root(),
                &wobu_store::thumbs::rel_path(&target.hash),
            )
            .to_string_lossy()
            .into_owned();
            (target.asset_id.to_string(), path)
        })
        .collect())
}

/// Draw every thumbnail the open project is missing.
///
/// The other half of "missing thumbs are regenerated lazily": a folder that
/// arrived over sync, out of a zip or off a USB stick can have a full
/// `assets/originals/` and no `assets/thumbs/` at all. `asset_thumb` covers one
/// tile scrolling into view; this covers the case where the answer is "all of
/// them", and it exists so that the grid is not drawing a thousand placeholders
/// while a thousand separate commands queue up behind the project mutex.
///
/// Three steps for one reason, and it is the rule in `state.rs`: read the list
/// under the lock, grind through it with *nothing* held, then record the results
/// under the lock again. The middle step is minutes for a large library on a
/// share, and holding the mutex across it would freeze every other command.
///
/// Returns how many blobs now have a thumbnail. Progress is emitted rather than
/// returned, exactly as `project_open`'s is.
#[tauri::command]
pub async fn asset_thumbs_ensure(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<usize> {
    let prepared =
        state.ticket(|project| Ok((project.missing_thumbs()?, project.can_write_thumb())));
    let (project, (targets, can_write)) = match prepared {
        Ok(prepared) => prepared,
        Err(error) if error.code == Code::ShareUnmounted => return Ok(0),
        Err(error) => return Err(error),
    };
    if targets.is_empty() {
        return Ok(0);
    }
    let root = project.root().to_path_buf();
    let work = targets.clone();

    let cancel = state.begin_thumbs();
    let done = {
        let emitter = app.clone();
        blocking("The thumbnail thread stopped unexpectedly.", move || {
            let mut last = 0u8;
            if !can_write {
                return Ok(work
                    .iter()
                    .filter(|target| wobu_store::thumbs::exists(&root, &target.hash))
                    .map(|target| target.asset_id)
                    .collect());
            }
            wobu_store::thumbs::ensure_all(&root, &work, &cancel, &mut |p| {
                // Throttled to whole percentage points, for the reason
                // `project_open` throttles: a thousand events through the
                // bridge is work taken from the pass itself.
                let pct = p.percent();
                if pct != last {
                    last = pct;
                    let _ = emitter.emit(THUMB_PROGRESS, p);
                }
            })
        })
        .await?
    };
    state.finish_thumbs();

    // Cancelling reports nothing and loses nothing. `Cancelled` is not a
    // failure (see `wobu_store::Error`), and every thumbnail the pass did draw
    // is on disk at a path only that picture can claim — so the next tile to ask
    // for one, or the next run of this, finds it already there and free.
    let made = match done {
        Ok(made) => made,
        Err(wobu_store::Error::Cancelled) => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let made: HashSet<_> = made.into_iter().collect();
    let completed: Vec<_> =
        targets.into_iter().filter(|target| made.contains(&target.asset_id)).collect();
    state.with_ticket(&project, |project| {
        if can_write {
            project.verify_writable()?;
        }
        Ok(project.record_thumb_targets(&completed)?)
    })?;
    // The folder gained files, but under `assets/thumbs/` — which the watcher
    // does not treat as a world change, and which nothing would otherwise
    // invalidate. Without this the grid keeps its placeholders until something
    // else happens to touch the project.
    let _ = app.emit(WORLD_CHANGED, ());
    Ok(completed.len())
}

/// Stop a thumbnail pass in progress.
///
/// A no-op when there is none, for the same reason `project_open_cancel` is: the
/// user can press it at the moment the pass finishes and that race must not be
/// an error.
#[tauri::command]
pub fn asset_thumbs_cancel(state: State<'_, AppState>) {
    state.cancel_thumbs();
}
