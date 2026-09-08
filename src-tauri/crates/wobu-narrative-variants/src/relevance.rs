use super::*;
use wobu_narrative::{Effect, Operand, Owner};
/// Static union of the target's potential condition/context reads. Predicates
/// remain relevant even when inactive in the default state.
pub(crate) fn relevant(input: &Input<'_>, target: &Target) -> Result<BTreeSet<Name>, Invalid> {
    let scene = input
        .scenes
        .iter()
        .find(|s| s.id == target.scene)
        .ok_or_else(|| invalid("Missing scene"))?;
    let beat = scene.beat(target.beat).ok_or_else(|| invalid("Missing beat"))?;
    let mut conditions: Vec<&Condition> = scene.entry.iter().collect();
    conditions.extend(
        beat.dialogue.iter().flat_map(|s| s.variants.iter().filter_map(|v| v.when.as_ref())),
    );
    conditions.extend(beat.choices.iter().filter_map(|c| c.requires.as_ref()));
    conditions.extend(beat.outcomes.iter().filter_map(|o| o.when.as_ref()));
    // Direct route gates and preceding automatic outcome priority can control
    // admission. They remain distinctions even though a runtime route is not proved.
    for predecessor in &scene.beats {
        conditions.extend(
            predecessor
                .choices
                .iter()
                .filter(|c| c.to == wobu_narrative::Destination::Beat(target.beat))
                .filter_map(|c| c.requires.as_ref()),
        );
        if let Some(last) = predecessor
            .outcomes
            .iter()
            .rposition(|o| o.to == wobu_narrative::Destination::Beat(target.beat))
        {
            conditions.extend(predecessor.outcomes[..=last].iter().filter_map(|o| o.when.as_ref()));
        }
    }
    let speakers: BTreeSet<_> = beat.dialogue.iter().filter_map(|s| s.speaker.entity()).collect();
    let participants: BTreeSet<_> = scene.participants.iter().map(|p| p.entity).collect();
    let facts: BTreeSet<_> = input
        .world
        .knowledge
        .iter()
        .filter(|k| speakers.contains(&k.character))
        .map(|k| k.fact)
        .collect();
    conditions.extend(
        input.world.knowledge.iter().filter(|k| speakers.contains(&k.character)).map(|k| &k.when),
    );
    conditions.extend(
        input
            .world
            .relationships
            .iter()
            .filter(|r| speakers.contains(&r.from) && participants.contains(&r.to))
            .map(|r| &r.when),
    );
    conditions.extend(
        input
            .world
            .restrictions
            .iter()
            .filter(|r| {
                r.characters.is_empty() || r.characters.iter().any(|id| speakers.contains(id))
            })
            .map(|r| &r.until),
    );
    conditions.extend(
        input
            .world
            .events
            .iter()
            .filter(|e| {
                e.entity_ids.iter().any(|id| speakers.contains(id))
                    || e.fact_ids.iter().any(|id| facts.contains(id))
            })
            .map(|e| &e.when),
    );
    for condition in &conditions {
        input.schema.check_condition(condition).map_err(invalid)?;
    }
    Ok(conditions.into_iter().flat_map(Condition::variables).cloned().collect())
}
fn reads(effect: &Effect) -> Vec<&Name> {
    match effect {
        Effect::Set(a) => match &a.value {
            Operand::Var(name) => vec![name],
            _ => vec![],
        },
        Effect::Add(a) => vec![&a.var],
        Effect::Command(c) => c
            .args
            .iter()
            .filter_map(|arg| match arg {
                Operand::Var(name) => Some(name),
                _ => None,
            })
            .collect(),
    }
}
pub(crate) fn unknown_reasons(
    input: &Input<'_>,
    policy: &Policy,
    transitions: &[Transition],
    relevant: &BTreeSet<Name>,
) -> Vec<String> {
    let mut closure = relevant.clone();
    loop {
        let before = closure.len();
        for transition in transitions {
            if transition.effects.iter().filter_map(Effect::writes).any(|n| closure.contains(n)) {
                closure.extend(transition.when.variables().into_iter().cloned());
                for effect in &transition.effects {
                    closure.extend(reads(effect).into_iter().cloned());
                }
            }
        }
        for invariant in &policy.invariants {
            let names = invariant.variables();
            if names.iter().any(|n| closure.contains(*n)) {
                closure.extend(names.into_iter().cloned());
            }
        }
        if closure.len() == before {
            break;
        }
    }
    let mut reasons = BTreeSet::new();
    if input
        .scenes
        .iter()
        .find(|s| s.id == policy.target.scene)
        .and_then(|s| s.beats.first())
        .is_some_and(|b| b.id != policy.target.beat)
    {
        reasons.insert("Target is not the scene entry beat; incoming route feasibility is unknown without runtime control exploration.".into());
    }
    if policy.external {
        reasons.insert("External transitions are not declared closed.".into());
    }
    for d in input.schema.iter().filter(|d| d.owner == Owner::Host && closure.contains(&d.name)) {
        reasons.insert(format!(
            "Host input {} can change outside the declared transition model.",
            d.name
        ));
    }
    // Authored state effects are not implied by a world-event or quest binding.
    // Without a control-flow proof they remain an explicit unknown frontier.
    for scene in input.scenes {
        for beat in &scene.beats {
            let effects = beat
                .choices
                .iter()
                .flat_map(|c| c.effects.iter())
                .chain(beat.outcomes.iter().flat_map(|o| o.effects.iter()));
            for effect in effects {
                if matches!(effect, Effect::Command(_))
                    || effect.writes().is_some_and(|n| closure.contains(n))
                {
                    reasons.insert(format!("Unmodelled authored effect/command at scene/{}/beat/{}; runtime route exploration is not claimed.",scene.id,beat.id));
                }
            }
        }
    }
    for quest in &input.world.quests {
        if quest.scene_ids.contains(&policy.target.scene)
            && !policy.quests.iter().any(|b| b.quest == quest.id)
        {
            reasons.insert(format!("Quest {} has no explicit state-variable binding.", quest.id));
        }
    }
    reasons.into_iter().collect()
}
