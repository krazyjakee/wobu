//! Facts observed during execution, rather than reconstructed from UI history.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSite {
    pub scene: String,
    pub beat: Option<String>,
    pub choice: Option<String>,
    pub outcome: Option<String>,
    pub slot: Option<String>,
    pub variant: Option<String>,
    /// The supporting text asset a decision belongs to (#167), when it is not a
    /// scene decision. Skipped when absent so an existing trace serializes to
    /// the bytes it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TraceEvent {
    Condition {
        path: Vec<usize>,
        expression: Condition,
        passed: bool,
        inputs: State,
    },
    Transition {
        to: Target,
    },
    Effect {
        index: usize,
        effect: Effect,
        before: State,
        after: State,
    },
    CommandResult {
        token: String,
        result: CommandResult,
        before: State,
        after: State,
        repeated: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceRecord {
    pub site: TraceSite,
    pub event: TraceEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTrace {
    /// False means this action failed and every tentative write was rolled back.
    pub committed: bool,
    pub error: Option<String>,
    pub records: Vec<TraceRecord>,
    pub omitted: u64,
}
impl Default for ExecutionTrace {
    fn default() -> Self {
        Self { committed: true, error: None, records: Vec::new(), omitted: 0 }
    }
}

impl ExecutionTrace {
    fn push(&mut self, record: TraceRecord) {
        if self.records.len() < 2048 {
            self.records.push(record);
        } else {
            self.omitted += 1;
        }
    }
}

impl Runtime {
    pub fn trace(&self) -> &ExecutionTrace {
        &self.trace
    }

    /// Stable current cursor, including at command and End boundaries.
    pub fn site(&self) -> TraceSite {
        TraceSite {
            scene: self.saved.scene.clone(),
            beat: (!self.saved.beat.is_empty()).then(|| self.saved.beat.clone()),
            ..TraceSite::default()
        }
    }

    pub(super) fn record(&mut self, site: TraceSite, event: TraceEvent) {
        self.trace.push(TraceRecord { site, event });
    }

    pub(super) fn matches_at(
        &mut self,
        condition: Option<&Condition>,
        site: TraceSite,
    ) -> Result<bool> {
        let condition = condition.unwrap_or(&Condition::Always);
        let trace = &mut self.trace;
        evaluate_recording(condition, &self.saved.state, &mut Vec::new(), &mut |event| {
            trace.push(TraceRecord { site: site.clone(), event });
        })
    }
}

pub(super) fn evaluate_recording(
    condition: &Condition,
    state: &State,
    path: &mut Vec<usize>,
    record: &mut impl FnMut(TraceEvent),
) -> Result<bool> {
    let mut child = |index, inner: &Condition| {
        path.push(index);
        let result = evaluate_recording(inner, state, path, record);
        path.pop();
        result
    };
    let result = match condition {
        Condition::Always => true,
        Condition::Never => false,
        Condition::Not(inner) => !child(0, inner)?,
        Condition::All(items) => {
            let mut passed = true;
            for (index, item) in items.iter().enumerate() {
                if !child(index, item)? {
                    passed = false;
                    break;
                }
            }
            passed
        }
        Condition::Any(items) => {
            let mut passed = false;
            for (index, item) in items.iter().enumerate() {
                if child(index, item)? {
                    passed = true;
                    break;
                }
            }
            passed
        }
        Condition::Compare(cmp) => {
            let left =
                state.get(&cmp.var).ok_or_else(|| Error::MissingState(cmp.var.to_string()))?;
            let right = operand(&cmp.value, state)?;
            if left.kind_name() != right.kind_name() {
                return Err(Error::InvalidState(cmp.var.to_string()));
            }
            match cmp.op {
                CompareOp::Eq => left == &right,
                CompareOp::Ne => left != &right,
                op => {
                    let (Value::Int(a), Value::Int(b)) = (left, right) else {
                        return Err(Error::InvalidState(cmp.var.to_string()));
                    };
                    match op {
                        CompareOp::Lt => *a < b,
                        CompareOp::Le => *a <= b,
                        CompareOp::Gt => *a > b,
                        CompareOp::Ge => *a >= b,
                        _ => unreachable!(),
                    }
                }
            }
        }
    };
    // Composite records report their result, while only comparison leaves claim
    // reads. Short-circuited siblings produce no evaluation record at all.
    let inputs = if matches!(condition, Condition::Compare(_)) {
        condition
            .variables()
            .into_iter()
            .filter_map(|name| state.get(name).map(|value| (name.clone(), value.clone())))
            .collect()
    } else {
        State::new()
    };
    record(TraceEvent::Condition {
        path: path.clone(),
        expression: condition.clone(),
        passed: result,
        inputs,
    });
    Ok(result)
}

/// Effects report only the variables they read/write, not a copy of the entire
/// world for each effect. Literal command arguments remain in the effect itself.
pub(super) fn effect_values(state: &State, effect: &Effect) -> State {
    let mut names = Vec::new();
    if let Some(name) = effect.writes() {
        names.push(name);
    }
    match effect {
        Effect::Set(a) => {
            if let Operand::Var(name) = &a.value {
                names.push(name);
            }
        }
        Effect::Command(c) => {
            for arg in &c.args {
                if let Operand::Var(name) = arg {
                    names.push(name);
                }
            }
        }
        Effect::Add(_) => {}
    }
    names
        .into_iter()
        .filter_map(|name| state.get(name).map(|value| (name.clone(), value.clone())))
        .collect()
}
