//! Beat matrices stay keyless; explicit structural materialization feeds the existing build queue.
use crate::{
    error::{Code, CommandResult, WobuError},
    state::AppState,
};
use serde::Serialize;
use tauri::State;
use wobu_core::Id;
use wobu_narrative::{BeatId, DialogueSlotId, SceneId};
use wobu_narrative_variants::{self as variants, Policy, Target};
use wobu_store::{
    Project,
    project::narrative_variants::{AnalysisCapture, AnalysisReport},
};
fn invalid(error: impl std::fmt::Display) -> WobuError {
    WobuError::new(Code::Invalid, error.to_string())
}
async fn work<T: Send + 'static>(
    state: &AppState,
    operation: impl FnOnce(&mut Project) -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    let (ticket, ()) = state.ticket(|_| Ok(()))?;
    let state = state.handle();
    super::blocking("Variant planning worker stopped.", move || {
        state.with_ticket(&ticket, |_| Ok(()))?;
        if state.reconcile_project_now(ticket.project)? {
            state.announce_local_change(ticket.project);
        }
        state.with_ticket(&ticket, operation)
    })
    .await?
}
#[derive(Serialize)]
pub struct View {
    pub policy: Policy,
    pub capture: AnalysisCapture,
    pub latest: Option<AnalysisReport>,
    pub stale: bool,
}
#[tauri::command]
pub async fn narrative_variants_get(
    state: State<'_, AppState>,
    scene: SceneId,
    beat: BeatId,
) -> CommandResult<View> {
    work(&state, move |project| view(project, Target { scene, beat })).await
}
fn view(project: &Project, target: Target) -> CommandResult<View> {
    let file = project.load_scene(target.scene)?;
    if file.scene.beat(target.beat).is_none() {
        return Err(invalid("Select an existing beat."));
    }
    let capture = project.narrative_analysis_capture()?;
    let schema = project.state_schema()?;
    let policy = capture.policies.iter().find(|p| p.target == target).cloned().unwrap_or(Policy {
        version: variants::VERSION,
        target: target.clone(),
        initial: vec![schema.iter().map(|d| (d.name.clone(), d.default.clone())).collect()],
        invariants: vec![],
        quests: vec![],
        events: vec![],
        external: true,
        limits: variants::Limits::default(),
    });
    let mut latest = None;
    for file in project.narrative_records(wobu_store::NarrativeRecordKind::Receipt)? {
        if file.document.payload["type"] == "narrative_analysis_report"
            && file.document.payload["analysis"]["report"]["target"]
                == serde_json::to_value(&target).map_err(invalid)?
        {
            latest = Some(project.narrative_analysis_report(file.document.id)?);
        }
    }
    let fingerprint = project.narrative_fingerprint()?;
    let stale = latest.as_ref().is_some_and(|saved| {
        saved.policies.guard != capture.guard || fingerprint != saved.source_guard
    });
    Ok(View { policy, capture, latest, stale })
}
#[tauri::command]
pub async fn narrative_variants_plan(
    state: State<'_, AppState>,
    source: String,
    guard: String,
) -> CommandResult<AnalysisReport> {
    work(&state, move |project| {
        plan(project, serde_json::from_str(&source).map_err(invalid)?, &guard)
    })
    .await
}
pub(crate) fn plan(
    project: &mut Project,
    policy: Policy,
    guard: &str,
) -> CommandResult<AnalysisReport> {
    let capture = project.narrative_analysis_capture()?;
    if capture.guard != guard {
        return Err(invalid("Analysis policies changed. Reload before saving."));
    }
    let capture = project.save_narrative_analysis_policy(policy.clone(), &capture)?;
    let snapshot = project.narrative_dependency_snapshot()?;
    let report = variants::analyze(
        variants::Input {
            scenes: &snapshot.scenes,
            schema: &snapshot.schema,
            world: &snapshot.world,
        },
        &policy,
    )
    .map_err(invalid)?;
    snapshot.check_current(project)?;
    capture.check_current(project)?;
    let saved = AnalysisReport {
        id: Id::generate(),
        world_guard: variants::hash(&(&snapshot.world, snapshot.schema.iter().collect::<Vec<_>>())),
        report,
        source_guard: snapshot.fingerprint,
        policies: capture,
    };
    super::narrative_preview::bridge_integers(&saved)?;
    project.save_narrative_analysis_report(&saved)?;
    Ok(saved)
}
#[derive(Serialize)]
pub struct Materialized {
    pub before: super::narrative::SceneFileView,
    pub after: super::narrative::SceneFileView,
}
#[tauri::command]
pub async fn narrative_variants_materialize(
    state: State<'_, AppState>,
    report: Id,
    slot: DialogueSlotId,
    rows: Vec<String>,
) -> CommandResult<Materialized> {
    work(&state, move |project| {
        let materialized = project.materialize_narrative_analysis(report, slot, &rows)?;
        Ok(Materialized {
            before: super::narrative::SceneFileView::of(&materialized.before),
            after: super::narrative::SceneFileView::of(&materialized.after),
        })
    })
    .await
}
#[tauri::command]
pub async fn narrative_variants_build(
    state: State<'_, AppState>,
    report: Id,
    slot: DialogueSlotId,
    rows: Vec<String>,
) -> CommandResult<wobu_narrative_build::Build> {
    work(&state, move |project| build(project, report, slot, &rows)).await
}
pub(crate) fn build(
    project: &mut Project,
    report: Id,
    slot: DialogueSlotId,
    rows: &[String],
) -> CommandResult<wobu_narrative_build::Build> {
    let saved = project.narrative_analysis_report(report)?;
    saved.policies.check_current(project)?;
    let snapshot = project.narrative_dependency_snapshot()?;
    if variants::hash(&(&snapshot.world, snapshot.schema.iter().collect::<Vec<_>>()))
        != saved.world_guard
    {
        return Err(invalid("State/world model changed since this matrix. Replan."));
    }
    let policy = saved
        .policies
        .policies
        .iter()
        .find(|p| p.target == saved.report.target)
        .ok_or_else(|| invalid("Missing saved policy."))?;
    if rows.is_empty() || rows.len() > policy.limits.variants {
        return Err(invalid("Select rows within the configured variant limit."));
    }
    let file = project.load_scene(policy.target.scene)?;
    let source_slot = file
        .scene
        .beat(policy.target.beat)
        .and_then(|b| b.dialogue.iter().find(|s| s.id == slot))
        .ok_or_else(|| invalid("Matrix slot is missing."))?;
    let mut states = std::collections::BTreeMap::new();
    for id in rows {
        let row = saved
            .report
            .rows
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| invalid("Unknown saved matrix row."))?;
        variants::verify_witness(
            variants::Input {
                scenes: &snapshot.scenes,
                schema: &snapshot.schema,
                world: &snapshot.world,
            },
            policy,
            row,
        )
        .map_err(invalid)?;
        let id = variants::candidate_id(&policy.target, slot, row);
        if !source_slot.variants.iter().any(|v| v.id == id && v.when.as_ref() == Some(&row.when)) {
            return Err(invalid(
                "Materialize the exact selected configurations before freezing generation.",
            ));
        }
        if states.insert(id, row.witness.as_ref().unwrap().state.clone()).is_some() {
            return Err(invalid("Duplicate selected configuration."));
        }
    }
    snapshot.check_current(project)?;
    saved.policies.check_current(project)?;
    let selection = crate::enhance::selection(&project.meta().providers);
    let model = crate::enhance::planning_model(&selection)?;
    let result = super::narrative_build::plan::build_with_matrix(
        project,
        wobu_narrative_build::Input {
            scope: wobu_narrative_build::Scope::AllSelected,
            containers: std::collections::BTreeSet::from([policy.target.scene]),
            state: Default::default(),
            commands: Default::default(),
            token_budget: 4000,
            max_output_tokens: 512,
        },
        &selection.provider,
        &model,
        Some((report, states)),
    )?;
    super::narrative_preview::bridge_integers(&result)?;
    Ok(result)
}
#[cfg(test)]
mod tests;
