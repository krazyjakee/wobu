//! Bounded deterministic execution of compiled narrative graphs. No IO or AI.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use wobu_narrative::{CompareOp, Condition, Effect, Name, Operand, Owner, Speaker, Value};
use wobu_narrative_compiler::{CompiledBeat, GRAPH_VERSION, Graph, Target, accepts};

pub type State = BTreeMap<Name, Value>;
pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Yield {
    Line {
        scene: String,
        beat: String,
        slot: String,
        variant: String,
        speaker: Speaker,
        text: String,
        revision: String,
    },
    Choices {
        scene: String,
        beat: String,
        choices: Vec<AvailableChoice>,
    },
    GameCommand {
        token: String,
        name: Name,
        args: Vec<Value>,
    },
    End {
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailableChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandResult {
    Success { host_inputs: State },
    Failed { message: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    #[error("incompatible graph or snapshot version/content")]
    Incompatible,
    #[error("invalid runtime state: {0}")]
    InvalidState(String),
    #[error("missing state variable {0}")]
    MissingState(String),
    #[error("no matching variant or transition at {0}")]
    NoMatch(String),
    #[error("scene entry condition failed for {0}")]
    EntryDenied(String),
    #[error("invalid action at this yield")]
    InvalidAction,
    #[error("choice {0} is unavailable")]
    UnavailableChoice(String),
    #[error("arithmetic overflow or declared range exceeded for {0}")]
    Overflow(String),
    #[error("automatic execution exceeded {0} steps")]
    StepLimit(u32),
    #[error("command token is not pending")]
    InvalidCommand,
    #[error("host command failed: {0}")]
    CommandFailed(String),
    #[error("host command was cancelled")]
    CommandCancelled,
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolvedCommand {
    token: String,
    name: Name,
    args: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Phase {
    Dialogue { index: usize, variant: String },
    Branch,
    Commands { commands: Vec<ResolvedCommand>, index: usize, to: Target },
    End { label: String },
}

/// A save belongs to one graph and one caller-assigned playthrough ID.
/// Private fields prevent constructing unchecked snapshots through the Rust API;
/// restore also validates deserialized saves before they can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    version: u32,
    graph_version: u32,
    graph_hash: String,
    run_id: String,
    seed: u64,
    scene: String,
    beat: String,
    phase: Phase,
    state: State,
    visits: BTreeMap<String, u64>,
    command_sequence: u64,
    acknowledged: BTreeMap<String, State>,
    step_limit: u32,
}

enum Action {
    Target(Target),
    Dialogue(usize),
    Transition(Vec<Effect>, Target),
}

#[derive(Debug, Clone)]
pub struct Runtime {
    graph: Graph,
    saved: Snapshot,
}

impl Runtime {
    /// The host must assign a distinct run_id for independent playthroughs and
    /// retain it across saves. Seed is reserved in v1: there is no random policy.
    pub fn start(
        graph: Graph,
        scene: &str,
        host_inputs: State,
        run_id: String,
        seed: u64,
        step_limit: u32,
    ) -> Result<Self> {
        for (name, value) in &host_inputs {
            let decl =
                graph.state.get(name).ok_or_else(|| Error::InvalidState(name.to_string()))?;
            if decl.owner != Owner::Host || !accepts(&decl.ty, value) {
                return Err(Error::InvalidState(name.to_string()));
            }
        }
        Self::start_with_state(graph, scene, host_inputs, run_id, seed, step_limit)
    }

    /// Scenario initialization may override narrative defaults as well as host
    /// inputs. Ownership restrictions apply to every subsequent write. This
    /// never edits graph defaults or changes the content hash.
    pub fn start_with_state(
        graph: Graph,
        scene: &str,
        initial: State,
        run_id: String,
        seed: u64,
        step_limit: u32,
    ) -> Result<Self> {
        if graph.version != GRAPH_VERSION {
            return Err(Error::Incompatible);
        }
        if run_id.is_empty() || step_limit == 0 {
            return Err(Error::InvalidState("run_id and positive step limit are required".into()));
        }
        let state = graph
            .state
            .iter()
            .filter(|(_, d)| d.owner == Owner::Narrative)
            .map(|(n, d)| (n.clone(), d.default.clone()))
            .collect();
        let mut runner = Self {
            saved: Snapshot {
                version: SNAPSHOT_VERSION,
                graph_version: graph.version,
                graph_hash: graph.hash(),
                run_id,
                seed,
                scene: scene.into(),
                beat: String::new(),
                phase: Phase::Branch,
                state,
                visits: BTreeMap::new(),
                command_sequence: 0,
                acknowledged: BTreeMap::new(),
                step_limit,
            },
            graph,
        };
        runner.saved.state.extend(initial);
        runner.validate_state()?;
        let mut budget = step_limit;
        runner.drive(Action::Target(Target::Scene(scene.into())), &mut budget)?;
        Ok(runner)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.saved.clone()
    }
    pub fn state(&self) -> &State {
        &self.saved.state
    }
    pub fn visits(&self) -> &BTreeMap<String, u64> {
        &self.saved.visits
    }

    pub fn restore(graph: Graph, snapshot: Snapshot) -> Result<Self> {
        if graph.version != GRAPH_VERSION
            || snapshot.version != SNAPSHOT_VERSION
            || snapshot.graph_version != graph.version
            || snapshot.graph_hash != graph.hash()
        {
            return Err(Error::Incompatible);
        }
        let runner = Self { graph, saved: snapshot };
        if runner.saved.run_id.is_empty() || runner.saved.step_limit == 0 {
            return Err(Error::InvalidState("invalid execution settings".into()));
        }
        runner.validate_state()?;
        runner.beat()?;
        for (id, count) in &runner.saved.visits {
            if *count == 0 || !runner.graph.scenes.values().any(|s| s.beats.contains_key(id)) {
                return Err(Error::InvalidState("invalid visit history".into()));
            }
        }
        if let Phase::Commands { commands, index, to } = &runner.saved.phase {
            if commands.is_empty() || *index >= commands.len() {
                return Err(Error::InvalidCommand);
            }
            runner.validate_target(to)?;
            let mut tokens = std::collections::BTreeSet::new();
            for command in commands {
                if !tokens.insert(&command.token) || !runner.valid_token(&command.token) {
                    return Err(Error::InvalidCommand);
                }
                let types =
                    runner.graph.commands.get(&command.name).ok_or(Error::InvalidCommand)?;
                if types.len() != command.args.len()
                    || !types.iter().zip(&command.args).all(|(ty, value)| accepts(ty, value))
                {
                    return Err(Error::InvalidCommand);
                }
            }
            for (position, command) in commands.iter().enumerate() {
                if runner.saved.acknowledged.contains_key(&command.token) != (position < *index) {
                    return Err(Error::InvalidCommand);
                }
            }
        }
        for (token, inputs) in &runner.saved.acknowledged {
            if !runner.valid_token(token) {
                return Err(Error::InvalidCommand);
            }
            runner.validate_host_inputs(inputs)?;
        }
        runner.current()?;
        Ok(runner)
    }

    /// Repeated reads never advance or select a different line.
    pub fn current(&self) -> Result<Yield> {
        match &self.saved.phase {
            Phase::Dialogue { index, variant } => {
                let slot = self.beat()?.dialogue.get(*index).ok_or(Error::InvalidAction)?;
                let text =
                    slot.variants.iter().find(|v| &v.id == variant).ok_or(Error::InvalidAction)?;
                Ok(Yield::Line {
                    scene: self.saved.scene.clone(),
                    beat: self.saved.beat.clone(),
                    slot: slot.id.clone(),
                    variant: text.id.clone(),
                    speaker: slot.speaker.clone(),
                    text: text.text.clone(),
                    revision: text.revision.clone(),
                })
            }
            Phase::Branch => {
                let choices = self.available_choices()?;
                if choices.is_empty() {
                    return Err(Error::NoMatch(self.saved.beat.clone()));
                }
                Ok(Yield::Choices {
                    scene: self.saved.scene.clone(),
                    beat: self.saved.beat.clone(),
                    choices,
                })
            }
            Phase::Commands { commands, index, .. } => {
                let command = commands.get(*index).ok_or(Error::InvalidCommand)?;
                Ok(Yield::GameCommand {
                    token: command.token.clone(),
                    name: command.name.clone(),
                    args: command.args.clone(),
                })
            }
            Phase::End { label } => Ok(Yield::End { label: label.clone() }),
        }
    }

    /// Failed actions leave cursor, variables, acknowledgements and visits unchanged.
    fn transaction(&mut self, action: impl FnOnce(&mut Self) -> Result<()>) -> Result<Yield> {
        let mut next = self.clone();
        action(&mut next)?;
        let yielded = next.current()?;
        *self = next;
        Ok(yielded)
    }

    pub fn advance(&mut self) -> Result<Yield> {
        self.transaction(|next| match next.saved.phase {
            Phase::Dialogue { index, .. } => {
                let mut budget = next.saved.step_limit;
                next.drive(Action::Dialogue(index + 1), &mut budget)
            }
            Phase::End { .. } => Ok(()),
            _ => Err(Error::InvalidAction),
        })
    }

    pub fn choose(&mut self, choice_id: &str) -> Result<Yield> {
        self.transaction(|next| {
            if next.saved.phase != Phase::Branch {
                return Err(Error::InvalidAction);
            }
            let choice = next
                .beat()?
                .choices
                .iter()
                .find(|c| c.id == choice_id)
                .cloned()
                .ok_or_else(|| Error::UnavailableChoice(choice_id.into()))?;
            if !next.matches(choice.requires.as_ref())? {
                return Err(Error::UnavailableChoice(choice_id.into()));
            }
            let mut budget = next.saved.step_limit;
            next.drive(Action::Transition(choice.effects, choice.to), &mut budget)
        })
    }

    /// The same successful token/result is safe to acknowledge again after restore.
    /// A changed result for an acknowledged token is rejected.
    pub fn complete_command(&mut self, token: &str, result: CommandResult) -> Result<Yield> {
        self.transaction(|next| {
            if let Some(previous) = next.saved.acknowledged.get(token) {
                return if matches!(&result, CommandResult::Success { host_inputs } if host_inputs == previous) { Ok(()) } else { Err(Error::InvalidCommand) };
            }
            let Phase::Commands { commands, index, to } = next.saved.phase.clone() else { return Err(Error::InvalidCommand); };
            if commands[index].token != token { return Err(Error::InvalidCommand); }
            let inputs = match result {
                CommandResult::Success { host_inputs } => host_inputs,
                CommandResult::Failed { message } => return Err(Error::CommandFailed(message)),
                CommandResult::Cancelled => return Err(Error::CommandCancelled),
            };
            next.apply_host_inputs(&inputs)?;
            next.saved.acknowledged.insert(token.into(), inputs);
            if index + 1 < commands.len() {
                next.saved.phase = Phase::Commands { commands, index: index + 1, to };
                Ok(())
            } else {
                let mut budget = next.saved.step_limit;
                next.drive(Action::Target(to), &mut budget)
            }
        })
    }

    pub fn update_host_inputs(&mut self, inputs: State) -> Result<Yield> {
        self.transaction(|next| next.apply_host_inputs(&inputs))
    }

    fn validate_host_inputs(&self, inputs: &State) -> Result<()> {
        for (name, value) in inputs {
            let decl =
                self.graph.state.get(name).ok_or_else(|| Error::InvalidState(name.to_string()))?;
            if decl.owner != Owner::Host || !accepts(&decl.ty, value) {
                return Err(Error::InvalidState(name.to_string()));
            }
        }
        Ok(())
    }

    fn apply_host_inputs(&mut self, inputs: &State) -> Result<()> {
        self.validate_host_inputs(inputs)?;
        self.saved.state.extend(inputs.clone());
        Ok(())
    }

    fn validate_state(&self) -> Result<()> {
        for (name, decl) in &self.graph.state {
            let value =
                self.saved.state.get(name).ok_or_else(|| Error::MissingState(name.to_string()))?;
            if !accepts(&decl.ty, value) {
                return Err(Error::InvalidState(name.to_string()));
            }
        }
        if self.saved.state.len() != self.graph.state.len() {
            return Err(Error::InvalidState("undeclared variables".into()));
        }
        Ok(())
    }

    fn beat(&self) -> Result<&CompiledBeat> {
        self.graph
            .scenes
            .get(&self.saved.scene)
            .and_then(|s| s.beats.get(&self.saved.beat))
            .ok_or_else(|| Error::InvalidState("missing scene or beat".into()))
    }

    fn matches(&self, condition: Option<&Condition>) -> Result<bool> {
        condition.map(|c| evaluate(c, &self.saved.state)).unwrap_or(Ok(true))
    }

    fn available_choices(&self) -> Result<Vec<AvailableChoice>> {
        let mut out = Vec::new();
        for choice in &self.beat()?.choices {
            if self.matches(choice.requires.as_ref())? {
                out.push(AvailableChoice { id: choice.id.clone(), label: choice.label.clone() });
            }
        }
        Ok(out)
    }

    fn consume(&self, budget: &mut u32) -> Result<()> {
        if *budget == 0 {
            return Err(Error::StepLimit(self.saved.step_limit));
        }
        *budget -= 1;
        Ok(())
    }

    // Explicit trampoline: cyclic authored graphs consume budget without
    // growing the native stack, even when the caller chooses a large budget.
    fn drive(&mut self, mut action: Action, budget: &mut u32) -> Result<()> {
        loop {
            self.consume(budget)?;
            action = match action {
                Action::Target(Target::End { label }) => {
                    self.saved.phase = Phase::End { label };
                    return Ok(());
                }
                Action::Target(Target::Scene(id)) => {
                    let scene = self
                        .graph
                        .scenes
                        .get(&id)
                        .ok_or_else(|| Error::InvalidState(format!("missing scene {id}")))?;
                    if !self.matches(scene.entry.as_ref())? {
                        return Err(Error::EntryDenied(id));
                    }
                    let first = scene.first.clone();
                    self.saved.scene = id;
                    Action::Target(Target::Beat(first))
                }
                Action::Target(Target::Beat(id)) => {
                    self.saved.beat = id.clone();
                    self.beat()?;
                    let count = self.saved.visits.entry(id).or_default();
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| Error::InvalidState("visit counter overflow".into()))?;
                    Action::Dialogue(0)
                }
                Action::Dialogue(index) => {
                    if let Some(slot) = self.beat()?.dialogue.get(index) {
                        let mut selected = None;
                        for variant in &slot.variants {
                            if self.matches(variant.when.as_ref())? {
                                selected = Some(variant.id.clone());
                                break;
                            }
                        }
                        let variant = selected.ok_or_else(|| Error::NoMatch(slot.id.clone()))?;
                        self.saved.phase = Phase::Dialogue { index, variant };
                        return Ok(());
                    }
                    if !self.available_choices()?.is_empty() {
                        self.saved.phase = Phase::Branch;
                        return Ok(());
                    }
                    let mut selected = None;
                    for outcome in &self.beat()?.outcomes {
                        if self.matches(outcome.when.as_ref())? {
                            selected = Some(outcome.clone());
                            break;
                        }
                    }
                    let outcome =
                        selected.ok_or_else(|| Error::NoMatch(self.saved.beat.clone()))?;
                    Action::Transition(outcome.effects, outcome.to)
                }
                Action::Transition(effects, to) => {
                    if self.prepare_transition(&effects, to.clone())? {
                        return Ok(());
                    }
                    Action::Target(to)
                }
            };
        }
    }

    fn validate_target(&self, target: &Target) -> Result<()> {
        let exists = match target {
            Target::Beat(id) => {
                self.graph.scenes.get(&self.saved.scene).is_some_and(|s| s.beats.contains_key(id))
            }
            Target::Scene(id) => self.graph.scenes.contains_key(id),
            Target::End { .. } => true,
        };
        if exists { Ok(()) } else { Err(Error::InvalidState("missing destination".into())) }
    }

    /// True means commands are pending; false means the driver can follow to.
    fn prepare_transition(&mut self, effects: &[Effect], to: Target) -> Result<bool> {
        self.validate_target(&to)?;
        let mut state = self.saved.state.clone();
        let mut commands = Vec::new();
        let mut sequence = self.saved.command_sequence;
        for effect in effects {
            match effect {
                Effect::Set(a) => {
                    let value = operand(&a.value, &state)?;
                    self.write(&mut state, &a.var, value)?;
                }
                Effect::Add(a) => {
                    let Value::Int(before) =
                        state.get(&a.var).ok_or_else(|| Error::MissingState(a.var.to_string()))?
                    else {
                        return Err(Error::InvalidState(a.var.to_string()));
                    };
                    let after = before
                        .checked_add(a.by)
                        .ok_or_else(|| Error::Overflow(a.var.to_string()))?;
                    self.write(&mut state, &a.var, Value::Int(after))?;
                }
                Effect::Command(c) => {
                    let args: Vec<_> =
                        c.args.iter().map(|arg| operand(arg, &state)).collect::<Result<_>>()?;
                    let types = self.graph.commands.get(&c.name).ok_or(Error::InvalidCommand)?;
                    if args.len() != types.len()
                        || !types.iter().zip(&args).all(|(ty, value)| accepts(ty, value))
                    {
                        return Err(Error::InvalidCommand);
                    }
                    sequence = sequence.checked_add(1).ok_or(Error::InvalidCommand)?;
                    commands.push(ResolvedCommand {
                        token: format!(
                            "{}:{}:{sequence}",
                            self.saved.run_id, self.saved.graph_hash
                        ),
                        name: c.name.clone(),
                        args,
                    });
                }
            }
        }
        self.saved.state = state;
        self.saved.command_sequence = sequence;
        if commands.is_empty() {
            Ok(false)
        } else {
            self.saved.phase = Phase::Commands { commands, index: 0, to };
            Ok(true)
        }
    }

    fn valid_token(&self, token: &str) -> bool {
        token
            .strip_prefix(&format!("{}:{}:", self.saved.run_id, self.saved.graph_hash))
            .and_then(|n| n.parse::<u64>().ok())
            .is_some_and(|n| n > 0 && n <= self.saved.command_sequence)
    }

    fn write(&self, state: &mut State, name: &Name, value: Value) -> Result<()> {
        let decl =
            self.graph.state.get(name).ok_or_else(|| Error::MissingState(name.to_string()))?;
        if decl.owner != Owner::Narrative {
            return Err(Error::InvalidState(format!("{name} is host-owned")));
        }
        if !accepts(&decl.ty, &value) {
            return Err(if matches!(value, Value::Int(_)) {
                Error::Overflow(name.to_string())
            } else {
                Error::InvalidState(name.to_string())
            });
        }
        state.insert(name.clone(), value);
        Ok(())
    }
}

fn operand(value: &Operand, state: &State) -> Result<Value> {
    match value {
        Operand::Literal(value) => Ok(value.clone()),
        Operand::Var(name) => {
            state.get(name).cloned().ok_or_else(|| Error::MissingState(name.to_string()))
        }
    }
}

/// Evaluation is deterministic and short-circuits left-to-right. The compiler
/// has already checked operand types; this also rejects invalid mixed values.
pub fn evaluate(condition: &Condition, state: &State) -> Result<bool> {
    match condition {
        Condition::Always => Ok(true),
        Condition::Never => Ok(false),
        Condition::Not(inner) => Ok(!evaluate(inner, state)?),
        Condition::All(items) => {
            for item in items {
                if !evaluate(item, state)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Condition::Any(items) => {
            for item in items {
                if evaluate(item, state)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Condition::Compare(cmp) => {
            let left =
                state.get(&cmp.var).ok_or_else(|| Error::MissingState(cmp.var.to_string()))?;
            let right = operand(&cmp.value, state)?;
            if left.kind_name() != right.kind_name() {
                return Err(Error::InvalidState(cmp.var.to_string()));
            }
            match cmp.op {
                CompareOp::Eq => Ok(left == &right),
                CompareOp::Ne => Ok(left != &right),
                op => {
                    let (Value::Int(a), Value::Int(b)) = (left, right) else {
                        return Err(Error::InvalidState(cmp.var.to_string()));
                    };
                    Ok(match op {
                        CompareOp::Lt => *a < b,
                        CompareOp::Le => *a <= b,
                        CompareOp::Gt => *a > b,
                        CompareOp::Ge => *a >= b,
                        _ => unreachable!(),
                    })
                }
            }
        }
    }
}
