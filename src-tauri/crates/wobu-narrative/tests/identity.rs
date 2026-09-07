//! Identity is the spine: renames and reorders preserve it, duplication mints
//! it fresh, deletion leaves enough behind to explain what is gone.

mod support;

use std::collections::HashSet;

use support::council_hearing;
use wobu_narrative::{Provenance, Revision, Text, TombstoneTarget};

/// Every id anywhere in a scene, flattened, so a test can assert on the whole
/// set at once rather than on the three levels it happened to remember.
fn all_ids(scene: &wobu_narrative::Scene) -> Vec<String> {
    let mut ids = vec![scene.id.to_string()];
    for beat in &scene.beats {
        ids.push(beat.id.to_string());
        for slot in &beat.dialogue {
            ids.push(slot.id.to_string());
            ids.extend(slot.variants.iter().map(|v| v.id.to_string()));
        }
        ids.extend(beat.choices.iter().map(|c| c.id.to_string()));
        ids.extend(beat.outcomes.iter().map(|o| o.id.to_string()));
    }
    ids
}

#[test]
fn renaming_changes_nothing_but_the_name() {
    let mut council = council_hearing();
    let before = all_ids(&council.scene);

    council.scene.name = "The hearing at Ashfall".to_string();
    let beat = council.scene.beats[0].id;
    council.scene.beat_mut(beat).unwrap().title = "Lay out the evidence".to_string();
    council.scene.beats[0].choices[0].label = "Produce the logbook".to_string();

    assert_eq!(all_ids(&council.scene), before);
}

#[test]
fn reordering_beats_preserves_every_id_and_every_destination() {
    let mut council = council_hearing();
    let before = all_ids(&council.scene);
    let order_before: Vec<_> = council.scene.beats.iter().map(|b| b.id).collect();
    let destinations_before: Vec<_> = council
        .scene
        .beats
        .iter()
        .flat_map(|b| b.destinations().map(|(_, to)| to.clone()).collect::<Vec<_>>())
        .collect();

    assert!(council.scene.reorder_beat(order_before[2], 0));

    assert_eq!(
        council.scene.beats.iter().map(|b| b.id).collect::<Vec<_>>(),
        vec![order_before[2], order_before[0], order_before[1]],
    );
    // The ids are the same set; only the order changed.
    assert_eq!(
        all_ids(&council.scene).into_iter().collect::<HashSet<_>>(),
        before.into_iter().collect::<HashSet<_>>(),
    );
    // And nothing was rewired: a reorder is a presentation change.
    let destinations_after: HashSet<_> = council
        .scene
        .beats
        .iter()
        .flat_map(|b| b.destinations().map(|(_, to)| to.clone()).collect::<Vec<_>>())
        .collect();
    assert_eq!(destinations_after, destinations_before.into_iter().collect::<HashSet<_>>());
}

#[test]
fn reordering_past_the_end_lands_at_the_end() {
    let mut council = council_hearing();
    let first = council.scene.beats[0].id;
    assert!(council.scene.reorder_beat(first, 99));
    assert_eq!(council.scene.beats.last().unwrap().id, first);
}

#[test]
fn duplicating_a_beat_allocates_a_fresh_id_for_everything_under_it() {
    let mut council = council_hearing();
    let original = council.scene.beats[0].id;
    let before: HashSet<_> = all_ids(&council.scene).into_iter().collect();

    let copy = council.scene.duplicate_beat(original).unwrap();

    assert_ne!(copy, original);
    // Placed beside the original rather than at the end.
    assert_eq!(council.scene.beat_index(copy), Some(1));

    let source = council.scene.beat(original).unwrap().clone();
    let copied = council.scene.beat(copy).unwrap();
    assert_eq!(copied.title, source.title);
    assert_eq!(copied.intents, source.intents);

    // Not one id is shared with anything that already existed.
    let new_ids: Vec<_> = {
        let mut ids = vec![copied.id.to_string()];
        for slot in &copied.dialogue {
            ids.push(slot.id.to_string());
            ids.extend(slot.variants.iter().map(|v| v.id.to_string()));
        }
        ids.extend(copied.choices.iter().map(|c| c.id.to_string()));
        ids.extend(copied.outcomes.iter().map(|o| o.id.to_string()));
        ids
    };
    assert!(!new_ids.is_empty());
    for id in &new_ids {
        assert!(!before.contains(id), "{id} was reused by the duplicate");
    }
}

