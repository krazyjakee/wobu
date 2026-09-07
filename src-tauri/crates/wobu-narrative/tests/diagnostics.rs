//! Source-level analysis: dangling and unresolved destinations keyed to the
//! element responsible, malformed references, and type errors.

mod support;

use support::{ashfall_state, council_hearing, name};
use wobu_narrative::{
    Assignment, BeatId, CompareOp, Comparison, Condition, Destination, DestinationSite, Diagnostic,
    DialogueSlot, Effect, Increment, Intent, Operand, Outcome, Problem, SceneCatalog, SceneId,
    Site, Speaker, StateSchema, Text, TypeError, Value, Variant,
};

fn compare(v: &str, op: CompareOp, value: Operand) -> Condition {
    Condition::Compare(Comparison { var: name(v), op, value })
}

fn problems(diagnostics: &[Diagnostic]) -> Vec<&Problem> {
    diagnostics.iter().map(|d| &d.problem).collect()
}

#[test]
fn a_complete_scene_reports_only_the_work_still_to_do() {
    let council = council_hearing();
    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());

    // The one deliberate gap in the fixture: a slot nobody has written yet.
    // Shown as a task rather than filled in on save.
    assert_eq!(problems(&found), vec![&Problem::MissingText], "{found:?}");
    assert!(council.scene.destination_issues(&SceneCatalog::unknown()).is_empty());
}

#[test]
fn a_destination_naming_no_beat_is_keyed_to_the_choice_responsible() {
    let mut council = council_hearing();
    let ghost = BeatId::new();
    let beat = council.scene.beats[0].id;
    let choice = council.scene.beats[0].choices[0].id;
    council.scene.beats[0].choices[0].to = Destination::Beat(ghost);

    let found = council.scene.destination_issues(&SceneCatalog::unknown());

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].site, Site::Destination(DestinationSite::Choice { beat, choice }));
    assert_eq!(found[0].problem, Problem::DanglingBeat { beat: ghost });
}

#[test]
fn a_destination_naming_no_beat_is_keyed_to_the_outcome_responsible() {
    let mut council = council_hearing();
    let beat = council.scene.beats[1].id;
    let outcome = council.scene.beats[1].outcomes[0].id;
    council.scene.beats[1].outcomes[0].to = Destination::Beat(BeatId::new());

    let found = council.scene.destination_issues(&SceneCatalog::unknown());

    assert_eq!(found[0].site, Site::Destination(DestinationSite::Outcome { beat, outcome }));
}

#[test]
fn a_deleted_beat_is_explained_by_its_tombstone_rather_than_by_its_id() {
    let mut council = council_hearing();
    let verdict = council.scene.beats[2].id;
    council.scene.remove_beat(verdict, Some("folded into the epilogue".into()));

    let found = council.scene.destination_issues(&SceneCatalog::unknown());
    let deleted: Vec<_> =
        found.iter().filter(|d| matches!(d.problem, Problem::DeletedBeat { .. })).collect();

    // Two routes reconverged on the verdict, so removing it breaks both, and
    // each is reported against its own choice or outcome.
    assert_eq!(deleted.len(), 2, "{found:?}");
    for diagnostic in &deleted {
        let text = diagnostic.to_string();
        assert!(text.contains("Verdict"), "{text}");
        assert!(text.contains("folded into the epilogue"), "{text}");
    }

    // Without the tombstone the same break would only be able to name an id.
    let mut fresh = council_hearing().scene;
    fresh.beats[0].choices[1].to = Destination::Beat(BeatId::new());
    let anonymous = fresh.destination_issues(&SceneCatalog::unknown());
    assert!(matches!(anonymous[0].problem, Problem::DanglingBeat { .. }));
}

#[test]
fn a_scene_link_is_only_checked_when_the_project_is_known() {
    let mut council = council_hearing();
    let elsewhere = SceneId::new();
    council.scene.beats[2].outcomes[0].to = Destination::Scene(elsewhere);

    // Holding one file, "every link out of this scene is broken" would be a
    // false alarm, so nothing is claimed.
    assert!(council.scene.destination_issues(&SceneCatalog::unknown()).is_empty());

    // Holding the project, the same link is a real break.
    let catalog = SceneCatalog::of([council.scene.id]);
    let found = council.scene.destination_issues(&catalog);
    assert_eq!(found[0].problem, Problem::UnknownScene { scene: elsewhere });

    // And a link the project does resolve is not reported.
    let catalog = SceneCatalog::of([council.scene.id, elsewhere]);
    assert!(council.scene.destination_issues(&catalog).is_empty());
}

