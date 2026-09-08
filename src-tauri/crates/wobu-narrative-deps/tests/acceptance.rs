//! The acceptance criteria of #168, each as one test over a real capture.
//!
//! The user story is US-06: changing Kael's voice marks dependent lines out of
//! date *without rebuilding unrelated scenes*. Both halves are asserted every
//! time — an affected set is checked as an exact set, never as "at least these",
//! because a tracker that marks everything affected passes every "did it notice"
//! test ever written and is worthless.

mod support;

use std::collections::BTreeSet;

use support::{harbour, variants};
use wobu_narrative::{
    Belief, Condition, KnowledgeClaim, KnowledgeProvenance, Name, Relationship, Value, WorldEvent,
};
use wobu_narrative_deps::{AffectedKind, Reason};

#[test]
fn changing_a_voice_marks_that_speakers_lines_and_nothing_else() {
    let mut world = harbour();
    let kael = world.kael_line();
    let (mira, codex) = (world.mira_line(), world.codex_line());
    let kael_id = world.kael;

    let affected = world.affected(|world| {
        let voice = world.characters.get_mut(&kael_id).unwrap();
        voice.voice = Some("Clipped, certain, and tired of being doubted.".into());
    });

    // Mira's line names Kael as a participant, so it reads his voice too — the
    // resolver puts every participant's voice in front of the model. The codex
    // page has a narrator speaker and no cast, so it does not.
    assert_eq!(variants(&affected), BTreeSet::from([kael, mira]));
    assert!(!variants(&affected).contains(&codex));
    let reasons = &affected.iter().find(|a| a.target.variant() == kael).unwrap().reasons;
    assert_eq!(
        reasons,
        &[Reason::FieldChanged { source: format!("character/{kael_id}/narrative_voice") }]
    );
}

#[test]
fn a_relationship_nobody_ever_referenced_invalidates_when_it_appears() {
    // The subtle criterion. The new relationship has an id no earlier request
    // could have named, so an id-keyed reverse index has nothing to look up. It
    // is found because the *candidate set* of `directed_relationships` changed.
    let mut world = harbour();
    let kael = world.kael_line();
    let (kael_id, mira_id) = (world.kael, world.mira);
    let new_id = wobu_core::new_id();

    let affected = world.affected(|world| {
        world.world.relationships.push(Relationship {
            id: new_id,
            name: "Kael resents Mira".into(),
            from: kael_id,
            to: mira_id,
            kind: Name::new("resentment").unwrap(),
            value: Value::Int(2),
            // Inactive in the default scenario, and still a dependency: the
            // resolver has to consider it before it can decide it does not
            // apply.
            when: Condition::Never,
        });
    });

    assert_eq!(variants(&affected), BTreeSet::from([kael]));
    assert_eq!(
        affected[0].reasons,
        vec![Reason::MemberAdded {
            query: "directed_relationships".into(),
            member: new_id.to_string(),
        }]
    );
}

#[test]
fn removing_a_relation_invalidates_the_lines_that_could_have_used_it() {
    let mut world = harbour();
    let kael = world.kael_line();
    let removed = world.world.relationships[0].id;

    let affected = world.affected(|world| world.world.relationships.clear());

    assert_eq!(variants(&affected), BTreeSet::from([kael]));
    assert_eq!(
        affected[0].reasons,
        vec![Reason::MemberRemoved {
            query: "directed_relationships".into(),
            member: removed.to_string(),
        }]
    );
}

#[test]
fn a_fact_a_query_reaches_invalidates_when_it_is_added_or_removed() {
    // Two directions in one test, because they are one claim: the fact is
    // reached through the speaker's knowledge claim and through the event that
    // cites it, so deleting it leaves a recorded hole and restoring it fills
    // one. Neither is an id the line ever quoted; both are addresses it read.
    let mut world = harbour();
    let kael = world.kael_line();
    let codex = world.codex_line();
    let fact = world.world.facts[0].clone();

    let removed = world.affected(|world| world.world.facts.retain(|f| f.id != fact.id));
    // The codex page cites the fact through a typed source link; Kael's line
    // reaches it through his knowledge claim and the wreck event.
    assert_eq!(variants(&removed), BTreeSet::from([kael, codex]));
    assert!(removed.iter().all(|a| {
        a.reasons.contains(&Reason::FieldRemoved { source: format!("world/facts/{}", fact.id) })
    }));

    let restored = world.affected(|world| world.world.facts.insert(0, fact.clone()));
    assert_eq!(variants(&restored), BTreeSet::from([kael, codex]));
    assert!(restored.iter().all(|a| {
        a.reasons.contains(&Reason::FieldAdded { source: format!("world/facts/{}", fact.id) })
    }));
}

