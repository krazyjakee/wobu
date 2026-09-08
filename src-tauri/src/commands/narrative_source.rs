//! Source is an editable view of the same scene document used by the forms.
//! Reads retain exact bytes. Explicit Format/Save normalises YAML with the
//! model's printer, including removing comments; no second source is stored.
use serde::Serialize;
use tauri::State;
use wobu_narrative::{Scene, SceneDocument, SceneId, SourceLocation};
use wobu_store::{Project, atomic};

use super::narrative::{DiagnosticView, SceneFileView, diagnostics};
use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    yaml: String,
    file: SceneFileView,
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
    diagnostics: Vec<DiagnosticView>,
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
    let path = wobu_store::paths::from_rel_string(project.root(), &entry.rel);
    let (yaml, stamp) = atomic::read_stamped(&path)?.ok_or_else(|| {
        WobuError::new(Code::Invalid, "The scene file was removed. Reload the scene library.")
    })?;
    let document =
        SceneDocument::parse(&yaml).map_err(|e| WobuError::new(Code::Malformed, e.to_string()))?;
    if document.scene.id != scene_id {
        return Err(WobuError::new(
            Code::Invalid,
            "The scene identity changed. Reload the scene library.",
        ));
    }
    Ok(SourceView {
        file: SceneFileView {
            scene: document.scene,
            slug: entry.slug.clone(),
            rel: entry.rel.clone(),
            stamp: Some(stamp),
        },
        yaml,
    })
}

#[tauri::command]
pub fn narrative_source_check(
    state: State<'_, AppState>,
    scene_id: SceneId,
    yaml: String,
) -> CommandResult<SourceCheck> {
    state.with(|project| source_check(project, scene_id, &yaml))
}

fn source_check(project: &Project, scene_id: SceneId, yaml: &str) -> CommandResult<SourceCheck> {
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
    if document.scene.id != scene_id {
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
    let diagnostics = diagnostics(project, scene_id, Some(document.scene.clone()))?;
    Ok(SourceCheck {
        scene: Some(document.scene),
        formatted: Some(formatted),
        problem: None,
        diagnostics,
    })
}

#[cfg(test)]
mod tests;
