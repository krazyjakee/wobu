//! Bounded authoring proof under an explicit state-transition model.
//! Witnesses describe this model, never an invented runtime route. The crate
//! depends on source semantics only, keeping compiler/runtime dependencies acyclic.
pub mod example;
mod explore;
mod policy;
mod relevance;
pub use policy::{EventBinding, Limits, Policy, QuestBinding, Transition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub use wobu_narrative::evaluate::State;
use wobu_narrative::{
    BeatId, Condition, DialogueSlotId, Name, Scene, SceneId, StateSchema, Value, VarType,
    VariantId, WorldDocument,
};
pub const VERSION: u32 = 1;
#[derive(Debug, thiserror::Error)]
#[error("Invalid variant analysis: {0}")]
pub struct Invalid(pub String);
pub(crate) fn invalid(error: impl std::fmt::Display) -> Invalid {
    Invalid(error.to_string())
}
pub fn hash(value: &impl Serialize) -> String {
    blake3::hash(&serde_json::to_vec(value).expect("typed analysis values serialize"))
        .to_hex()
        .to_string()
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub scene: SceneId,
    pub beat: BeatId,
}
pub struct Input<'a> {
    pub scenes: &'a [Scene],
    pub schema: &'a StateSchema,
    pub world: &'a WorldDocument,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Domain {
    pub name: Name,
    pub ty: VarType,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Included,
    Excluded,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub state: State,
    /// Ordered source IDs in the explicitly declared model. No runtime path is claimed.
    pub transitions: Vec<String>,
    pub initial: usize,
    pub target: Target,
    pub runtime_route_verified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotCoverage {
    pub slot: DialogueSlotId,
    pub matching: Vec<VariantId>,
    pub selected: Option<VariantId>,
    pub fallback: Option<VariantId>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub values: State,
    pub when: Condition,
    pub classification: Classification,
    pub reason: String,
    pub witness: Option<Witness>,
    pub coverage: Vec<SlotCoverage>,
}
/// Decimal strings preserve cardinalities across JavaScript/i64 boundaries.
/// A saturated value is a lower bound, explicitly marked inexact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Count {
    pub value: String,
    pub exact: bool,
}
impl Count {
    fn of(value: u128, exact: bool) -> Self {
        Self { value: value.to_string(), exact }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u32,
    pub target: Target,
    pub policy_hash: String,
    pub domains: Vec<Domain>,
    pub potential: Count,
    pub included: Count,
    pub excluded: Count,
    pub unknown: Count,
    pub rows: Vec<Row>,
    pub complete: bool,
    pub explored_states: usize,
    pub limits: Limits,
    pub reasons: Vec<String>,
    pub assumptions: Vec<String>,
}
pub fn analyze(input: Input<'_>, policy: &Policy) -> Result<Report, Invalid> {
    explore::analyze(input, policy, || false)
}
/// Deterministic cancellation seam, also used to test elapsed-budget frontiers.
pub fn analyze_with_stop(
    input: Input<'_>,
    policy: &Policy,
    stop: impl FnMut() -> bool,
) -> Result<Report, Invalid> {
    explore::analyze(input, policy, stop)
}
pub fn candidate_id(target: &Target, slot: DialogueSlotId, row: &Row) -> VariantId {
    let bytes = blake3::hash(
        &serde_json::to_vec(&("narrative_configuration/1", target, slot, &row.id))
            .expect("typed identity"),
    );
    let mut raw = [0; 16];
    raw.copy_from_slice(&bytes.as_bytes()[..16]);
    VariantId::from_raw(wobu_narrative::EntityId::from(u128::from_be_bytes(raw)))
}
pub(crate) fn project(state: &State, names: &BTreeSet<Name>) -> State {
    state
        .iter()
        .filter(|(name, _)| names.contains(*name))
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect()
}
pub(crate) fn state_key(state: &State) -> String {
    serde_json::to_string(state).expect("typed state")
}
pub(crate) fn condition(values: &State) -> Condition {
    Condition::All(
        values
            .iter()
            .map(|(name, value)| {
                Condition::Compare(wobu_narrative::Comparison {
                    var: name.clone(),
                    op: wobu_narrative::CompareOp::Eq,
                    value: wobu_narrative::Operand::Literal(value.clone()),
                })
            })
            .collect(),
    )
}
pub(crate) fn coverage(
    scene: &Scene,
    target: &Target,
    state: &State,
    stop: &mut impl FnMut() -> bool,
) -> Result<Option<Vec<SlotCoverage>>, Invalid> {
    let beat = scene.beat(target.beat).ok_or_else(|| invalid("Missing beat"))?;
    let mut result = Vec::new();
    for slot in &beat.dialogue {
        let mut matching = Vec::new();
        for variant in &slot.variants {
            if stop() {
                return Ok(None);
            }
            if wobu_narrative::evaluate::evaluate(
                variant.when.as_ref().unwrap_or(&Condition::Always),
                state,
            )
            .map_err(invalid)?
            {
                matching.push(variant.id);
            }
        }
        result.push(SlotCoverage {
            slot: slot.id,
            selected: matching.first().copied(),
            matching,
            fallback: slot
                .variants
                .iter()
                .find(|v| v.when.is_none() || v.when == Some(Condition::Always))
                .map(|v| v.id),
        });
    }
    Ok(Some(result))
}
/// Replays retained evidence from canonical model sources before structural writes.
/// This verifies a model witness, deliberately not an engine route.
pub fn verify_witness(input: Input<'_>, policy: &Policy, row: &Row) -> Result<(), Invalid> {
    let transitions = policy.validate(&input)?;
    let relevant = relevance::relevant(&input, &policy.target)?;
    let domains: Vec<_> = relevant
        .iter()
        .map(|name| Domain { name: name.clone(), ty: input.schema.get(name).unwrap().ty.clone() })
        .collect();
    let proof = row.witness.as_ref().ok_or_else(|| invalid("Configuration has no witness"))?;
    if row.classification != Classification::Included
        || proof.target != policy.target
        || proof.runtime_route_verified
        || proof.transitions.len() > policy.limits.states
        || row.id != hash(&("narrative_configuration/1", &policy.target, &domains, &row.values))
        || row.when != condition(&row.values)
    {
        return Err(invalid("Invalid configuration identity or witness contract"));
    }
    let mut state =
        policy.initial.get(proof.initial).ok_or_else(|| invalid("Unknown initial state"))?.clone();
    for source in &proof.transitions {
        let transition = transitions
            .iter()
            .find(|t| &t.source == source)
            .ok_or_else(|| invalid("Witness transition source is missing"))?;
        if !wobu_narrative::evaluate::evaluate(&transition.when, &state).map_err(invalid)? {
            return Err(invalid("Witness transition is not enabled"));
        }
        for effect in &transition.effects {
            let (name, value) =
                wobu_narrative::evaluate::assignment(effect, &state).map_err(invalid)?;
            let d = input.schema.get(&name).ok_or_else(|| invalid("Missing witness variable"))?;
            wobu_narrative::evaluate::validate_write(&name, &d.ty, d.owner, &value)
                .map_err(invalid)?;
            state.insert(name, value);
        }
        if !policy.permits(&state)? {
            return Err(invalid("Witness violates an invariant"));
        }
    }
    let scene = input.scenes.iter().find(|s| s.id == policy.target.scene).unwrap();
    if state != proof.state
        || project(&state, &relevant) != row.values
        || !wobu_narrative::evaluate::evaluate(
            scene.entry.as_ref().unwrap_or(&Condition::Always),
            &state,
        )
        .map_err(invalid)?
    {
        return Err(invalid("Witness does not reach its declared state/admission condition"));
    }
    Ok(())
}