#[test]
fn an_event_becomes_relevant_by_matching_the_query_not_by_being_named() {
    let mut world = harbour();
    let kael = world.kael_line();
    let known = world.world.facts[0].id;
    let new_id = wobu_core::new_id();

    let affected = world.affected(|world| {
        world.world.events.push(WorldEvent {
            id: new_id,
            name: "The inquest".into(),
            summary: "The harbourmaster asked who lit the lamp.".into(),
            fact_ids: vec![known],
            entity_ids: vec![],
            when: Condition::Always,
        })
    });

    assert_eq!(variants(&affected), BTreeSet::from([kael]));
    assert_eq!(
        affected[0].reasons,
        vec![Reason::MemberAdded { query: "relevant_events".into(), member: new_id.to_string() }]
    );
}

#[test]
fn a_knowledge_claim_widens_the_reachable_facts_and_says_so() {
    let mut world = harbour();
    let kael = world.kael_line();
    let kael_id = world.kael;
    let hidden = world.world.facts[1].id;
    let claim = wobu_core::new_id();

    let affected = world.affected(|world| {
        world.world.knowledge.push(KnowledgeClaim {
            id: claim,
            name: "Kael suspects the tide".into(),
            character: kael_id,
            fact: hidden,
            belief: Belief::Unknown,
            provenance: KnowledgeProvenance::Inferred { reason: "The bar was dry.".into() },
            when: Condition::Always,
        })
    });

    assert_eq!(variants(&affected), BTreeSet::from([kael]));
    // Three things moved together and each is named: the claim joined the
    // speaker's candidate set, the fact it names became reachable, and the
    // event query's parameters therefore changed.
    assert!(affected[0].reasons.contains(&Reason::MemberAdded {
        query: "speaker_knowledge".into(),
        member: claim.to_string(),
    }));
    assert!(
        affected[0]
            .reasons
            .contains(&Reason::QueryParametersChanged { query: "relevant_events".into() })
    );
}

#[test]
fn renaming_and_relabelling_without_semantic_change_invalidates_nothing() {
    let mut world = harbour();

    let affected = world.affected(|world| {
        // A display label the model documents as deriving nothing.
        world.texts[0].entries[0].label = "First page".into();
        // A beat that no captured line lives in — added, then renamed.
        world.scenes[0].beats[0].dialogue[1].variants[0].text.lifecycle.review =
            wobu_narrative::ReviewState::Approved;
        // Canvas classification: nothing in the resolver reads an act.
        world.scenes[0].act_id = Some(wobu_core::new_id());
        world.scenes[0].tag_ids = vec![wobu_core::new_id()];
    });

    assert_eq!(affected, Vec::new());
}

#[test]
fn rewriting_one_line_does_not_touch_its_neighbours() {
    // The imprecision this issue exists to remove. The review context that
    // exists today hashes the whole scene document, so this edit currently
    // withdraws the approval on every other line in the file.
    let mut world = harbour();
    let mira = world.mira_line();

    let affected = world.affected(|world| {
        world.scenes[0].beats[0].dialogue[1].variants[0]
            .text
            .set_body("You saw a lantern and a lie.", wobu_narrative::Provenance::Human);
    });

    assert_eq!(
        affected,
        Vec::new(),
        "editing wording moved a neighbour's dependencies: {affected:?}"
    );
    let _ = mira;
}

#[test]
fn upgrading_a_prompt_version_affects_every_line_and_names_the_component() {
    let mut world = harbour();
    let all = variants(&world.index().diff(&Default::default()));

    let affected = world.affected(|world| world.versions.prompt += 1);

    assert_eq!(variants(&affected), all);
    assert!(affected.iter().all(|item| item.reasons.iter().any(|reason| matches!(
        reason,
        Reason::VersionChanged { component, .. } if component == "prompt"
    ))));
}

#[test]
fn a_narrowed_variable_declaration_reaches_the_branches_that_read_it() {
    let mut world = harbour();
    let kael = world.kael_line();
    let trusted = Name::new("trusted").unwrap();

    // Only the line whose condition names the variable is affected, so the
    // second declared variable being untouched is not what makes this pass.
    world.scenes[0].beats[0].dialogue[0].variants[0].when =
        Some(Condition::Compare(wobu_narrative::Comparison {
            var: trusted.clone(),
            op: wobu_narrative::CompareOp::Eq,
            value: wobu_narrative::Operand::Literal(Value::Bool(true)),
        }));

    let affected = world.affected(|world| {
        let declarations: Vec<_> = world
            .schema
            .iter()
            .cloned()
            .map(|mut decl| {
                if decl.name == trusted {
                    decl.description = "Whether Mira believes him.".into();
                }
                decl
            })
            .collect();
        world.schema = wobu_narrative::StateSchema::new(declarations).unwrap();
    });

    assert_eq!(variants(&affected), BTreeSet::from([kael]));
    assert_eq!(
        affected[0].reasons,
        vec![Reason::FieldChanged { source: format!("state/{trusted}") }]
    );
}

