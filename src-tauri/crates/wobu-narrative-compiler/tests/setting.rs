//! #206. A scene's place reaches the graph, and an unplaceable scene does not.

use std::collections::BTreeSet;

use wobu_narrative::{Beat, Destination, EntityId, Outcome, Scene, StateSchema};
use wobu_narrative_compiler::{CompileOptions, Profile, Severity, compile};

/// One finished beat, so nothing but the setting can be the reason a compile
/// fails.
fn scene(name: &str) -> Scene {
    let mut beat = Beat::new("Only beat");
    beat.outcomes.push(Outcome::new(Destination::End { label: "Done".into() }));
    let mut scene = Scene::new(name);
    scene.beats.push(beat);
    scene
}

fn report(scene: &Scene, settings: BTreeSet<EntityId>, profile: Profile) -> (bool, Vec<String>) {
    let compiled = compile(
        std::slice::from_ref(scene),
        &[],
        &StateSchema::default(),
        &CompileOptions { profile, known_settings: settings, ..Default::default() },
    );
    let blockers = compiled
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code.clone())
        .collect();
    (compiled.graph.is_some(), blockers)
}

#[test]
fn a_stated_setting_reaches_the_compiled_scene_so_no_host_parses_a_summary() {
    let diner = EntityId::generate();
    let mut placed = scene("A shift at the diner");
    placed.setting_id = Some(diner);
    placed.summary = "Rosa puts her to work.".into();

    let compiled = compile(
        &[placed.clone()],
        &[],
        &StateSchema::default(),
        &CompileOptions { known_settings: BTreeSet::from([diner]), ..Default::default() },
    );
    let graph = compiled.graph.expect("a placed scene must compile");
    assert_eq!(
        graph.scenes[&placed.id.to_string()].setting.as_deref(),
        Some(diner.to_string().as_str())
    );
    // The summary is not consulted and is nowhere in the graph.
    assert!(!String::from_utf8_lossy(&graph.canonical_bytes()).contains("Rosa puts her to work"));
}

#[test]
fn a_setting_that_does_not_resolve_blocks_the_compile_at_every_profile() {
    // Unlike missing text, which is a Development task: a scene that cannot be
    // placed has no defensible runtime meaning, so it is an error either way.
    let mut misplaced = scene("Misplaced");
    misplaced.setting_id = Some(EntityId::generate());
    for profile in [Profile::Development, Profile::Release] {
        let (compiled, blockers) = report(&misplaced, BTreeSet::new(), profile);
        assert!(!compiled, "{profile:?} compiled a scene that names no real place");
        assert!(blockers.contains(&"unknown_setting".to_string()), "{blockers:?}");
    }
}

#[test]
fn a_project_that_states_no_settings_compiles_to_the_graph_it_always_did() {
    // The guarantee existing saves, packages and scenario tapes depend on: the
    // new field is skipped when absent, so the hash does not move.
    let unplaced = scene("Unplaced");
    let (compiled, blockers) = report(&unplaced, BTreeSet::new(), Profile::Development);
    assert!(compiled && blockers.is_empty(), "{blockers:?}");

    let graph = compile(&[unplaced], &[], &StateSchema::default(), &CompileOptions::default())
        .graph
        .unwrap();
    assert!(graph.scenes.values().all(|scene| scene.setting.is_none()));
    assert!(!String::from_utf8_lossy(&graph.canonical_bytes()).contains("setting"));
}
