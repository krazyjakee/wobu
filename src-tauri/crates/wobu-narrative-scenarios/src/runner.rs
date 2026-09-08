use super::*;
use serde_json::{Value as Json, json};
use wobu_narrative_compiler::Graph;
use wobu_narrative_runtime::{
    Error, ExecutionTrace, Runtime, Snapshot, TraceEvent, TraceSite, Yield,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Divergence {
    /// Zero-based boundary index; 0 is the initial yield.
    pub step: usize,
    pub field: String,
    pub expected: Json,
    pub actual: Json,
    pub site: TraceSite,
    pub trace: ExecutionTrace,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub passed: bool,
    pub checked_steps: usize,
    pub divergence: Option<Divergence>,
}

/// Replays against current compiled content. Assertions deliberately do not pin a
/// content hash: a wording-only edit must not turn into a regression failure.
pub fn run(graph: &Graph, scenario: &Scenario) -> Result<RunReport, Invalid> {
    scenario.validate()?;
    if graph.commands != scenario.commands {
        return Err(Invalid("compiled command registry differs from scenario".into()));
    }
    let mut runtime = match Runtime::start_with_state(
        graph.clone(),
        &scenario.scene.to_string(),
        scenario.initial_state.clone(),
        "scenario".into(),
        scenario.seed,
        1000,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            return Ok(failure(
                0,
                "start",
                json!("initial yield"),
                json!(error.to_string()),
                TraceSite { scene: scenario.scene.to_string(), ..TraceSite::default() },
                ExecutionTrace {
                    committed: false,
                    error: Some(error.to_string()),
                    ..ExecutionTrace::default()
                },
            ));
        }
    };
    let mut checkpoint: Option<Snapshot> = None;
    for (index, step) in scenario.steps.iter().enumerate() {
        let no_command = matches!(step.action, Some(Action::CompleteCommand { .. }))
            && !matches!(runtime.current(), Ok(Yield::GameCommand { .. }));
        let result = match &step.action {
            None => Ok(()),
            Some(Action::Advance) => runtime.advance().map(|_| ()),
            Some(Action::Choose { choice }) => runtime.choose(choice).map(|_| ()),
            Some(Action::CompleteCommand { result }) => match runtime.current() {
                Ok(Yield::GameCommand { token, .. }) => {
                    runtime.complete_command(&token, result.clone()).map(|_| ())
                }
                _ => Err(Error::InvalidCommand),
            },
            Some(Action::SaveCheckpoint) => {
                checkpoint = Some(runtime.snapshot());
                Ok(())
            }
            Some(Action::RestoreCheckpoint) => match &checkpoint {
                None => Err(Error::InvalidState("no scenario checkpoint has been saved".into())),
                Some(saved) => Runtime::restore(graph.clone(), saved.clone()).map(|restored| {
                    runtime = restored;
                }),
            },
        };
        let mut trace = if no_command
            || matches!(step.action, Some(Action::SaveCheckpoint | Action::RestoreCheckpoint))
        {
            ExecutionTrace::default()
        } else {
            runtime.trace().clone()
        };
        if let Err(error) = &result {
            trace.committed = false;
            trace.error = Some(error.to_string());
        }
        let actual_error = match &result {
            Err(Error::CommandFailed(_)) => Some(ExpectedError::CommandFailed),
            Err(Error::CommandCancelled) => Some(ExpectedError::CommandCancelled),
            _ => None,
        };
        if actual_error != step.expect.error || (result.is_err() && actual_error.is_none()) {
            return Ok(failure(
                index,
                "error",
                json!(step.expect.error),
                json!(result.err().map(|e| e.to_string())),
                runtime.site(),
                trace,
            ));
        }
        if let Some((field, expected, actual)) = mismatch(&runtime, &step.expect) {
            let effect = field.strip_prefix("state.").and_then(|name| {
                trace.records.iter().rev().find(|record| match &record.event {
                    TraceEvent::Effect { after, .. } => {
                        after.keys().any(|key| key.as_str() == name)
                    }
                    _ => false,
                })
            });
            let site = effect
                .or_else(|| trace.records.last())
                .map(|record| record.site.clone())
                .unwrap_or_else(|| runtime.site());
            return Ok(failure(index, &field, expected, actual, site, trace));
        }
    }
    Ok(RunReport { passed: true, checked_steps: scenario.steps.len(), divergence: None })
}
fn failure(
    step: usize,
    field: &str,
    expected: Json,
    actual: Json,
    site: TraceSite,
    trace: ExecutionTrace,
) -> RunReport {
    RunReport {
        passed: false,
        checked_steps: step,
        divergence: Some(Divergence { step, field: field.into(), expected, actual, site, trace }),
    }
}
fn check<T: Serialize>(
    field: &str,
    expected: &Option<T>,
    actual: &T,
) -> Option<(String, Json, Json)> {
    expected.as_ref().and_then(|expected| {
        let a = json!(actual);
        let e = json!(expected);
        (a != e).then(|| (field.into(), e, a))
    })
}
fn mismatch(runtime: &Runtime, assertion: &Assertion) -> Option<(String, Json, Json)> {
    let current = runtime.current().ok()?;
    if let Some(boundary) = &assertion.boundary {
        let wrong_kind = || Some(("boundary.kind".into(), json!(boundary), json!(kind(&current))));
        let found = match (boundary, &current) {
            (
                Boundary::Line { scene, beat, slot, variant },
                Yield::Line { scene: s, beat: b, slot: l, variant: v, .. },
            ) => check("line.scene", scene, s)
                .or_else(|| check("line.beat", beat, b))
                .or_else(|| check("line.slot", slot, l))
                .or_else(|| check("line.variant", variant, v)),
            (
                Boundary::Choices { scene, beat, ids },
                Yield::Choices { scene: s, beat: b, choices },
            ) => check("choices.scene", scene, s)
                .or_else(|| check("choices.beat", beat, b))
                .or_else(|| {
                    check(
                        "choices.ids",
                        ids,
                        &choices.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                    )
                }),
            (Boundary::Command { name, args }, Yield::GameCommand { name: n, args: a, .. }) => {
                check("command.name", name, n).or_else(|| check("command.args", args, a))
            }
            (Boundary::End { scene, beat }, Yield::End { .. }) => {
                let site = runtime.site();
                check("end.scene", scene, &site.scene)
                    .or_else(|| check("end.beat", beat, &site.beat.unwrap_or_default()))
            }
            _ => wrong_kind(),
        };
        if found.is_some() {
            return found;
        }
    }
    for (name, expected) in &assertion.state {
        if runtime.state().get(name) != Some(expected) {
            return Some((
                format!("state.{name}"),
                json!(expected),
                json!(runtime.state().get(name)),
            ));
        }
    }
    None
}
fn kind(value: &Yield) -> &'static str {
    match value {
        Yield::Line { .. } => "line",
        Yield::Choices { .. } => "choices",
        Yield::GameCommand { .. } => "command",
        Yield::End { .. } => "end",
    }
}
