//! Portable regression tapes and a pure offline runner. No filesystem, provider or host side effects.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wobu_narrative::{Name, SceneId, Value, VarType};
use wobu_narrative_runtime::{CommandResult, State};

mod runner;
pub use runner::{Divergence, RunReport, run};
pub const SCENARIO_VERSION: u32 = 1;
pub const MAX_STEPS: usize = 1000;

/// Stored as the payload of a canonical named scenario record. Its version evolves
/// independently of the storage envelope, which owns the name and stable record ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub version: u32,
    pub scene: SceneId,
    pub initial_state: State,
    pub seed: u64,
    pub commands: BTreeMap<Name, Vec<VarType>>,
    pub steps: Vec<Step>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: Option<Action>,
    pub expect: Assertion,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Advance,
    Choose {
        choice: String,
    },
    /// Resolve the current pending token on each replay; never persist run-specific tokens.
    CompleteCommand {
        result: CommandResult,
    },
    SaveCheckpoint,
    RestoreCheckpoint,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    /// Omitted fields are unconstrained. Supplied state is a subset, never a state update.
    pub boundary: Option<Boundary>,
    #[serde(default)]
    pub state: State,
    pub error: Option<ExpectedError>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Boundary {
    Line {
        scene: Option<String>,
        beat: Option<String>,
        slot: Option<String>,
        variant: Option<String>,
    },
    Choices {
        scene: Option<String>,
        beat: Option<String>,
        ids: Option<Vec<String>>,
    },
    Command {
        name: Option<Name>,
        args: Option<Vec<Value>>,
    },
    End {
        scene: Option<String>,
        beat: Option<String>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedError {
    CommandFailed,
    CommandCancelled,
}
#[derive(Debug, thiserror::Error)]
#[error("invalid scenario: {0}")]
pub struct Invalid(pub String);
impl Scenario {
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.version != SCENARIO_VERSION {
            return Err(Invalid(format!("unsupported payload version {}", self.version)));
        }
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return Err(Invalid(format!("expected 1..={MAX_STEPS} steps")));
        }
        for (index, step) in self.steps.iter().enumerate() {
            if step.action.is_none() != (index == 0) {
                return Err(Invalid(format!(
                    "step {index}: only the initial boundary has no action"
                )));
            }
            if index == 0 && step.expect.error.is_some() {
                return Err(Invalid("initial boundary cannot expect a command error".into()));
            }
            if let Some(error) = step.expect.error {
                let matching = matches!(
                    (&step.action, error),
                    (
                        Some(Action::CompleteCommand { result: CommandResult::Failed { .. } }),
                        ExpectedError::CommandFailed
                    ) | (
                        Some(Action::CompleteCommand { result: CommandResult::Cancelled }),
                        ExpectedError::CommandCancelled
                    )
                );
                if !matching {
                    return Err(Invalid(format!(
                        "step {index}: expected command error does not match its action"
                    )));
                }
            }
        }
        Ok(())
    }
}
