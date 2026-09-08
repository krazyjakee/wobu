use super::*;
use wobu_narrative::{SceneDocument, StateDocument};

struct Fixture {
    root: std::path::PathBuf,
    project: Project,
    scenario: Scenario,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("wobu-scenario-{}", wobu_core::new_id()));
        let mut project = Project::create(&root, "Harbor Watch").unwrap();
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/narrative/harbor-watch");
        let scene =
            SceneDocument::parse(&std::fs::read_to_string(source.join("scene.yaml")).unwrap())
                .unwrap()
                .scene;
        let mut file = wobu_store::SceneFile {
            rel: format!("narrative/scenes/{}.yaml", scene.id),
            scene,
            stamp: None,
        };
        project.save_scene(&mut file).unwrap();
        let state =
            StateDocument::parse(&std::fs::read_to_string(source.join("state.yaml")).unwrap())
                .unwrap();
        project.save_state(&state, None).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(
            &std::fs::read(source.join("scenarios/00000000000000000000000020.json")).unwrap(),
        )
        .unwrap();
        let scenario = serde_json::from_value(doc["payload"].clone()).unwrap();
        Self { root, project, scenario }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn persists_named_scenario_replays_pending_restore_and_never_updates_world_canon() {
    let mut f = Fixture::new();
    let saved =
        save(&mut f.project, None, "Low trust witness".into(), f.scenario.clone(), None).unwrap();
    assert!(saved.stamp.is_some());
    assert_eq!(list(&f.project).unwrap()[0].scenario, f.scenario);
    let before = f.project.narrative_fingerprint().unwrap();
    let state = std::fs::read(f.project.root().join("narrative/state.yaml")).unwrap();
    let scenario_path = f.project.root().join(format!("narrative/scenarios/{}.json", saved.id));
    let record = std::fs::read(&scenario_path).unwrap();
    let result = run_saved(&f.project, saved.id).unwrap();
    assert!(result.report.unwrap().passed);
    assert_eq!(before, f.project.narrative_fingerprint().unwrap());
    assert_eq!(state, std::fs::read(f.project.root().join("narrative/state.yaml")).unwrap());
    assert_eq!(record, std::fs::read(scenario_path).unwrap());
}
#[test]
fn guarded_edit_conflicts_and_invalid_versions_or_unknown_fields_never_overwrite() {
    let mut f = Fixture::new();
    let saved = save(&mut f.project, None, "Original".into(), f.scenario.clone(), None).unwrap();
    let newer = save(
        &mut f.project,
        Some(saved.id),
        "Other writer".into(),
        f.scenario.clone(),
        saved.stamp.clone(),
    )
    .unwrap();
    assert!(
        save(&mut f.project, Some(saved.id), "Stale".into(), f.scenario.clone(), saved.stamp)
            .is_err()
    );
    let before =
        std::fs::read(f.project.root().join(format!("narrative/scenarios/{}.json", saved.id)))
            .unwrap();
    let mut future = f.scenario.clone();
    future.version = 999;
    assert!(save(&mut f.project, Some(saved.id), "Future".into(), future, newer.stamp).is_err());
    assert_eq!(
        before,
        std::fs::read(f.project.root().join(format!("narrative/scenarios/{}.json", saved.id)))
            .unwrap()
    );
    let mut unknown = serde_json::to_value(&f.scenario).unwrap();
    unknown["extra"] = true.into();
    assert!(serde_json::from_value::<Scenario>(unknown).is_err());
    let mut document: serde_json::Value = serde_json::from_slice(&before).unwrap();
    document["payload"]["version"] = 999.into();
    std::fs::write(
        f.project.root().join(format!("narrative/scenarios/{}.json", saved.id)),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    assert!(list(&f.project).is_err());
    assert!(run_saved(&f.project, saved.id).is_err());
}
#[test]
fn changed_consequence_returns_precise_failure_and_bad_transport_numbers_are_rejected() {
    let mut f = Fixture::new();
    let saved = save(&mut f.project, None, "Witness".into(), f.scenario.clone(), None).unwrap();
    let mut file = f.project.load_scene(f.scenario.scene).unwrap();
    let wobu_narrative::Effect::Add(increment) = &mut file.scene.beats[0].choices[0].effects[0]
    else {
        panic!()
    };
    increment.by = 6;
    f.project.save_scene(&mut file).unwrap();
    let result = run_saved(&f.project, saved.id).unwrap().report.unwrap().divergence.unwrap();
    assert_eq!(result.step, 3);
    assert_eq!(result.field, "state.trust");
    assert_eq!(result.site.choice, Some(file.scene.beats[0].choices[0].id.to_string()));
    let mut invalid = f.scenario.clone();
    invalid.seed = 9_007_199_254_740_992;
    assert!(save(&mut f.project, None, "Unsafe transport".into(), invalid, None).is_err());
}
