//! Named portable regression sources. Only explicit saves write a scenario record;
//! compiling and running a scenario never applies its state to canonical project data.
use super::narrative_preview::{bridge_integers, compile_project};
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use wobu_core::Id;
use wobu_narrative_compiler::CompileDiagnostic;
use wobu_narrative_scenarios::{RunReport, Scenario, run};
use wobu_store::{
    NarrativeRecordDocument, NarrativeRecordFile, NarrativeRecordKind, Project, SourceSave,
    atomic::Stamp,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFile {
    pub id: Id,
    pub name: String,
    pub scenario: Scenario,
    pub stamp: Option<Stamp>,
}
fn invalid(error: impl std::fmt::Display) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}
fn decode(record: NarrativeRecordFile) -> CommandResult<ScenarioFile> {
    let scenario: Scenario = serde_json::from_value(record.document.payload).map_err(invalid)?;
    scenario.validate().map_err(invalid)?;
    bridge_integers(&scenario)?;
    Ok(ScenarioFile {
        id: record.document.id,
        name: record.document.name,
        scenario,
        stamp: record.stamp,
    })
}
#[tauri::command]
pub fn narrative_scenarios_list(state: State<'_, AppState>) -> CommandResult<Vec<ScenarioFile>> {
    state.with(|project| list(project))
}
fn list(project: &Project) -> CommandResult<Vec<ScenarioFile>> {
    project.narrative_records(NarrativeRecordKind::Scenario)?.into_iter().map(decode).collect()
}
#[tauri::command]
pub fn narrative_scenario_save(
    state: State<'_, AppState>,
    id: Option<Id>,
    name: String,
    source: String,
    expected: Option<Stamp>,
) -> CommandResult<ScenarioFile> {
    let scenario: Scenario = serde_json::from_str(&source).map_err(invalid)?;
    state.with(|project| save(project, id, name, scenario, expected))
}
fn save(
    project: &mut Project,
    id: Option<Id>,
    name: String,
    scenario: Scenario,
    expected: Option<Stamp>,
) -> CommandResult<ScenarioFile> {
    scenario.validate().map_err(invalid)?;
    bridge_integers(&scenario)?;
    if name.trim().is_empty() {
        return Err(invalid("Give the scenario a name"));
    }
    let id = id.unwrap_or_else(wobu_core::new_id);
    if let Some(existing) = project.narrative_record(NarrativeRecordKind::Scenario, id)? {
        decode(existing)?;
    }
    let mut file = NarrativeRecordFile {
        document: NarrativeRecordDocument::new(
            NarrativeRecordKind::Scenario,
            id,
            name.trim().to_owned(),
            serde_json::to_value(scenario).map_err(invalid)?,
        ),
        stamp: expected,
    };
    match project.save_narrative_record(&mut file)? {
        SourceSave::Saved(_) => decode(file),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}
#[derive(Debug, Serialize)]
pub struct ScenarioResult {
    pub id: Id,
    pub report: Option<RunReport>,
    pub diagnostics: Vec<CompileDiagnostic>,
}
#[tauri::command]
pub async fn narrative_scenario_run(
    state: State<'_, AppState>,
    id: Id,
) -> CommandResult<ScenarioResult> {
    state.with(|project| run_saved(project, id))
}
fn run_saved(project: &Project, id: Id) -> CommandResult<ScenarioResult> {
    let record = project
        .narrative_record(NarrativeRecordKind::Scenario, id)?
        .ok_or_else(|| invalid("Scenario no longer exists; reload the list"))?;
    let scenario = decode(record)?.scenario;
    let compiled = compile_project(project, scenario.commands.clone())?;
    let report =
        compiled.graph.as_ref().map(|graph| run(graph, &scenario)).transpose().map_err(invalid)?;
    let result = ScenarioResult { id, report, diagnostics: compiled.diagnostics };
    bridge_integers(&result)?;
    Ok(result)
}
#[cfg(test)]
mod tests;
