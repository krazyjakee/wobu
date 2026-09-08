use wobu_narrative::{Beat, Choice, Destination, Outcome, Scene, StateSchema};
use wobu_narrative_compiler::{CompileOptions, Profile, Severity, Target, compile};
#[test]
fn unresolved_routes_never_lower_to_an_ending_or_emit_a_graph() {
    let mut scene = Scene::new("Disconnected");
    let mut beat = Beat::new("Question");
    beat.choices.push(Choice::new("Ask", Destination::Unresolved {}));
    beat.outcomes.push(Outcome::new(Destination::Unresolved {}));
    scene.beats.push(beat);
    assert!(Target::try_from(&Destination::Unresolved {}).is_err());
    for profile in [Profile::Development, Profile::Release] {
        let report = compile(
            &[scene.clone()],
            &StateSchema::default(),
            &CompileOptions { profile, ..Default::default() },
        );
        assert!(report.graph.is_none());
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error && d.message.contains("disconnected"))
                .count(),
            2
        );
    }
}