#[test]
fn an_explicit_end_resolves() {
    let council = council_hearing();
    // The verdict's outcome is an explicit end; nothing about it is dangling.
    assert!(council.scene.destination_issues(&SceneCatalog::of([council.scene.id])).is_empty());
}

#[test]
fn a_beat_with_no_way_out_is_reported_against_the_beat_itself() {
    let mut council = council_hearing();
    let verdict = council.scene.beats[2].id;
    council.scene.beats[2].outcomes.clear();

    let found = council.scene.destination_issues(&SceneCatalog::unknown());

    assert_eq!(found[0].site, Site::Destination(DestinationSite::Beat(verdict)));
    assert_eq!(found[0].problem, Problem::NoDestination);
    // The message has to distinguish this from a finished branch.
    assert!(found[0].to_string().contains("explicit end"), "{}", found[0]);
}

#[test]
fn a_speaker_who_is_not_in_the_scene_is_a_malformed_reference() {
    let mut council = council_hearing();
    let stranger = wobu_core::new_id();
    let beat = council.scene.beats[0].id;
    let mut slot = DialogueSlot::new(Speaker::Entity(stranger));
    slot.variants.push(Variant::new(Text::written("Who is this?")));
    let slot_id = slot.id;
    council.scene.beats[0].dialogue.push(slot);

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    let hit = found
        .iter()
        .find(|d| d.problem == Problem::NotAParticipant { entity: stranger })
        .unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(hit.site, Site::DialogueSlot { beat, slot: slot_id });
}

#[test]
fn an_intent_about_someone_outside_the_scene_is_reported_by_position() {
    let mut council = council_hearing();
    let stranger = wobu_core::new_id();
    let beat = council.scene.beats[2].id;
    council.scene.beats[2].intents.push(Intent {
        subject: Speaker::Entity(stranger),
        intent: "Object from the gallery".to_string(),
    });

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    let hit = found.iter().find(|d| d.site == Site::Intent { beat, index: 0 }).unwrap();
    assert_eq!(hit.problem, Problem::NotAParticipant { entity: stranger });
}

#[test]
fn a_condition_over_an_undeclared_variable_is_reported_at_its_field() {
    let mut council = council_hearing();
    let beat = council.scene.beats[0].id;
    let choice = council.scene.beats[0].choices[0].id;
    council.scene.beats[0].choices[0].requires =
        Some(compare("morale", CompareOp::Ge, Operand::Literal(Value::Int(1))));

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    let hit = found
        .iter()
        .find(|d| d.site == Site::Destination(DestinationSite::Choice { beat, choice }))
        .unwrap();
    assert_eq!(hit.problem, Problem::Type(TypeError::UndeclaredVariable(name("morale"))));
}

#[test]
fn an_effect_writing_a_host_owned_variable_is_reported() {
    let mut council = council_hearing();
    council.scene.beats[0].choices[1].effects.push(Effect::Set(Assignment {
        var: name("difficulty"),
        value: Operand::Literal(Value::Enum(name("harsh"))),
    }));

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    assert!(
        found.iter().any(|d| d.problem == Problem::Type(TypeError::HostOwned(name("difficulty")))),
        "{found:?}"
    );
}

#[test]
fn an_out_of_range_effect_is_reported() {
    let mut council = council_hearing();
    council.scene.beats[1].outcomes[0].effects =
        vec![Effect::Add(Increment { var: name("has_logbook"), by: 1 })];

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    assert!(
        found.iter().any(|d| matches!(d.problem, Problem::Type(TypeError::NotAnInteger { .. }))),
        "{found:?}"
    );
}

#[test]
fn a_scene_entry_condition_is_checked_too() {
    let mut council = council_hearing();
    council.scene.entry =
        Some(compare("beacon_quest", CompareOp::Gt, Operand::Literal(Value::Int(1))));

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    let hit = found.iter().find(|d| d.site == Site::Entry).unwrap();
    assert!(matches!(hit.problem, Problem::Type(TypeError::NotOrdered { .. })), "{hit}");
}

