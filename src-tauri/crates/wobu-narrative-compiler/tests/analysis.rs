use wobu_narrative::{Destination, Outcome};
use wobu_narrative_compiler::*;
use wobu_narrative_variants::example::fixture;
#[test]
fn declared_first_match_overlaps_warn_and_reachable_gaps_block_compilation() {
    let mut f = fixture();
    f.scenes[0].beats[0].outcomes.push(Outcome::new(Destination::End { label: "Done".into() }));
    let options = CompileOptions::default();
    let report = compile_with_analysis(
        &f.scenes,
        &[],
        &f.schema,
        &options,
        &f.world,
        std::slice::from_ref(&f.policy),
    );
    assert!(report.graph.is_some(), "{:?}", report.diagnostics);
    assert!(report.diagnostics.iter().any(|d|d.code=="overlapping_variants" && d.message.contains("Authored first-match")));
    f.scenes[0].beats[0].dialogue[0].variants.pop();
    let report = compile_with_analysis(&f.scenes, &[], &f.schema, &options, &f.world, &[f.policy]);
    assert!(report.graph.is_none());
    assert!(report.diagnostics.iter().any(|d| d.code == "uncovered_configuration"));
}
#[test]
fn no_policy_retains_identical_graph_and_display_limits_cannot_hide_known_gaps() {
    let mut f = fixture();
    f.scenes[0].beats[0].outcomes.push(Outcome::new(Destination::End { label: "Done".into() }));
    let options = CompileOptions::default();
    assert_eq!(
        compile(&f.scenes, &[], &f.schema, &options),
        compile_with_analysis(&f.scenes, &[], &f.schema, &options, &f.world, &[])
    );
    f.policy.limits.variants = 1;
    let report = compile_with_analysis(&f.scenes, &[], &f.schema, &options, &f.world, &[f.policy]);
    assert!(report.graph.is_none());
    assert!(report.diagnostics.iter().any(|d| d.code == "analysis_coverage_limit"));
}
