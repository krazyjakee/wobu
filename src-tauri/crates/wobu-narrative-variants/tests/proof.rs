mod support;
use support::*;
use wobu_narrative::*;
use wobu_narrative_variants::*;
#[test]
fn twelve_configurations_have_exactly_seven_declared_model_witnesses() {
    let f = fixture();
    let report = analyze(f.input(), &f.policy).unwrap();
    assert!(report.complete, "{:?}", report.reasons);
    assert_eq!(
        (
            &report.potential.value,
            &report.included.value,
            &report.excluded.value,
            &report.unknown.value
        ),
        (&"12".into(), &"7".into(), &"5".into(), &"0".into())
    );
    let included: Vec<_> =
        report.rows.iter().filter(|r| r.classification == Classification::Included).collect();
    assert_eq!(included.len(), 7);
    let transitions = f.policy.validate(&f.input()).unwrap();
    for row in included {
        let witness = row.witness.as_ref().unwrap();
        assert!(!witness.runtime_route_verified);
        let mut state = f.policy.initial[witness.initial].clone();
        for source in &witness.transitions {
            let transition = transitions.iter().find(|t| &t.source == source).unwrap();
            assert!(evaluate::evaluate(&transition.when, &state).unwrap());
            for effect in &transition.effects {
                let (name, value) = evaluate::assignment(effect, &state).unwrap();
                state.insert(name, value);
            }
            assert!(f.policy.permits(&state).unwrap());
        }
        assert_eq!(state, witness.state);
        assert!(evaluate::evaluate(&row.when, &state).unwrap());
    }
}
#[test]
fn external_inputs_and_each_budget_keep_unexplored_rows_unknown() {
    let mut f = fixture();
    f.policy.external = true;
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.excluded.value, "0");
    assert_eq!(report.unknown.value, "5");
    f.policy.external = false;
    f.policy.limits.states = 3;
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.explored_states, 3);
    assert_eq!(report.excluded.value, "0");
    assert!(!report.complete);
    let report = analyze_with_stop(f.input(), &f.policy, || true).unwrap();
    assert_eq!(report.included.value, "0");
    assert_eq!(report.unknown.value, "12");
    assert_eq!(report.excluded.value, "0");
    f.policy.limits = Limits { variants: 2, ..Limits::default() };
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.included.value, "7");
    assert!(report.reasons.iter().any(|s| s.contains("Variant display")));
}
#[test]
fn initial_states_also_obey_the_concrete_state_bound() {
    let mut f = fixture();
    f.policy.quests.clear();
    f.policy.initial.push(config(true, "arrival", 0));
    f.policy.limits.states = 1;
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.explored_states, 1);
    assert_eq!(report.excluded.value, "0");
    assert!(!report.complete);
}
#[test]
fn stable_ids_ignore_unrelated_prose_and_policy_timing_settings() {
    let mut f = fixture();
    let before = analyze(f.input(), &f.policy).unwrap();
    f.scenes[0].name = "Renamed".into();
    f.scenes[0].summary = "Unrelated prose".into();
    f.policy.limits.milliseconds = 9000;
    let after = analyze(f.input(), &f.policy).unwrap();
    for (a, b) in before.rows.iter().zip(&after.rows) {
        assert_eq!(a.id, b.id);
        assert_eq!(
            candidate_id(&before.target, f.scenes[0].beats[0].dialogue[0].id, a),
            candidate_id(&after.target, f.scenes[0].beats[0].dialogue[0].id, b)
        );
    }
}
#[test]
fn overlap_reports_authored_first_match_and_explicit_fallback_and_missing_coverage() {
    let mut f = fixture();
    let report = analyze(f.input(), &f.policy).unwrap();
    let row = report.rows.iter().find(|r| r.values == config(true, "hearing", 1)).unwrap();
    let coverage = &row.coverage[0];
    assert_eq!(coverage.matching.len(), 4);
    assert_eq!(coverage.selected, Some(f.scenes[0].beats[0].dialogue[0].variants[0].id));
    assert_eq!(coverage.fallback, Some(f.scenes[0].beats[0].dialogue[0].variants[3].id));
    f.scenes[0].beats[0].dialogue[0].variants.pop();
    let report = analyze(f.input(), &f.policy).unwrap();
    let row = report.rows.iter().find(|r| r.values == config(false, "arrival", 0)).unwrap();
    assert_eq!(row.coverage[0].selected, None);
    assert_eq!(row.coverage[0].fallback, None);
}
#[test]
fn authored_effects_cannot_be_silently_omitted_from_an_exclusion_proof() {
    let mut f = fixture();
    let mut outcome = Outcome::new(Destination::End { label: String::new() });
    outcome.effects.push(Effect::Set(Assignment {
        var: name("evidence"),
        value: Operand::Literal(Value::Int(1)),
    }));
    f.scenes[0].beats[0].outcomes.push(outcome);
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.excluded.value, "0");
    assert!(report.reasons.iter().any(|r| r.contains("Unmodelled authored effect")));
}
#[test]
fn rhs_variables_and_integer_steps_are_exact_not_representative_samples() {
    let mut f = fixture();
    let mut declarations: Vec<_> = f.schema.iter().cloned().collect();
    declarations.push(VariableDecl {
        name: name("threshold"),
        ty: VarType::Int { min: 0, max: 1 },
        default: Value::Int(0),
        owner: Owner::Host,
        description: String::new(),
    });
    f.schema = StateSchema::new(declarations).unwrap();
    f.policy.initial[0].insert(name("threshold"), Value::Int(0));
    f.scenes[0].entry = Some(Condition::Compare(Comparison {
        var: name("evidence"),
        op: CompareOp::Ge,
        value: Operand::Var(name("threshold")),
    }));
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.potential.value, "24");
    assert!(report.domains.iter().any(|d| d.name == name("threshold")));
    assert_eq!(report.excluded.value, "0");
}
#[test]
fn corrupt_witness_paths_values_and_ids_do_not_authorize_structural_materialization() {
    let f = fixture();
    let report = analyze(f.input(), &f.policy).unwrap();
    let row = report
        .rows
        .iter()
        .find(|r| r.witness.as_ref().is_some_and(|w| !w.transitions.is_empty()))
        .unwrap();
    verify_witness(f.input(), &f.policy, row).unwrap();
    let mut tampered = row.clone();
    tampered.witness.as_mut().unwrap().transitions.push("invented runtime route".into());
    assert!(verify_witness(f.input(), &f.policy, &tampered).is_err());
    let mut tampered = row.clone();
    tampered.values.insert(name("permit"), Value::Bool(false));
    tampered.id = "collision".into();
    assert!(verify_witness(f.input(), &f.policy, &tampered).is_err());
    let mut tampered = row.clone();
    tampered.witness.as_mut().unwrap().runtime_route_verified = true;
    assert!(verify_witness(f.input(), &f.policy, &tampered).is_err());
}
#[test]
fn retained_witness_budget_bounds_report_work_without_falsifying_summary_counts() {
    let mut f = fixture();
    f.policy.limits.witness_steps = 0;
    let report = analyze(f.input(), &f.policy).unwrap();
    assert_eq!(report.included.value, "7");
    assert_eq!(report.excluded.value, "5");
    assert!(report.complete);
    assert!(
        report.rows.iter().filter(|r| r.classification == Classification::Included).count() < 7
    );
    assert!(report.reasons.iter().any(|r| r.contains("witness-step limit")));
    assert_eq!(
        report
            .rows
            .iter()
            .filter_map(|r| r.witness.as_ref())
            .map(|w| w.transitions.len())
            .sum::<usize>(),
        0
    );
}
#[test]
fn incoming_target_gates_are_relevant_and_nonentry_route_claims_stay_unknown() {
    let mut f = fixture();
    let mut declarations: Vec<_> = f.schema.iter().cloned().collect();
    declarations.push(VariableDecl {
        name: name("door"),
        ty: VarType::Bool,
        default: Value::Bool(false),
        owner: Owner::Narrative,
        description: String::new(),
    });
    f.schema = StateSchema::new(declarations).unwrap();
    f.policy.initial[0].insert(name("door"), Value::Bool(false));
    let mut entry = Beat::new("Door");
    let mut choice = Choice::new("Enter", Destination::Beat(f.policy.target.beat));
    choice.requires = Some(eq("door", Value::Bool(true)));
    entry.choices.push(choice);
    f.scenes[0].beats.insert(0, entry);
    let report = analyze(f.input(), &f.policy).unwrap();
    assert!(report.domains.iter().any(|d| d.name == name("door")));
    assert_eq!(report.excluded.value, "0");
    assert!(report.reasons.iter().any(|r| r.contains("incoming route feasibility")));
}
