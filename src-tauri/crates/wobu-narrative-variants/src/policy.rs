use super::*;
use wobu_narrative::{Effect, EntityId, Operand, Owner};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub states: usize,
    pub variants: usize,
    pub milliseconds: u64,
    /// Total transition references retained across all displayed witness paths.
    #[serde(default = "witness_steps")]
    pub witness_steps: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            states: 10_000,
            variants: 1_000,
            milliseconds: 1_000,
            witness_steps: witness_steps(),
        }
    }
}
fn witness_steps() -> usize {
    20_000
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestBinding {
    pub quest: EntityId,
    pub variable: Name,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBinding {
    pub event: EntityId,
    pub effects: Vec<Effect>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub target: Target,
    pub initial: Vec<State>,
    pub invariants: Vec<Condition>,
    pub quests: Vec<QuestBinding>,
    pub events: Vec<EventBinding>,
    /// False explicitly declares the transition model closed, subject to the
    /// automatic host/authored-effect checks. Absence is conservatively open.
    #[serde(default = "open")]
    pub external: bool,
    pub limits: Limits,
}
fn open() -> bool {
    true
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub source: String,
    pub when: Condition,
    pub effects: Vec<Effect>,
}
impl Policy {
    pub fn validate(&self, input: &Input<'_>) -> Result<Vec<Transition>, Invalid> {
        if self.limits.witness_steps > 100_000
            || self.version != VERSION
            || self.initial.is_empty()
            || self.initial.len() > 1000
            || !(1..=100_000).contains(&self.limits.states)
            || !(1..=10_000).contains(&self.limits.variants)
            || !(1..=10_000).contains(&self.limits.milliseconds)
        {
            return Err(invalid("Unsupported policy version, initial states or limits"));
        }
        let scene = input
            .scenes
            .iter()
            .find(|s| s.id == self.target.scene)
            .ok_or_else(|| invalid("Analysis scene is missing"))?;
        if scene.beat(self.target.beat).is_none() {
            return Err(invalid("Analysis beat is missing"));
        }
        for condition in &self.invariants {
            input.schema.check_condition(condition).map_err(invalid)?;
        }
        for state in &self.initial {
            if state.len() != input.schema.len()
                || input.schema.iter().any(|d| {
                    !state.get(&d.name).is_some_and(|v| wobu_narrative::evaluate::accepts(&d.ty, v))
                })
            {
                return Err(invalid(
                    "Initial states must specify every declared variable with a valid value",
                ));
            }
            if !self.permits(state)? {
                return Err(invalid("Initial state violates a declared invariant"));
            }
        }
        let mut transitions = Vec::new();
        let mut seen = BTreeSet::new();
        for binding in &self.quests {
            if !seen.insert(binding.quest) {
                return Err(invalid("Duplicate quest binding"));
            }
            let quest = input
                .world
                .quests
                .iter()
                .find(|q| q.id == binding.quest)
                .ok_or_else(|| invalid("Bound quest is missing"))?;
            let decl = input
                .schema
                .get(&binding.variable)
                .ok_or_else(|| invalid("Quest state variable is missing"))?;
            if decl.owner != Owner::Narrative
                || !matches!(&decl.ty,VarType::Enum{members} if members==&quest.stages)
                || self
                    .initial
                    .iter()
                    .any(|s| s.get(&binding.variable) != Some(&Value::Enum(quest.initial.clone())))
            {
                return Err(invalid(
                    "Quest binding requires a narrative enum matching its stages and declared initial stage",
                ));
            }
            for (index, t) in quest.transitions.iter().enumerate() {
                if !quest.stages.contains(&t.from) || !quest.stages.contains(&t.to) {
                    return Err(invalid("Quest transition references an unknown stage"));
                }
                let from = super::condition(&BTreeMap::from([(
                    binding.variable.clone(),
                    Value::Enum(t.from.clone()),
                )]));
                transitions.push(Transition {
                    source: format!("world/quests/{}/transitions/{index}", quest.id),
                    when: Condition::All(vec![from, t.when.clone()]),
                    effects: vec![Effect::Set(wobu_narrative::Assignment {
                        var: binding.variable.clone(),
                        value: Operand::Literal(Value::Enum(t.to.clone())),
                    })],
                });
            }
        }
        seen.clear();
        for binding in &self.events {
            if !seen.insert(binding.event) {
                return Err(invalid("Duplicate event binding"));
            }
            let event = input
                .world
                .events
                .iter()
                .find(|e| e.id == binding.event)
                .ok_or_else(|| invalid("Bound event is missing"))?;
            transitions.push(Transition {
                source: format!("world/events/{}", event.id),
                when: event.when.clone(),
                effects: binding.effects.clone(),
            });
        }
        for transition in &transitions {
            input.schema.check_condition(&transition.when).map_err(invalid)?;
            for effect in &transition.effects {
                input.schema.check_effect(effect).map_err(invalid)?;
            }
        }
        Ok(transitions)
    }
    pub fn permits(&self, state: &State) -> Result<bool, Invalid> {
        for invariant in &self.invariants {
            if !wobu_narrative::evaluate::evaluate(invariant, state).map_err(invalid)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
