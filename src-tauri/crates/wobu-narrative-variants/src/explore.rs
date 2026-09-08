use super::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use wobu_narrative::{Effect, evaluate};
struct Visited {
    state: State,
    parent: Option<usize>,
    source: Option<String>,
    initial: usize,
    depth: usize,
}
fn witness(
    visited: &[Visited],
    mut index: usize,
    target: &Target,
    stop: &mut impl FnMut() -> bool,
) -> Option<Witness> {
    let state = visited[index].state.clone();
    let initial = visited[index].initial;
    let mut transitions = Vec::new();
    loop {
        if stop() {
            return None;
        }
        if let Some(source) = &visited[index].source {
            transitions.push(source.clone());
        }
        if let Some(parent) = visited[index].parent { index = parent } else { break }
    }
    transitions.reverse();
    Some(Witness {
        state,
        transitions,
        initial,
        target: target.clone(),
        runtime_route_verified: false,
    })
}
fn size(ty: &VarType) -> u128 {
    match ty {
        VarType::Bool => 2,
        VarType::Enum { members } => members.len() as u128,
        VarType::Int { min, max } => ((*max as i128) - (*min as i128) + 1) as u128,
    }
}
fn value(ty: &VarType, index: u128) -> Value {
    match ty {
        VarType::Bool => Value::Bool(index != 0),
        VarType::Enum { members } => Value::Enum(members[index as usize].clone()),
        VarType::Int { min, .. } => Value::Int((*min as i128 + index as i128) as i64),
    }
}
fn configuration(domains: &[Domain], mut ordinal: u128) -> State {
    domains
        .iter()
        .rev()
        .map(|d| {
            let width = size(&d.ty);
            let value = value(&d.ty, ordinal % width);
            ordinal /= width;
            (d.name.clone(), value)
        })
        .collect()
}
pub(super) fn analyze(
    input: Input<'_>,
    policy: &Policy,
    mut stop: impl FnMut() -> bool,
) -> Result<Report, Invalid> {
    let began = Instant::now();
    let transitions = policy.validate(&input)?;
    let relevant = super::relevance::relevant(&input, &policy.target)?;
    let domains: Vec<_> = relevant
        .iter()
        .map(|n| {
            input
                .schema
                .get(n)
                .map(|d| Domain { name: n.clone(), ty: d.ty.clone() })
                .ok_or_else(|| invalid("Missing relevant declaration"))
        })
        .collect::<Result<_, _>>()?;
    let mut exact = true;
    let potential = domains.iter().fold(1u128, |total, d| match total.checked_mul(size(&d.ty)) {
        Some(n) => n,
        None => {
            exact = false;
            u128::MAX
        }
    });
    let mut reasons = super::relevance::unknown_reasons(&input, policy, &transitions, &relevant);
    let mut visited = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for (initial, state) in policy.initial.iter().enumerate() {
        if seen.contains(&state_key(state)) {
            continue;
        }
        if visited.len() >= policy.limits.states {
            reasons.push("Concrete state limit left initial states unexplored.".into());
            break;
        }
        if seen.insert(state_key(state)) {
            queue.push_back(visited.len());
            visited.push(Visited {
                state: state.clone(),
                parent: None,
                source: None,
                initial,
                depth: 0,
            });
        }
    }
    let scene = input
        .scenes
        .iter()
        .find(|s| s.id == policy.target.scene)
        .ok_or_else(|| invalid("Missing target scene"))?;
    let mut admitted = BTreeMap::<String, (State, usize)>::new();
    let mut entry_denied = BTreeSet::new();
    'exploration: while let Some(index) = queue.pop_front() {
        if stop() || began.elapsed() >= Duration::from_millis(policy.limits.milliseconds) {
            reasons.push("Time/cancellation limit left an unexplored frontier.".into());
            break;
        }
        let state = visited[index].state.clone();
        let projected = project(&state, &relevant);
        let key = state_key(&projected);
        if evaluate::evaluate(scene.entry.as_ref().unwrap_or(&Condition::Always), &state)
            .map_err(invalid)?
        {
            admitted.entry(key).or_insert((projected, index));
        } else {
            entry_denied.insert(key);
        }
        for transition in &transitions {
            if stop() || began.elapsed() >= Duration::from_millis(policy.limits.milliseconds) {
                reasons
                    .push("Time/cancellation limit left an unexplored transition frontier.".into());
                break 'exploration;
            }
            if !evaluate::evaluate(&transition.when, &state).map_err(invalid)? {
                continue;
            }
            let mut next = state.clone();
            let mut valid = true;
            for effect in &transition.effects {
                if matches!(effect, Effect::Command(_)) {
                    reasons.push(format!(
                        "External command at {} has no modelled result.",
                        transition.source
                    ));
                    valid = false;
                    break;
                }
                let result = evaluate::assignment(effect, &next).and_then(|(name, value)| {
                    let decl = input
                        .schema
                        .get(&name)
                        .ok_or_else(|| evaluate::EvaluationError::MissingState(name.to_string()))?;
                    evaluate::validate_write(&name, &decl.ty, decl.owner, &value)?;
                    next.insert(name, value);
                    Ok(())
                });
                if let Err(error) = result {
                    reasons.push(format!("{}: {error}; successor unknown.", transition.source));
                    valid = false;
                    break;
                }
            }
            if !valid || !policy.permits(&next)? {
                continue;
            }
            let key = state_key(&next);
            if seen.contains(&key) {
                continue;
            }
            if visited.len() >= policy.limits.states {
                reasons.push("Concrete state limit left an unexplored frontier.".into());
                continue;
            }
            seen.insert(key);
            queue.push_back(visited.len());
            visited.push(Visited {
                state: next,
                parent: Some(index),
                source: Some(transition.source.clone()),
                initial: visited[index].initial,
                depth: visited[index].depth + 1,
            });
        }
    }
    reasons.sort();
    reasons.dedup();
    let complete = reasons.is_empty();
    let included = admitted.len() as u128;
    let unknown =
        if complete { entry_denied.len() as u128 } else { potential.saturating_sub(included) };
    let excluded = if complete { potential.saturating_sub(included + unknown) } else { 0 };
    let mut values: BTreeMap<String, State> = admitted
        .iter()
        .take(policy.limits.variants)
        .map(|(key, (state, _))| (key.clone(), state.clone()))
        .collect();
    let mut ordinal = 0u128;
    while values.len() < policy.limits.variants && ordinal < potential {
        if stop() || began.elapsed() >= Duration::from_millis(policy.limits.milliseconds) {
            reasons.push("Display enumeration reached its time limit; remaining configurations are not materialized.".into());
            break;
        }
        let state = configuration(&domains, ordinal);
        values.entry(state_key(&state)).or_insert(state);
        ordinal += 1;
    }
    if (values.len() as u128) < potential {
        reasons.push("Variant display/materialization limit: remaining configurations are summarized by the counts.".into());
    }
    let mut rows = Vec::new();
    let mut witness_steps = 0usize;
    for (key, values) in values {
        if stop() || began.elapsed() >= Duration::from_millis(policy.limits.milliseconds) {
            reasons.push(
                "Report construction reached its time limit; remaining rows were omitted.".into(),
            );
            break;
        }
        if let Some((_, index)) = admitted.get(&key) {
            if witness_steps.saturating_add(visited[*index].depth) > policy.limits.witness_steps {
                reasons.push(
                    "Total retained witness-step limit reached; remaining rows were omitted."
                        .into(),
                );
                break;
            }
            witness_steps += visited[*index].depth;
        }
        let (classification, reason, proof) = if let Some((_, index)) = admitted.get(&key) {
            let Some(proof) = witness(&visited, *index, &policy.target, &mut || {
                stop() || began.elapsed() >= Duration::from_millis(policy.limits.milliseconds)
            }) else {
                reasons.push(
                    "Witness reconstruction reached its time limit; remaining rows were omitted."
                        .into(),
                );
                break;
            };
            (
                Classification::Included,
                "Witness under the declared state model; runtime route is not verified.".into(),
                Some(proof),
            )
        } else if complete && !entry_denied.contains(&key) {
            (Classification::Excluded,"Exhaustive closed-model exploration found no state for this configuration; declared invariants restrict every successor.".into(),None)
        } else {
            (
                Classification::Unknown,
                if entry_denied.contains(&key) {
                    "Scene entry rejects the model witness; no runtime route proof is available."
                        .into()
                } else {
                    "External input or an unfinished proof frontier prevents exclusion.".into()
                },
                None,
            )
        };
        let coverage = if let Some(proof) = &proof {
            let Some(coverage) = super::coverage(scene, &policy.target, &proof.state, &mut || {
                stop() || began.elapsed() >= Duration::from_millis(policy.limits.milliseconds)
            })?
            else {
                reasons.push(
                    "Coverage report reached its time limit; remaining rows were omitted.".into(),
                );
                break;
            };
            coverage
        } else {
            Vec::new()
        };
        rows.push(Row {
            id: hash(&("narrative_configuration/1", &policy.target, &domains, &values)),
            when: condition(&values),
            values,
            classification,
            reason,
            witness: proof,
            coverage,
        });
    }
    Ok(Report {version:VERSION,target:policy.target.clone(),policy_hash:hash(policy),domains,potential:Count::of(potential,exact),included:Count::of(included,true),excluded:Count::of(excluded,exact || !complete),unknown:Count::of(unknown,exact || complete),rows,complete,explored_states:visited.len(),limits:policy.limits.clone(),reasons,
        assumptions:vec!["World-event effects and quest/state bindings are explicitly authored in this policy; source prose supplies no effects.".into(),"Transitions model configurations before admission to this beat. They are not an executed runtime route or command acknowledgement.".into(),"Invariants constrain initial states and every model successor. Exclusions hold only under these declared assumptions.".into()]})
}
