//! Source is an editable view of the same scene document used by the forms.
//! Reads retain exact bytes. Explicit Format/Save normalises YAML with the
//! model's printer, including removing comments; no second source is stored.
use serde::Serialize;
use tauri::State;
use wobu_narrative::{Scene, SceneCatalog, SceneDocument, SceneId, SourceLocation, SourcePathPart};
use wobu_store::{Project, atomic};

use super::narrative::{DiagnosticView, SceneFileView};
use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    yaml: String,
    file: Option<SceneFileView>,
    rel: String,
    stamp: atomic::Stamp,
    scene_id: Option<SceneId>,
    repair_blocked: bool,
    problem: Option<SourceProblem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProblem {
    message: String,
    location: Option<SourceLocation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCheck {
    scene: Option<Scene>,
    formatted: Option<String>,
    problem: Option<SourceProblem>,
    diagnostics: Vec<LocatedDiagnostic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocatedDiagnostic {
    #[serde(flatten)]
    diagnostic: DiagnosticView,
    source_path: Vec<SourcePathPart>,
}

#[tauri::command]
pub fn narrative_source_get(
    state: State<'_, AppState>,
    scene_id: SceneId,
) -> CommandResult<SourceView> {
    state.with(|project| source_get(project, scene_id))
}

fn source_get(project: &Project, scene_id: SceneId) -> CommandResult<SourceView> {
    let catalog = project.scene_catalog()?;
    let entry = catalog.find(scene_id).ok_or_else(|| {
        WobuError::new(Code::Invalid, "This scene no longer exists in the project.")
    })?;
    source_at(project, &entry.rel, Some(scene_id))
}

#[tauri::command]
pub fn narrative_source_open(state: State<'_, AppState>, rel: String) -> CommandResult<SourceView> {
    state.with(|project| {
        let known = project
            .scene_catalog()?
            .scenes
            .into_iter()
            .find(|entry| entry.rel == rel)
            .map(|entry| entry.id);
        source_at(project, &rel, known)
    })
}

fn source_at(
    project: &Project,
    rel: &str,
    expected_id: Option<SceneId>,
) -> CommandResult<SourceView> {
    let path = project.scene_source_path(rel)?;
    let (yaml, stamp) = atomic::read_stamped(&path)?.ok_or_else(|| {
        WobuError::new(Code::Invalid, "The source file was removed. Reload the scene library.")
    })?;
    let parsed = SceneDocument::parse(&yaml);
    let repair_blocked =
        matches!(&parsed, Err(wobu_narrative::Error::UnsupportedSchemaVersion { .. }));
    let (file, problem) = match parsed {
        Ok(document) if expected_id.is_none_or(|id| id == document.scene.id) => (
            Some(SceneFileView {
                scene: document.scene,
                slug: rel.rsplit('/').next().unwrap_or_default().trim_end_matches(".yaml").into(),
                rel: rel.into(),
                stamp: Some(stamp.clone()),
            }),
            None,
        ),
        Ok(_) => {
            return Err(WobuError::new(
                Code::Invalid,
                "The scene identity changed. Reload the scene library.",
            ));
        }
        Err(error) => (None, Some(source_problem(error))),
    };
    let scene_id = file.as_ref().map(|one| one.scene.id).or(expected_id);
    Ok(SourceView { yaml, file, rel: rel.into(), stamp, scene_id, problem, repair_blocked })
}

fn source_problem(error: wobu_narrative::Error) -> SourceProblem {
    let location = match &error {
        wobu_narrative::Error::Source { location, .. } => *location,
        _ => None,
    };
    SourceProblem { message: error.to_string(), location }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRepair {
    source: SourceView,
    recovery_rel: String,
}

#[tauri::command]
pub fn narrative_source_repair(
    state: State<'_, AppState>,
    rel: String,
    yaml: String,
    expected: atomic::Stamp,
    scene_id: Option<SceneId>,
) -> CommandResult<SourceRepair> {
    state.with(|project| source_repair(project, &rel, &yaml, &expected, scene_id))
}
fn source_repair(
    project: &mut Project,
    rel: &str,
    yaml: &str,
    expected: &atomic::Stamp,
    scene_id: Option<SceneId>,
) -> CommandResult<SourceRepair> {
    let document = SceneDocument::parse(yaml)
        .map_err(|error| WobuError::new(Code::Malformed, error.to_string()))?;
    super::narrative::validate_approval(None, &document.scene, false)?;
    let known = project
        .scene_catalog()?
        .scenes
        .into_iter()
        .find(|entry| entry.rel == rel)
        .map(|entry| entry.id)
        .or(scene_id);
    let (outcome, recovery_rel) = project.repair_scene_source(rel, &document, expected, known)?;
    match outcome {
        wobu_store::SourceSave::Saved(_) => Ok(SourceRepair {
            source: source_at(project, rel, Some(document.scene.id))?,
            recovery_rel,
        }),
        wobu_store::SourceSave::Conflict { conflict_path } => {
            Err(WobuError::conflict(conflict_path))
        }
    }
}

#[tauri::command]
pub fn narrative_source_check(
    state: State<'_, AppState>,
    scene_id: Option<SceneId>,
    yaml: String,
) -> CommandResult<SourceCheck> {
    state.with(|project| source_check(project, scene_id, &yaml))
}

fn source_check(
    project: &Project,
    scene_id: impl Into<Option<SceneId>>,
    yaml: &str,
) -> CommandResult<SourceCheck> {
    let scene_id = scene_id.into();
    let document = match SceneDocument::parse(yaml) {
        Ok(document) => document,
        Err(error) => {
            let location = match &error {
                wobu_narrative::Error::Source { location, .. } => *location,
                _ => None,
            };
            return Ok(SourceCheck {
                scene: None,
                formatted: None,
                problem: Some(SourceProblem { message: error.to_string(), location }),
                diagnostics: Vec::new(),
            });
        }
    };
    if scene_id.is_some_and(|id| document.scene.id != id) {
        return Ok(SourceCheck {
            scene: None,
            formatted: None,
            problem: Some(SourceProblem {
                message: "Keep the scene id unchanged. Create or duplicate a scene to give it a new identity.".into(),
                location: None,
            }),
            diagnostics: Vec::new(),
        });
    }
    let formatted = document.to_yaml().map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?;
    let schema = project.state_schema()?;
    let catalog = SceneCatalog::of(project.scene_ids()?);
    let diagnostics = document
        .scene
        .source_diagnostics(&schema, &catalog)
        .into_iter()
        .map(|(diagnostic, source_path)| LocatedDiagnostic {
            diagnostic: DiagnosticView::of(&diagnostic),
            source_path,
        })
        .collect();
    Ok(SourceCheck {
        scene: Some(document.scene),
        formatted: Some(formatted),
        problem: None,
        diagnostics,
    })
}

#[cfg(test)]
mod tests;
