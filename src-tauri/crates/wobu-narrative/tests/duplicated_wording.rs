//! #209. One digest in two places is one wording stored twice — and nothing else
//! is.

use wobu_narrative::*;

fn slot_with(texts: Vec<Text>) -> DialogueSlot {
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants = texts.into_iter().map(Variant::new).collect();
    slot
}

fn scene_with(name: &str, beats: Vec<(&str, Vec<Text>)>) -> Scene {
    let mut scene = Scene::new(name);
    for (title, texts) in beats {
        let mut beat = Beat::new(title);
        beat.dialogue.push(slot_with(texts));
        scene.beats.push(beat);
    }
    scene
}

fn asset_with(name: &str, texts: Vec<Text>) -> TextAsset {
    let mut asset = TextAsset::new(TextKind::QuestSummary, name, Name::new("objective").unwrap());
    let mut entry = TextEntry::new("Objective");
    entry.lines.push(slot_with(texts));
    asset.entries.push(entry);
    asset
}

#[test]
fn one_wording_in_a_scene_and_a_text_asset_is_one_finding_naming_both_sites() {
    let line = Text::written("You look like you need a job more than a coffee.");
    let scene = scene_with("A shift at the diner", vec![("Rosa sizes her up", vec![line.clone()])]);
    let asset = asset_with("Objective — A shift at the diner", vec![line]);

    let found = duplicated_wording(std::slice::from_ref(&scene), std::slice::from_ref(&asset), &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].sites.len(), 2);
    assert_eq!(
        found[0].sites.iter().map(|s| s.container).collect::<Vec<_>>(),
        // Ordered, so a rendered list does not reshuffle between runs.
        {
            let mut expected =
                vec![WordingContainer::Scene(scene.id), WordingContainer::TextAsset(asset.id)];
            expected.sort();
            expected
        }
    );
    // Each site is keyed finely enough to select that copy and no other.
    assert!(
        found[0]
            .sites
            .iter()
            .all(|site| matches!(site.site, Site::Variant { .. } | Site::TextVariant { .. }))
    );
}

#[test]
fn the_same_line_pasted_into_two_beats_of_one_scene_is_reported() {
    let bark = Text::written("Keep moving.");
    let scene = scene_with(
        "The gate",
        vec![("First pass", vec![bark.clone()]), ("Second pass", vec![bark])],
    );
    let found = duplicated_wording(&[scene], &[], &[]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].sites.len(), 2);
}

#[test]
fn wordings_that_differ_only_by_provenance_are_not_reported() {
    // The check is on the digest, not on text similarity. A revision hashes the
    // words *and* their provenance, so these are two different wordings — which
    // is the honest answer: one was typed and one arrived from elsewhere, and
    // calling that a paste would be a claim about history that is not true.
    let body = "The ridge is burning.";
    let typed = Text::written(body);
    let mut imported = typed.clone();
    imported.set_body(body, Provenance::Imported { source: "mcp".into() });
    assert_ne!(typed.revision, imported.revision);

    let scene = scene_with("Ashfall", vec![("A", vec![typed]), ("B", vec![imported])]);
    assert!(duplicated_wording(&[scene], &[], &[]).is_empty());
}

#[test]
fn a_wording_that_appears_once_is_not_a_duplicate_and_neither_is_an_empty_project() {
    let scene = scene_with("Alone", vec![("Only", vec![Text::written("Once.")])]);
    assert!(duplicated_wording(&[scene], &[], &[]).is_empty());
    assert!(duplicated_wording(&[], &[], &[]).is_empty());
}

#[test]
fn a_suppression_with_a_reason_removes_the_finding_and_one_without_does_not() {
    let refrain = Text::written("We were never here.");
    let scene =
        scene_with("Refrain", vec![("A", vec![refrain.clone()]), ("B", vec![refrain.clone()])]);
    let revision = refrain.revision.clone();

    let blank = WordingSuppression { revision: revision.clone(), rationale: "   ".into() };
    assert_eq!(duplicated_wording(std::slice::from_ref(&scene), &[], &[blank]).len(), 1);

    let stated = WordingSuppression { revision, rationale: "A deliberate refrain.".into() };
    assert!(duplicated_wording(&[scene], &[], &[stated]).is_empty());
}

#[test]
fn a_supporting_text_authoring_adapter_is_not_counted_beside_its_own_asset() {
    // A `TextAsset` projected into a `Scene` for the review machinery is the same
    // document, so counting both would report every line of every bark as
    // duplicated with itself.
    let asset = asset_with("Objective", vec![Text::written("Find Rosa at the diner.")]);
    let adapter = asset.editorial_scene();
    assert!(duplicated_wording(&[adapter], &[asset], &[]).is_empty());
}