#[test]
fn duplicating_copies_wording_and_therefore_copies_its_revision() {
    // The other half of the identity rule: an id is new because this is a
    // different slot, a revision is the same because these are the same words.
    let mut council = council_hearing();
    let original = council.scene.beats[1].id;
    let before: Vec<Revision> = council.scene.beats[1].dialogue[0]
        .variants
        .iter()
        .map(|v| v.text.revision.clone())
        .collect();

    let copy = council.scene.duplicate_beat(original).unwrap();
    let after: Vec<Revision> = council.scene.beat(copy).unwrap().dialogue[0]
        .variants
        .iter()
        .map(|v| v.text.revision.clone())
        .collect();

    assert_eq!(after, before);
}

#[test]
fn duplicating_a_scene_rewires_its_internal_destinations() {
    let council = council_hearing();
    let copy = council.scene.duplicated();

    assert_ne!(copy.id, council.scene.id);
    let copied_beats: HashSet<_> = copy.beats.iter().map(|b| b.id).collect();
    let original_beats: HashSet<_> = council.scene.beats.iter().map(|b| b.id).collect();
    assert!(copied_beats.is_disjoint(&original_beats));

    // A copy that still pointed into the original would read as a copy and
    // behave as a trapdoor back into the scene it came from.
    for beat in &copy.beats {
        for (_, destination) in beat.destinations() {
            if let wobu_narrative::Destination::Beat(target) = destination {
                assert!(copied_beats.contains(target), "{target} escapes the copy");
            }
        }
    }
}

#[test]
fn rewriting_a_line_changes_its_revision_and_not_its_identity() {
    let mut council = council_hearing();
    let variant = &mut council.scene.beats[1].dialogue[0].variants[0];
    let id = variant.id;
    let before = variant.text.revision.clone();

    variant.text.set_body("I was there. He tells it straight.", Provenance::Human);

    assert_eq!(variant.id, id);
    assert_ne!(variant.text.revision, before);
    assert!(variant.text.revision_matches());
}

#[test]
fn an_approval_does_not_survive_a_rewrite() {
    let mut text = Text::written("The council finds for the witness.");
    text.lifecycle.review = wobu_narrative::ReviewState::Approved;
    text.set_body("The council finds against the witness.", Provenance::Human);
    assert_eq!(text.lifecycle.review, wobu_narrative::ReviewState::Draft);
}

#[test]
fn deleting_a_beat_leaves_a_tombstone_naming_it() {
    let mut council = council_hearing();
    let verdict = council.scene.beats[2].id;
    let title = council.scene.beats[2].title.clone();
    let slot = council.scene.beats[2].dialogue[0].id;
    let variant = council.scene.beats[2].dialogue[0].variants[0].id;

    let removed = council.scene.remove_beat(verdict, Some("folded into the epilogue".into()));

    assert_eq!(removed.unwrap().id, verdict);
    assert!(!council.scene.has_beat(verdict));

    let stone = council.scene.beat_tombstone(verdict).unwrap();
    assert_eq!(stone.label, title);
    assert_eq!(stone.reason.as_deref(), Some("folded into the epilogue"));

    // Production references key on slots and variants, so those have to be
    // explainable after the beat is gone too.
    assert!(council.scene.tombstone_for(TombstoneTarget::DialogueSlot(slot)).is_some());
    let line = council.scene.tombstone_for(TombstoneTarget::Variant(variant)).unwrap();
    assert!(line.label.contains("The council finds"), "{}", line.label);
}

#[test]
fn deleting_a_beat_does_not_silently_repair_what_pointed_at_it() {
    let mut council = council_hearing();
    let verdict = council.scene.beats[2].id;
    council.scene.remove_beat(verdict, None);

    // The choice still states where the author sent it. Rewriting it here would
    // destroy that statement and leave nothing to undo.
    let still_points_there = council.scene.beats[0]
        .destinations()
        .any(|(_, to)| matches!(to, wobu_narrative::Destination::Beat(b) if *b == verdict));
    assert!(still_points_there);
}
