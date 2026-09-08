#[path = "../examples/harbor_voices_fixture.rs"]
mod example;

#[test]
fn original_example_has_real_speakers_sources_and_ready_context_for_every_type() {
    let temp = tempfile::tempdir().unwrap();
    let project = example::create(temp.path()).unwrap();
    assert_eq!(
        project
            .list_nodes()
            .unwrap()
            .iter()
            .filter(|node| node.kind == wobu_core::NodeKind::Character)
            .count(),
        3
    );
    let world = project.world_document().unwrap().unwrap().0;
    assert_eq!(world.quests.len(), 1);
    let assets = project.text_assets().unwrap();
    assert_eq!(assets.len(), 6);
    let schema = project.state_schema().unwrap();
    let mut state = schema
        .iter()
        .map(|decl| (decl.name.clone(), decl.default.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    state.insert(
        "knowledge".parse().unwrap(),
        wobu_narrative::Value::Enum("witnessed".parse().unwrap()),
    );
    state.insert("trust".parse().unwrap(), wobu_narrative::Value::Int(70));
    state.insert("recorded".parse().unwrap(), wobu_narrative::Value::Bool(true));
    for asset in assets {
        let scene = asset.editorial_scene();
        let beat = &scene.beats[0];
        let slot = &beat.dialogue[0];
        let context = wobu_store::project::narrative_context::capture(
            &project,
            wobu_narrative_context::Options {
                selection: wobu_narrative_context::Selection {
                    scene: scene.id,
                    beat: beat.id,
                    slot: slot.id,
                    variant: Some(slot.variants[0].id),
                },
                state: state.clone(),
                token_budget: 10000,
            },
            || {},
        )
        .unwrap();
        assert!(context.ready, "{}: {:?}", asset.name, context.diagnostics);
        assert!(context.fragments.iter().any(|fragment| fragment.kind == "linked_quest"));
        let view = project.review_scene(scene.id, None).unwrap();
        assert!(view.lines.iter().all(|line| !line.reason.contains("missing")), "{}", asset.name);
    }
}