#[test]
fn an_empty_schema_reports_every_variable_rather_than_none() {
    // A scene opened without its project must not look clean.
    let council = council_hearing();
    let found = council.scene.diagnostics(&StateSchema::empty(), &SceneCatalog::unknown());
    assert!(
        found.iter().any(|d| matches!(d.problem, Problem::Type(TypeError::UndeclaredVariable(_)))),
        "{found:?}"
    );
}

#[test]
fn a_repeated_id_is_reported() {
    // The shape a copy-paste in the Source view leaves behind. Undetected, one
    // locale row would overwrite another.
    let mut council = council_hearing();
    let clone = council.scene.beats[2].clone();
    council.scene.beats.push(clone);

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    assert!(found.iter().any(|d| matches!(d.problem, Problem::DuplicateId { .. })), "{found:?}");
}

#[test]
fn a_hand_edited_body_that_no_longer_matches_its_revision_is_reported_not_repaired() {
    let mut council = council_hearing();
    let variant = &mut council.scene.beats[1].dialogue[0].variants[0];
    let recorded = variant.text.revision.clone();
    // What editing the YAML body without touching the revision looks like.
    variant.text.body = "I was there, and I said so.".to_string();

    let found = council.scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    let hit = found
        .iter()
        .find(|d| matches!(d.problem, Problem::RevisionMismatch { .. }))
        .unwrap_or_else(|| panic!("{found:?}"));
    assert!(hit.to_string().contains("translation"), "{hit}");

    // Still not repaired: the recorded revision is what a locale pack is keyed
    // to, so adopting the new one has to be a decision somebody makes.
    assert_eq!(council.scene.beats[1].dialogue[0].variants[0].text.revision, recorded);
    council.scene.beats[1].dialogue[0].variants[0].text.reseal();
    assert!(council.scene.beats[1].dialogue[0].variants[0].text.revision_matches());
}

#[test]
fn an_empty_scene_says_so() {
    let scene = wobu_narrative::Scene::new("Nothing yet");
    let found = scene.diagnostics(&ashfall_state(), &SceneCatalog::unknown());
    assert_eq!(problems(&found), vec![&Problem::NoBeats]);
}

#[test]
fn a_locked_slot_never_becomes_eligible_for_generation() {
    // Enforced on the model rather than in the UI, so a caller that bypassed the
    // form cannot generate into it either.
    let council = council_hearing();
    let verdict = &council.scene.beats[2].dialogue[0];
    assert!(!verdict.may_generate());
    assert!(council.scene.beats[0].dialogue[0].may_generate());
}

#[test]
fn an_outcome_added_to_a_dead_end_clears_the_diagnostic() {
    let mut council = council_hearing();
    council.scene.beats[2].outcomes.clear();
    assert!(!council.scene.destination_issues(&SceneCatalog::unknown()).is_empty());

    council.scene.beats[2]
        .outcomes
        .push(Outcome::new(Destination::End { label: "Adjourned".into() }));
    assert!(council.scene.destination_issues(&SceneCatalog::unknown()).is_empty());
}

#[test]
fn a_scene_lists_its_outward_links_for_a_caller_that_can_resolve_them() {
    // The seam the project-wide validation of #171 sits on: this crate says
    // which scenes are named, and something holding the project says whether
    // they exist.
    let mut council = council_hearing();
    let elsewhere = SceneId::new();
    let beat = council.scene.beats[2].id;
    let outcome = council.scene.beats[2].outcomes[0].id;
    council.scene.beats[2].outcomes[0].to = Destination::Scene(elsewhere);

    let links: Vec<_> = council.scene.scene_links().collect();
    assert_eq!(links, vec![(DestinationSite::Outcome { beat, outcome }, elsewhere)]);
}

#[test]
fn a_scene_lists_every_dialogue_slot_with_the_beat_it_belongs_to() {
    let council = council_hearing();
    let slots: Vec<_> = council.scene.dialogue_slots().collect();

    assert_eq!(slots.len(), 4);
    assert_eq!(slots[0].0, council.scene.beats[0].id);
    // Including the one still waiting for words, so a "needs text" filter sees
    // it without walking the tree itself.
    assert_eq!(slots.iter().filter(|(_, slot)| slot.is_missing_text()).count(), 1);
}
