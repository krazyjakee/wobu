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

/// One choice at the branch a run is stopped at, and the evaluation of its gate.
///
/// [`Yield::Choices`] lists what the player may pick and deliberately nothing
/// else: a game has no business being told which options it was refused, and a
/// list of them would be a spoiler the runtime handed out. An authoring overlay
/// is the other case — "why can I not see *Show logbook*" is the question #188
/// exists to answer — so this is a separate, opt-in read rather than a wider
/// `Yield`.
///
/// It reports through [`TraceRecord`] rather than through a summary of its own,
/// so the reason a branch is closed is spelled in exactly the vocabulary the
/// played route already uses: the same sites, the same authored [`Condition`],
/// the same evaluated inputs. A second shape here would be a second evaluator's
/// worth of ways to disagree with `evaluate_recording`, and the caller would
/// have no way to tell which of the two was right.
///
/// Recomputed on demand instead of pushed into [`ExecutionTrace`], because a
/// restored snapshot has no trace at all — [`Runtime::restore`] starts an empty
/// one. The checkpoint a writer went back to was evaluated in a session that
/// has since ended, and an overlay that searched the history for the last
/// branch that *looked* like this one would be guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceStatus {
    pub id: String,
    pub label: String,
    /// True when the gate passed. False is the branch an overlay draws closed.
    pub available: bool,
    /// The gate's evaluation, in the order the runtime evaluated it. Never
    /// empty: an absent condition is evaluated as `Always` and recorded, so
    /// "always available" is a stated fact rather than an absence to interpret.
    pub records: Vec<TraceRecord>,
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

    /// Every choice at the branch this run is stopped at, taken or not.
    ///
    /// Empty away from a branch, and that is the whole rule: only a run that is
    /// *offering* choices has an availability to report, and answering for a
    /// beat the cursor has not reached yet would be evaluating gates against
    /// state the effects on the way there have not written.
    ///
    /// Evaluation cannot fail here in practice — `Phase::Branch` is only ever
    /// entered after every gate on the beat has already evaluated cleanly — but
    /// the error is propagated rather than swallowed, because a caller that saw
    /// a silently shortened list would read it as "this choice does not exist".
    pub fn branch(&self) -> Result<Vec<ChoiceStatus>> {
        if !matches!(self.saved.phase, Phase::Branch) {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for choice in &self.beat()?.choices {
            let site = TraceSite { choice: Some(choice.id.clone()), ..self.site() };
            let mut records = Vec::new();
            let condition = choice.requires.as_ref().unwrap_or(&Condition::Always);
            let available =
                evaluate_recording(condition, &self.saved.state, &mut Vec::new(), &mut |event| {
                    records.push(TraceRecord { site: site.clone(), event });
                })?;
            out.push(ChoiceStatus {
                id: choice.id.clone(),
                label: choice.label.clone(),
                available,
                records,
            });
        }
        Ok(out)
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
