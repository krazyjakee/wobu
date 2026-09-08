//! Compile a coherent source read and drive an isolated, stateless Preview.
//! A Preview command returns snapshots; it never writes world/project state.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;
use wobu_narrative::{Name, SceneId, VarType};
use wobu_narrative_compiler::{CompileOptions, CompileReport, Graph, compile};
use wobu_narrative_runtime::{
    CommandResult as HostResult, Runtime, Snapshot, State as Values, Yield,
};
use wobu_store::Project;

use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

#[tauri::command]
pub fn narrative_compile(
    state: State<'_, AppState>,
    commands: BTreeMap<Name, Vec<VarType>>,
) -> CommandResult<CompileReport> {
    state.with(|project| compile_project(project, commands))
}
pub(super) fn compile_project(
    project: &Project,
    commands: BTreeMap<Name, Vec<VarType>>,
) -> CommandResult<CompileReport> {
    let before = project.narrative_fingerprint()?;
    let catalog = project.scene_catalog()?;
    if !catalog.unreadable.is_empty() {
        return Err(WobuError::new(
            Code::Malformed,
            "Some scene files cannot be read. Repair the files listed in the Scene library before compiling.",
        ));
    }
    let scenes = catalog
        .scenes
        .iter()
        .map(|entry| project.load_scene(entry.id).map(|file| file.scene))
        .collect::<Result<Vec<_>, _>>()?;
    let schema = project.state_schema()?;
    // Match the character-only cast and speaker pickers. A prop or style guide
    // existing in the world does not make it a valid scene participant.
    let known_entities = project
        .list_nodes()?
        .iter()
        .filter(|node| node.kind == wobu_core::NodeKind::Character)
        .map(|node| node.id)
        .collect();
    if project.narrative_fingerprint()? != before {
        return Err(WobuError::new(
            Code::Invalid,
            "Narrative source changed during compilation. Try compiling again.",
        ));
    }
    let report = compile(
        &scenes,
        &schema,
        &CompileOptions { known_entities, commands, ..CompileOptions::default() },
    );
    if let Some(graph) = &report.graph {
        bridge_integers(graph)?;
    }
    Ok(report)
}

// Rust supports i64, but the webview carries JSON numbers as IEEE-754 doubles.
// Reject values it would round before a graph/snapshot crosses that boundary.
pub(super) fn bridge_integers(value: &impl Serialize) -> CommandResult<()> {
    fn exact(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Number(number) => {
                number.as_i64().is_some_and(|n| n.unsigned_abs() <= 9_007_199_254_740_991)
            }
            serde_json::Value::Array(items) => items.iter().all(exact),
            serde_json::Value::Object(fields) => fields.values().all(exact),
            _ => true,
        }
    }
    let value =
        serde_json::to_value(value).map_err(|e| WobuError::new(Code::Invalid, e.to_string()))?;
    if !exact(&value) {
        return Err(WobuError::new(
            Code::Invalid,
            "Desktop Preview requires integer values and bounds within ±9007199254740991. Narrow the declared range before previewing.",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct PreviewFrame {
    snapshot: Snapshot,
    site: wobu_narrative_runtime::TraceSite,
    current: Yield,
    state: Values,
    trace: wobu_narrative_runtime::ExecutionTrace,
}
fn frame(runtime: Runtime) -> CommandResult<PreviewFrame> {
    let frame = PreviewFrame {
        snapshot: runtime.snapshot(),
        site: runtime.site(),
        current: runtime.current().map_err(runtime_error)?,
        state: runtime.state().clone(),
        trace: runtime.trace().clone(),
    };
    // Valid inputs can still advance counters beyond the exact JS range.
    // Validate the result too, before it leaves Rust and loses precision.
    bridge_integers(&frame)?;
    Ok(frame)
}
fn runtime_error(error: wobu_narrative_runtime::Error) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}

#[tauri::command]
pub fn narrative_preview_start(
    graph: Graph,
    scene_id: SceneId,
    initial_state: Values,
    seed: Option<u64>,
) -> CommandResult<PreviewFrame> {
    bridge_integers(&graph)?;
    bridge_integers(&initial_state)?;
    bridge_integers(&seed)?;
    frame(
        Runtime::start_with_state(
            graph,
            &scene_id.to_string(),
            initial_state,
            wobu_core::new_id().to_string(),
            seed.unwrap_or(0),
            1000,
        )
        .map_err(runtime_error)?,
    )
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PreviewAction {
    Advance,
    Choose {
        #[serde(rename = "choiceId")]
        choice_id: String,
    },
    CompleteCommand {
        token: String,
        result: HostResult,
    },
    Restore,
}
#[tauri::command]
pub fn narrative_preview_step(
    graph: Graph,
    snapshot: Snapshot,
    action: PreviewAction,
) -> CommandResult<PreviewFrame> {
    bridge_integers(&graph)?;
    bridge_integers(&snapshot)?;
    let mut runtime = Runtime::restore(graph, snapshot).map_err(runtime_error)?;
    match action {
        PreviewAction::Advance => {
            runtime.advance().map_err(runtime_error)?;
        }
        PreviewAction::Choose { choice_id } => {
            runtime.choose(&choice_id).map_err(runtime_error)?;
        }
        PreviewAction::CompleteCommand { token, result } => {
            bridge_integers(&result)?;
            match runtime.complete_command(&token, result) {
                Ok(_)
                | Err(wobu_narrative_runtime::Error::CommandFailed(_))
                | Err(wobu_narrative_runtime::Error::CommandCancelled) => {}
                Err(error) => return Err(runtime_error(error)),
            }
        }
        PreviewAction::Restore => {}
    }
    frame(runtime)
}

#[cfg(test)]
mod tests;
