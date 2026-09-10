//! Capture immutable authoring input before any future job can be queued.
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use tauri::State;
use wobu_narrative_context::{FrozenContext, Options, Selection};
use wobu_store::Project;

#[derive(Serialize)]
pub struct Freshness {
    current: bool,
    hash: String,
}

#[tauri::command]
pub fn narrative_context_capture(
    state: State<'_, AppState>,
    selection: Selection,
    state_json: String,
    token_budget: u32,
) -> CommandResult<FrozenContext> {
    let options = parse_capture_options(selection, &state_json, token_budget)?;
    state.reconcile_now()?;
    state.with(|project| capture(project, options, || {}))
}
#[tauri::command]
pub fn narrative_context_freshness(
    state: State<'_, AppState>,
    options: Options,
    expected_hash: String,
) -> CommandResult<Freshness> {
    state.reconcile_now()?;
    state.with(|project| {
        let current = capture(project, options, || {})?;
        Ok(Freshness { current: current.hash == expected_hash, hash: current.hash })
    })
}

// Keep authored numeric spelling intact until the Rust typed parser sees it.
// JSON.parse can otherwise round a near-limit decimal into a valid integer.
fn parse_capture_options(
    selection: Selection,
    state_json: &str,
    token_budget: u32,
) -> CommandResult<Options> {
    let state = serde_json::from_str(state_json).map_err(|error| invalid(error.to_string()))?;
    Ok(Options { selection, state, token_budget })
}

fn invalid(message: impl Into<String>) -> WobuError {
    WobuError::new(Code::Invalid, message)
}
pub(super) fn capture(
    project: &Project,
    options: Options,
    after_read: impl FnOnce(),
) -> CommandResult<FrozenContext> {
    let result = wobu_store::project::narrative_context::capture(project, options, after_read)?;
    if unsafe_integer(&serde_json::to_value(&result).map_err(|error| invalid(error.to_string()))?) {
        return Err(invalid(
            "Context contains integers outside the webview safe range. Narrow the authored values before inspecting through the desktop UI.",
        ));
    }
    Ok(result)
}
pub(super) fn unsafe_integer(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .is_none_or(|value| !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value)),
        serde_json::Value::Array(values) => values.iter().any(unsafe_integer),
        serde_json::Value::Object(values) => values.values().any(unsafe_integer),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
