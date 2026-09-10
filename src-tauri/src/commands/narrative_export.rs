//! Build an immutable, verified source snapshot and publish only its runtime assets.
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tauri::State;
use wobu_narrative::{Name, VarType};
use wobu_narrative_compiler::{CompileDiagnostic, CompileOptions, Profile, compile_with_analysis};
use wobu_narrative_package::Package;
use wobu_store::Project;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportCheck {
    pub diagnostics: Vec<CompileDiagnostic>,
    pub locale_diagnostics: Vec<wobu_narrative_locale::Diagnostic>,
    pub media_diagnostics: Vec<wobu_narrative_media::Diagnostic>,
    pub payload_hash: Option<String>,
    pub scenes: usize,
    pub strings: usize,
    pub bytes: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    destination: String,
    payload_hash: String,
    scenes: usize,
    strings: usize,
    bytes: u64,
}
fn package_error(error: wobu_narrative_package::Error) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}

#[tauri::command]
pub fn narrative_export_check(
    state: State<'_, AppState>,
    profile: Profile,
    commands: BTreeMap<Name, Vec<VarType>>,
    debug: bool,
) -> CommandResult<ExportCheck> {
    state.reconcile_now()?;
    state.with(|project| prepare(project, profile, commands, debug).map(|(check, _)| check))
}
#[tauri::command]
pub async fn narrative_export(
    state: State<'_, AppState>,
    destination: String,
    profile: Profile,
    commands: BTreeMap<Name, Vec<VarType>>,
    debug: bool,
    expected_hash: String,
) -> CommandResult<ExportReport> {
    state.reconcile_now()?;
    let (check, package) = state.with(|project| {
        let dest = PathBuf::from(&destination);
        let parent = dest.parent().and_then(|path| path.canonicalize().ok()).ok_or_else(|| {
            WobuError::new(
                Code::Invalid,
                "Choose a new folder inside an existing destination directory.",
            )
        })?;
        if parent.starts_with(
            project
                .root()
                .canonicalize()
                .map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?,
        ) {
            return Err(WobuError::new(
                Code::Invalid,
                "Choose a destination outside the project folder.",
            ));
        }
        prepare(project, profile, commands, debug)
    })?;
    let package = package.ok_or_else(|| {
        WobuError::new(Code::Invalid, "Resolve export blockers before exporting.")
    })?;
    if expected_hash != package.manifest.payload_hash {
        return Err(WobuError::new(
            Code::Invalid,
            "Saved narrative changed after validation. Check the export again.",
        ));
    }
    let path = PathBuf::from(&destination);
    let report = ExportReport {
        destination,
        payload_hash: expected_hash,
        scenes: check.scenes,
        strings: check.strings,
        bytes: check.bytes,
    };
    tauri::async_runtime::spawn_blocking(move || wobu_narrative_package::publish(&package, &path))
        .await
        .map_err(|e| {
            WobuError::new(
                Code::Invalid,
                format!("Export stopped; an incomplete folder may remain: {e}"),
            )
        })?
        .map_err(package_error)?;
    Ok(report)
}
pub(super) fn prepare(
    project: &Project,
    profile: Profile,
    commands: BTreeMap<Name, Vec<VarType>>,
    debug: bool,
) -> CommandResult<(ExportCheck, Option<Package>)> {
    prepare_checked(project, profile, commands, debug, || {})
}
fn prepare_checked(
    project: &Project,
    profile: Profile,
    commands: BTreeMap<Name, Vec<VarType>>,
    debug: bool,
    after_read: impl FnOnce(),
) -> CommandResult<(ExportCheck, Option<Package>)> {
    if debug && profile == Profile::Release {
        return Err(WobuError::new(
            Code::Invalid,
            "Debug source maps are available only for development exports.",
        ));
    }
    let analysis = project.narrative_analysis_capture()?;
    let fingerprint = project.narrative_fingerprint()?;
    let world = project.world_document()?.map(|(world, _)| world).unwrap_or_default();
    let catalog = project.scene_catalog()?;
    if !catalog.unreadable.is_empty()
        || (catalog.scenes.is_empty() && project.text_catalog()?.assets.is_empty())
    {
        return Err(WobuError::new(
            Code::Malformed,
            "Create a scene or supporting text asset and repair unreadable source files before exporting.",
        ));
    }
    let files = catalog
        .scenes
        .iter()
        .map(|entry| project.load_scene(entry.id))
        .collect::<Result<Vec<_>, _>>()?;
    let state_document = project.state_document()?;
    let schema = state_document
        .as_ref()
        .map(|(document, _)| document.schema())
        .transpose()
        .map_err(|e| WobuError::new(Code::Malformed, e.to_string()))?
        .unwrap_or_default();
    let (known_entities, known_settings) = membership(project)?;
    let (wording_suppressions, _) = project.wording_suppressions()?;
    after_read();
    // Re-read every captured scene stamp as well as the aggregate source tree. This
    // detects a change during capture, including one overwritten back before the hash.
    let changed = files.iter().any(|file| {
        wobu_store::atomic::read_stamped(&project.root().join(&file.rel))
            .ok()
            .flatten()
            .map(|(_, stamp)| stamp)
            != file.stamp
    });
    let current_membership = membership(project)?;
    let current_state_stamp = project.state_document()?.map(|(_, stamp)| stamp);
    let state_changed = current_state_stamp != state_document.map(|(_, stamp)| stamp);
    if changed
        || state_changed
        || current_membership != (known_entities.clone(), known_settings.clone())
        || project.narrative_fingerprint()? != fingerprint
    {
        return Err(WobuError::new(
            Code::Invalid,
            "Narrative source changed while capturing the export. Check again.",
        ));
    }
    let scenes: Vec<_> = files.into_iter().map(|file| file.scene).collect();
    let verified_reviews = super::narrative_review::verified(project, &scenes, &fingerprint)?;
    // Supporting text is read after the fingerprint check above, and the check
    // covers it: `narrative_fingerprint` walks `narrative/texts/` too, so an
    // asset edited mid-export aborts rather than shipping half of an edit.
    let texts = project.text_assets()?;
    let mut verified_text_reviews = BTreeMap::new();
    let snapshots = project.review_snapshots(
        &texts
            .iter()
            .map(|asset| wobu_narrative::SceneId::from_raw(asset.id.raw()))
            .collect::<Vec<_>>(),
        None,
    )?;
    for (asset, snapshot) in texts.iter().zip(&snapshots) {
        if snapshot.scene().editorial_text().as_ref() != Some(asset) {
            return Err(WobuError::new(
                Code::Invalid,
                "Supporting text changed while capturing its approvals.",
            ));
        }
        verified_text_reviews.extend(snapshot.text_evidence()?);
    }
    project.verify_review_snapshots(&snapshots)?;
    if project.narrative_fingerprint()? != fingerprint {
        return Err(WobuError::new(
            Code::Invalid,
            "Narrative source changed while capturing text approvals. Check again.",
        ));
    }
    let report = compile_with_analysis(
        &scenes,
        &texts,
        &schema,
        &CompileOptions {
            profile,
            known_entities,
            known_settings,
            commands,
            verified_reviews,
            verified_text_reviews,
            // Read before the fingerprint check below, so a suppression added
            // mid-export aborts the capture rather than silencing a warning in
            // half of it.
            wording_suppressions,
            quests: world.quests.clone(),
        },
        &world,
        &analysis.policies,
    );
    analysis.check_current(project)?;
    let (mut locales, locale_diagnostics) = project.locale_release()?;
    let locale_blocked = locale_diagnostics.iter().any(|d| d.code == "missing_translation");
    if locale_blocked {
        // Development can omit unavailable target locales, but its source table
        // must still use the configured source locale rather than Package's default.
        locales.policy.required.clear();
        locales.strings.clear();
    }
    let mut media = project.media_release()?;
    let media_blocked = media.diagnostics.iter().any(|d| d.code == "missing_media");
    if profile == Profile::Development {
        media.bundle.required.clear();
        media.bundle.timing.clear();
        media.bundle.fallback.clear();
        if locale_blocked {
            media.bundle.takes.retain(|_, take| take.key.locale == locales.policy.source);
            let paths: BTreeSet<_> = media
                .bundle
                .takes
                .values()
                .flat_map(|take| {
                    std::iter::once(&take.audio.path).chain(take.timing.iter().map(|b| &b.path))
                })
                .cloned()
                .collect();
            media.files.retain(|path, _| paths.contains(path));
        }
    }
    let package = report
        .graph
        .filter(|_| profile != Profile::Release || (!locale_blocked && !media_blocked))
        .map(|graph| {
            Package::build(graph, debug)
                .and_then(|p| p.with_locales(locales))
                .and_then(|p| p.with_media(media.bundle, media.files))
        })
        .transpose()
        .map_err(package_error)?;
    let package = if locale_blocked && profile == Profile::Release { None } else { package };
    if project.narrative_fingerprint()? != fingerprint {
        return Err(WobuError::new(
            Code::Invalid,
            "Narrative changed while checking locales. Check export again.",
        ));
    }
    analysis.check_current(project)?;
    let check = ExportCheck {
        media_diagnostics: media.diagnostics,
        locale_diagnostics,
        diagnostics: report.diagnostics,
        payload_hash: package.as_ref().map(|p| p.manifest.payload_hash.clone()),
        scenes: scenes.len(),
        strings: package
            .as_ref()
            .map(Package::string_count)
            .transpose()
            .map_err(package_error)?
            .unwrap_or(0),
        bytes: package.as_ref().map(Package::total_bytes).unwrap_or(0),
    };
    Ok((check, package))
}

#[cfg(test)]
mod tests;

/// The character and `setting` nodes a compilation has to resolve against.
///
/// Read in one pass and returned together, because the export re-reads this to
/// detect a change during capture: two passes could see the world before an edit
/// for one kind and after it for the other, and agree with neither.
type Membership = (BTreeSet<wobu_narrative::EntityId>, BTreeSet<wobu_narrative::EntityId>);

fn membership(project: &Project) -> CommandResult<Membership> {
    let mut characters = BTreeSet::new();
    let mut settings = BTreeSet::new();
    for summary in project.list_nodes()? {
        let node = project.get_node(summary.id)?;
        if node.id != summary.id {
            return Err(WobuError::new(
                Code::Invalid,
                "World entity identity changed. Reload before exporting.",
            ));
        }
        match node.kind {
            wobu_core::NodeKind::Character => characters.insert(node.id),
            wobu_core::NodeKind::Setting => settings.insert(node.id),
            _ => false,
        };
    }
    Ok((characters, settings))
}
