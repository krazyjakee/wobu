//! Cross-boundary proof using the real capture, compiler and export preparation.
use super::{narrative_context, narrative_export, narrative_preview};
use std::{collections::BTreeMap, fs, path::Path};
use wobu_narrative::{
    Beat, Destination, DialogueSlot, Outcome, Speaker, Text, Variant,
    review::{EditorialAction, ReviewTarget},
};
use wobu_narrative_compiler::Profile;
use wobu_narrative_context::{Options, Selection};
use wobu_store::{
    GraphKey, Layout, LayoutMode, NodeKey, Project, project::narrative_review::ReviewRequest,
};

fn package_bytes(project: &Project, path: &Path) -> BTreeMap<String, Vec<u8>> {
    let (_, package) =
        narrative_export::prepare(project, Profile::Release, BTreeMap::new(), false).unwrap();
    let package = package.expect("the fixture compiles");
    wobu_narrative_package::publish(&package, path).unwrap();
    let mut result: BTreeMap<_, _> = package
        .manifest
        .files
        .keys()
        .map(|rel| (rel.clone(), fs::read(path.join(rel)).unwrap()))
        .collect();
    result.insert("manifest.json".into(), package.manifest_bytes().unwrap());
    result
}
#[test]
fn layout_only_edits_leave_context_dependencies_approval_records_graph_and_package_bytes_identical()
{
    let home = std::env::temp_dir().join(format!("wobu-layout-contract-{}", wobu_core::new_id()));
    fs::create_dir_all(&home).unwrap();
    let mut project = Project::create(&home, "Ashfall").unwrap();
    let mut file = project.create_scene("Council aftermath").unwrap();
    let mut beat = Beat::new("Support");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    let text = Text::written_locked("The council chamber falls silent.");
    slot.variants.push(Variant::new(text));
    beat.dialogue.push(slot);
    beat.outcomes.push(Outcome::new(Destination::End { label: "pause".into() }));
    file.scene.beats.push(beat);
    project.save_scene(&mut file).unwrap();
    let slot = &file.scene.beats[0].dialogue[0];
    let target = ReviewTarget {
        scene: file.scene.id,
        beat: file.scene.beats[0].id,
        slot: slot.id,
        variant: Some(slot.variants[0].id),
    };
    let review = project.review_scene(file.scene.id, None).unwrap();
    let request = ReviewRequest {
        guard: review.guard,
        target,
        context_revision: review.lines[0].context_revision.clone(),
        state_json: review.state_json,
        action: EditorialAction::Approve,
    };
    (file, _) = project.apply_review(&request).unwrap();
    let slot = &file.scene.beats[0].dialogue[0];
    let options = Options {
        selection: Selection {
            scene: file.scene.id,
            beat: file.scene.beats[0].id,
            slot: slot.id,
            variant: Some(slot.variants[0].id),
        },
        state: BTreeMap::new(),
        token_budget: 4000,
    };
    let canonical = |project: &Project| -> BTreeMap<String, Vec<u8>> {
        project
            .narrative_manifest()
            .unwrap()
            .into_iter()
            .map(|entry| {
                let bytes = fs::read(project.root().join(&entry.rel)).unwrap();
                (entry.rel, bytes)
            })
            .collect()
    };
    let source_and_receipts = canonical(&project);
    let review_before =
        serde_json::to_vec(&project.review_scene(file.scene.id, None).unwrap()).unwrap();
    let fingerprint = project.narrative_fingerprint().unwrap();
    let context =
        serde_json::to_vec(&narrative_context::capture(&project, options.clone(), || {}).unwrap())
            .unwrap();
    let graph =
        serde_json::to_vec(&narrative_preview::compile_project(&project, BTreeMap::new()).unwrap())
            .unwrap();
    let package = package_bytes(&project, &home.join("before-export"));
    let mut layout = Layout::empty(GraphKey::of_scene(file.scene.id));
    layout.set_mode(LayoutMode::Manual);
    layout.place(NodeKey::Beat(file.scene.beats[0].id), 450.0, 125.0);
    project.save_scene_layout(&file.scene, &layout).unwrap();
    project.reconcile().unwrap();
    assert_eq!(project.narrative_fingerprint().unwrap(), fingerprint);
    assert_eq!(canonical(&project), source_and_receipts);
    assert_eq!(
        serde_json::to_vec(&project.review_scene(file.scene.id, None).unwrap()).unwrap(),
        review_before
    );
    assert_eq!(
        serde_json::to_vec(&narrative_context::capture(&project, options, || {}).unwrap()).unwrap(),
        context
    );
    assert_eq!(
        serde_json::to_vec(&narrative_preview::compile_project(&project, BTreeMap::new()).unwrap())
            .unwrap(),
        graph
    );
    assert_eq!(package_bytes(&project, &home.join("after-export")), package);
    drop(project);
    fs::remove_dir_all(home).unwrap();
}
