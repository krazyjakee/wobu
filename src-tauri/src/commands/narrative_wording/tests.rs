//! #209, against a real project folder: a quest summary copied out of a scene is
//! found, and the answer to it outlives the index.

use wobu_narrative::{
    DialogueSlot, Name, Speaker, Text, TextEntry, TextKind, Variant, WordingSuppression,
};

use super::*;
use crate::commands::narrative::tests::{Temp, project};

const LINE: &str = "You look like you need a job more than a coffee.";

/// A scene whose only line is `LINE`, and a quest summary asset holding a
/// byte-identical copy of it — the shape an import produced in the downstream
/// project this issue came from.
fn project_with_a_copied_line(project: &mut Project) -> (String, String) {
    let mut file = project.create_scene("A shift at the diner").unwrap();
    let mut beat = wobu_narrative::Beat::new("Rosa sizes her up");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    let line = Text::written(LINE);
    slot.variants.push(Variant::new(line.clone()));
    beat.dialogue.push(slot);
    beat.outcomes.push(wobu_narrative::Outcome::new(wobu_narrative::Destination::End {
        label: "Done".into(),
    }));
    file.scene.beats.push(beat);
    assert!(matches!(project.save_scene(&mut file).unwrap(), wobu_store::SourceSave::Saved(_)));

    let mut asset_file = project
        .create_text_asset(
            TextKind::QuestSummary,
            "Objective — A shift at the diner",
            Name::new("objective_s02").unwrap(),
        )
        .unwrap();
    let mut entry = TextEntry::new("Objective");
    let mut copied = DialogueSlot::new(Speaker::Narrator);
    // The same words with the same provenance, which is what "pasted" means and
    // therefore what one shared revision means.
    copied.variants.push(Variant::new(line));
    entry.lines.push(copied);
    asset_file.asset.entries.push(entry);
    assert!(matches!(
        project.save_text_asset(&mut asset_file).unwrap(),
        wobu_store::SourceSave::Saved(_)
    ));
    (file.scene.id.to_string(), asset_file.asset.id.to_string())
}

#[test]
fn a_quest_summary_copied_from_a_scene_line_is_reported_at_both_of_its_sites() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let (scene, asset) = project_with_a_copied_line(&mut project);

    let found = report(&mut project).unwrap();
    assert_eq!(found.duplicated.len(), 1, "{:?}", found.duplicated);
    let duplicated = &found.duplicated[0];
    assert_eq!(duplicated.body, LINE);
    // Every copy, so a reader can select each one — not just the first found.
    let containers: Vec<String> =
        duplicated.sites.iter().map(|site| site.container.id().to_string()).collect();
    assert!(containers.contains(&scene), "{containers:?}");
    assert!(containers.contains(&asset), "{containers:?}");
    assert!(duplicated.message().contains("more than once"));
}

#[test]
fn a_suppression_needs_a_reason_and_then_outlives_a_rebuilt_index() {
    let temp = Temp::new();
    let mut project = project(&temp);
    project_with_a_copied_line(&mut project);
    let before = report(&mut project).unwrap();
    let revision = before.duplicated[0].revision.clone();

    // A blank reason suppresses nothing: it would be indistinguishable from a
    // warning somebody clicked away.
    assert!(
        project
            .save_wording_suppressions(
                vec![WordingSuppression { revision: revision.clone(), rationale: "  ".into() }],
                &before.guard,
            )
            .is_err()
    );
    assert!(check_rationale(" ").is_err());

    project
        .save_wording_suppressions(
            vec![WordingSuppression {
                revision: revision.clone(),
                rationale: "Rosa's opening line is the objective, on purpose.".into(),
            }],
            &before.guard,
        )
        .unwrap();
    let after = report(&mut project).unwrap();
    assert!(after.duplicated.is_empty(), "{:?}", after.duplicated);
    assert_eq!(after.suppressions.len(), 1);

    // The suppression is a file, not a row in the derived index, so throwing the
    // index away does not throw the decision away.
    let root = project.root().to_path_buf();
    drop(project);
    for entry in std::fs::read_dir(root.join(".wobu")).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().contains("index") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let mut reopened = Project::open(&root).unwrap();
    let rebuilt = report(&mut reopened).unwrap();
    assert!(rebuilt.duplicated.is_empty(), "a rebuilt index lost the suppression");
    assert_eq!(rebuilt.suppressions[0].revision, revision);
}

#[test]
fn a_stale_guard_is_refused_rather_than_overwriting_somebody_elses_answer() {
    let temp = Temp::new();
    let mut project = project(&temp);
    project_with_a_copied_line(&mut project);
    let first = report(&mut project).unwrap();
    let revision = first.duplicated[0].revision.clone();
    project
        .save_wording_suppressions(
            vec![WordingSuppression { revision, rationale: "Deliberate refrain.".into() }],
            &first.guard,
        )
        .unwrap();

    let error = project
        .save_wording_suppressions(Vec::new(), &first.guard)
        .expect_err("a write that did not see the current list was accepted");
    assert!(error.to_string().contains("reload"), "{error}");
}
