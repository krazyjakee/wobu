use wobu_narrative::{SceneCatalog, SceneDocument, SourcePathPart, StateDocument};

fn parts(path: &[SourcePathPart]) -> Vec<String> {
    path.iter()
        .map(|part| match part {
            SourcePathPart::Key(key) => key.clone(),
            SourcePathPart::Index(index) => index.to_string(),
        })
        .collect()
}
#[test]
fn semantic_paths_distinguish_nested_conditions_effects_destinations_and_duplicate_ids() {
    let source = r#"schema_version: 1
scene:
  id: 01J00000000000000000000001
  name: Council
  entry: {not: {compare: {var: missing_entry, op: eq, value: {literal: true}}}}
  beats:
    - id: 01J00000000000000000000002
      title: Evidence
      choices:
        - id: 01J00000000000000000000003
          label: Ask
          requires: {any: [always, {compare: {var: missing_gate, op: eq, value: {literal: true}}}]}
          effects:
            - add: {var: missing_effect, by: 1}
            - set: {var: known, value: {literal: 12}}
            - command: {name: journal_add, args: [{literal: true}, {var: missing_arg}]}
          to: {beat: 01J00000000000000000000009}
    - id: 01J00000000000000000000002
      title: Duplicate
"#;
    let schema = StateDocument::parse(
        "schema_version: 1\nvariables:\n- {name: known, type: bool, default: false}\n",
    )
    .unwrap()
    .schema()
    .unwrap();
    let scene = SceneDocument::parse(source).unwrap().scene;
    let catalog = SceneCatalog::unknown();
    let located = scene.source_diagnostics(&schema, &catalog);
    assert_eq!(
        located.iter().map(|(diagnostic, _)| diagnostic).collect::<Vec<_>>(),
        scene.diagnostics(&schema, &catalog).iter().collect::<Vec<_>>()
    );
    let paths = located.iter().map(|(_, path)| parts(path).join("/")).collect::<Vec<_>>();
    for expected in [
        "scene/entry/not/compare/var",
        "scene/beats/0/choices/0/requires/any/1/compare/var",
        "scene/beats/0/choices/0/effects/0/add/var",
        "scene/beats/0/choices/0/effects/1/set/value/literal",
        "scene/beats/0/choices/0/effects/2/command/args/1/var",
        "scene/beats/0/choices/0/to/beat",
        "scene/beats/1/id",
    ] {
        assert!(paths.iter().any(|path| path == expected), "missing {expected}: {paths:?}");
    }
}