#[test]
fn a_new_line_is_reported_as_untracked_rather_than_as_a_change() {
    let mut world = harbour();
    let affected = world.affected(|world| {
        let slot = &mut world.scenes[0].beats[0].dialogue[0];
        slot.variants.push(wobu_narrative::Variant::new(wobu_narrative::Text::written(
            "Or a lantern. I know what I saw.",
        )));
    });
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0].kind, AffectedKind::Untracked);
    assert!(affected[0].before.is_none());
}

#[test]
fn a_deleted_line_is_reported_as_absent_and_keeps_the_fingerprint_its_receipts_used() {
    let mut world = harbour();
    let kael = world.kael_line();
    let fingerprint = world.index().get(kael).unwrap().fingerprint();

    let affected = world.affected(|world| world.scenes[0].beats[0].dialogue[0].variants.clear());

    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0].kind, AffectedKind::Absent);
    assert_eq!(affected[0].before.as_deref(), Some(fingerprint.as_str()));
}

#[test]
fn an_earlier_variant_shadows_a_later_one_and_that_is_a_dependency() {
    let mut world = harbour();
    let kael = world.kael_line();

    let affected = world.affected(|world| {
        let slot = &mut world.scenes[0].beats[0].dialogue[0];
        let mut earlier = wobu_narrative::Variant::new(wobu_narrative::Text::written("Nothing."));
        earlier.when = Some(Condition::Never);
        slot.variants.insert(0, earlier);
    });

    // The inserted line is new; the existing one is affected because what runs
    // before it changed, which is what first-match selection makes a dependency.
    let existing = affected.iter().find(|item| item.target.variant() == kael).unwrap();
    assert_eq!(existing.kind, AffectedKind::Changed);
    assert!(existing.reasons.iter().any(|reason| matches!(
        reason,
        Reason::FieldChanged { source } if source.ends_with("/precedence")
    )));
}

#[test]
fn every_affected_line_explains_itself_as_source_context_line() {
    let mut world = harbour();
    let kael_id = world.kael;
    let affected = world.affected(|world| {
        world.characters.get_mut(&kael_id).unwrap().voice = Some("Hoarse.".into());
    });

    let explanations: Vec<_> = affected.iter().flat_map(|item| item.explanations()).collect();
    assert!(!explanations.is_empty());
    for explanation in &explanations {
        assert_eq!(explanation.source, format!("character/{kael_id}/narrative_voice"));
        assert_eq!(explanation.context, "context");
        assert!(explanation.line.starts_with("scene/"));
        assert!(explanation.line.contains("/variant/"));
        assert!(explanation.message.ends_with("was edited."));
    }
}

#[test]
fn ambient_neighbor_wording_affects_only_other_lines_in_its_exchange() {
    use wobu_narrative::{DialogueSlot, Speaker, Text, TextEntry, TextKind, Variant};
    let mut world = harbour();
    let first = world.codex_line();
    world.texts[0].kind = TextKind::Ambient;
    let mut neighbor = DialogueSlot::new(Speaker::Narrator);
    neighbor.variants.push(Variant::new(Text::written("A reply.")));
    world.texts[0].entries[0].lines.push(neighbor);
    let mut unrelated = TextEntry::new("Another exchange");
    let mut line = DialogueSlot::new(Speaker::Narrator);
    line.variants.push(Variant::new(Text::written("Elsewhere.")));
    unrelated.lines.push(line);
    world.texts[0].entries.push(unrelated);
    let affected = world.affected(|world| {
        world.texts[0].entries[0].lines[1].variants[0]
            .text
            .set_body("A different reply.", wobu_narrative::Provenance::Human);
    });
    assert_eq!(variants(&affected), BTreeSet::from([first]));
    assert!(affected[0].reasons.iter().any(|reason| matches!(reason, Reason::FieldChanged { source } if source.ends_with("/ambient_neighbors"))));
}

#[test]
fn changing_supporting_text_repeat_policy_changes_its_context_only() {
    let mut world = harbour();
    let codex = world.codex_line();
    let affected =
        world.affected(|world| world.texts[0].repeat = wobu_narrative::RepeatPolicy::Once);
    assert_eq!(variants(&affected), BTreeSet::from([codex]));
}
