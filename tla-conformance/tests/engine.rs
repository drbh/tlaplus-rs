use std::fs;
use std::path::Path;

use tla_bridge::{
    Answers, Comparison, Mode, ModelRequest, Outcome, RunEnv, compare, observe_faithful, with_model,
};
use tla_engine::{Completion, Options, ViolationKind, explore, validate_trace};

fn fixture(module: &str, spec: &str, cfg: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(format!("{module}.tla")), spec).unwrap();
    fs::write(dir.path().join(format!("{module}.cfg")), cfg).unwrap();
    dir
}

fn compare_model(dir: &Path, module: &str) -> Answers {
    let answers = with_model(
        ModelRequest::new(dir.join(format!("{module}.tla"))),
        |model| {
            let system = model.system()?;
            let report = explore(
                &system,
                &Options {
                    check_deadlock: model.info().check_deadlock,
                    ..Options::default()
                },
            )?;
            let outcome = match &report.completion {
                Completion::Complete => Outcome::Success,
                Completion::Violated(violation) => {
                    assert!(validate_trace(&system, violation)?);
                    let values = violation
                        .trace
                        .states
                        .iter()
                        .map(|s| model.state_values(s))
                        .collect::<Result<Vec<_>, _>>()?;
                    let encoded = serde_json::to_string(&values).unwrap();
                    let decoded: Vec<Vec<tla_bridge::checker::ValueKey>> =
                        serde_json::from_str(&encoded).unwrap();
                    assert_eq!(values, decoded);
                    match &violation.kind {
                        ViolationKind::Invariant(_) => Outcome::InvariantViolated,
                        ViolationKind::InitialProperty(_) => Outcome::InitialPropertyViolated,
                        ViolationKind::ActionProperty(_) => Outcome::ActionPropertyViolated,
                        ViolationKind::Deadlock => Outcome::Deadlock,
                    }
                }
                Completion::StateLimit => panic!("unexpected limit"),
            };
            Ok(Answers {
                mode: Mode::Bfs,
                outcome,
                initial_distinct: Some(report.statistics.initial_distinct as u64),
                distinct: Some(report.nodes.len() as u64),
                depth: Some(report.statistics.depth as u64),
            })
        },
    )
    .unwrap();
    let observed = observe_faithful(
        vec![
            "-workers".into(),
            "1".into(),
            "-seed".into(),
            "1".into(),
            "-fp".into(),
            "0".into(),
            "-noGenerateSpecTE".into(),
            "-metadir".into(),
            dir.join("states").to_string_lossy().into(),
            module.into(),
        ],
        RunEnv {
            user_dir: dir.into(),
            max_heap_bytes: Some(64 << 20),
            max_direct_memory_bytes: Some(16 << 20),
            ..RunEnv::default()
        },
    )
    .unwrap();
    if let Some(Ok(states)) = &observed.typed_states {
        let kind = if observed.deadlock_reported {
            Some(ViolationKind::Deadlock)
        } else {
            observed
                .properties
                .first()
                .and_then(|p| match p.property.kind {
                    tla_bridge::PropertyKind::Invariant => {
                        Some(ViolationKind::Invariant(p.property.name.clone()))
                    }
                    tla_bridge::PropertyKind::Initial => {
                        Some(ViolationKind::InitialProperty(p.property.name.clone()))
                    }
                    tla_bridge::PropertyKind::Action => {
                        Some(ViolationKind::ActionProperty(p.property.name.clone()))
                    }
                    _ => None,
                })
        };
        if let Some(kind) = kind {
            let states = states.clone();
            with_model(
                ModelRequest::new(dir.join(format!("{module}.tla"))),
                move |model| {
                    assert!(
                        model.validate_safety_records(&states, &kind)?,
                        "{kind:?}: {states:?}"
                    );
                    Ok(())
                },
            )
            .unwrap();
        }
    }
    let faithful = observed.summary;
    assert_eq!(
        compare(&faithful, &answers),
        Comparison::Match,
        "faithful={faithful:?}, engine={answers:?}"
    );
    answers
}

#[test]
fn engine_matches_faithful_safety_verdicts_and_validates_actual_traces() {
    for cfg in [
        "INIT Init\nNEXT Next\nINVARIANT Safe",
        "INIT Init\nNEXT Next\nINVARIANT Broken",
        "INIT Init\nNEXT Next\nINVARIANT InitiallyBroken",
        "INIT Init\nNEXT Next\nPROPERTY InitiallyBroken",
        "SPECIFICATION Spec\nPROPERTY NoChange",
        "INIT Init\nNEXT Stop",
    ] {
        let dir = fixture(
            "Counter",
            include_str!("../../tests/fixtures/Counter.tla"),
            cfg,
        );
        compare_model(dir.path(), "Counter");
    }
}

#[test]
fn symmetry_view_and_constraints_preserve_the_faithful_state_quotient() {
    let cfg = "INIT Init\nNEXT Next\nCONSTANT Limit = 99\nCONSTANT Bound <- Ceiling\nCONSTANT Labels = {a, b, c}\nINVARIANT TypeOK\n";
    for (extra, count, initial) in [
        ("", 9, 6),
        ("SYMMETRY Sym", 3, 2),
        ("VIEW View", 3, 2),
        ("SYMMETRY Sym\nVIEW View", 3, 2),
        ("CONSTRAINT KeepState", 3, 3),
        ("ACTION_CONSTRAINT KeepStep", 9, 6),
    ] {
        let dir = fixture(
            "Choices",
            include_str!("../../tests/fixtures/Choices.tla"),
            &format!("{cfg}{extra}"),
        );
        let answers = compare_model(dir.path(), "Choices");
        assert_eq!(answers.distinct, Some(count));
        assert_eq!(answers.initial_distinct, Some(initial));
    }
}

#[test]
fn exact_keys_unify_extensionally_equal_functions_but_preserve_value_types() {
    let spec = "---- MODULE Keys ----\nEXTENDS Naturals, TLC\nCONSTANT m\nVARIABLE v\nInit == v = <<1, 2>> \\/ v = [i \\in 1..2 |-> i] \\/ v = [a |-> 1] \\/ v = \"m\" \\/ v = m\nNext == UNCHANGED v\n====";
    let dir = fixture("Keys", spec, "INIT Init\nNEXT Next\nCONSTANT m = m");
    let answers = compare_model(dir.path(), "Keys");
    assert_eq!(answers.distinct, Some(4));
}

#[test]
fn safety_adapter_rejects_liveness_instead_of_reporting_success() {
    let dir = fixture(
        "Counter",
        include_str!("../../tests/fixtures/Counter.tla"),
        "SPECIFICATION Spec\nPROPERTY EventuallyAlwaysZero",
    );
    with_model(ModelRequest::new(dir.path().join("Counter.tla")), |model| {
        assert!(model.system().is_err());
        Ok(())
    })
    .unwrap();
}

#[test]
fn faithful_capture_preserves_original_states_under_alias_and_resets() {
    use tla_bridge::{checker::ValueKey, observe_faithful};
    for (property, expected) in [
        ("BadInit", vec![0]),
        ("BadNext", vec![0, 1]),
        ("Good", vec![]),
    ] {
        let dir = fixture(
            "Capture",
            "---- MODULE Capture ----\nEXTENDS Naturals\nVARIABLE x\nInit == x = 0\nNext == x' = 1\nBadInit == x > 0\nBadNext == x = 0\nGood == x <= 1\nAlias == [hidden |-> 99 + x]\n====",
            &format!("INIT Init\nNEXT Next\nINVARIANT {property}\nALIAS Alias"),
        );
        let observed = observe_faithful(
            vec![
                "-workers".into(),
                "1".into(),
                "-noGenerateSpecTE".into(),
                "-metadir".into(),
                dir.path().join("states").to_string_lossy().into(),
                "Capture".into(),
            ],
            RunEnv {
                user_dir: dir.path().into(),
                max_heap_bytes: Some(64 << 20),
                max_direct_memory_bytes: Some(16 << 20),
                ..RunEnv::default()
            },
        )
        .unwrap();
        let states = observed.typed_states.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(states.len(), expected.len(), "{observed:?}");
        for ((ordinal, state), (index, value)) in
            states.iter().zip(expected.into_iter().enumerate())
        {
            assert_eq!(*ordinal, index as i64 + 1);
            assert_eq!(
                *state,
                ValueKey::Function(vec![(ValueKey::String("x".into()), ValueKey::Int(value))])
            );
        }
        for trace in &observed.counterexamples {
            assert!(
                trace
                    .states
                    .iter()
                    .all(|state| state.display.contains("hidden"))
            );
        }
        if !states.is_empty() {
            let states = states.clone();
            let kind = ViolationKind::Invariant(property.into());
            with_model(
                ModelRequest::new(dir.path().join("Capture.tla")),
                move |model| {
                    assert!(model.validate_safety_records(&states, &kind)?);
                    let mut corrupt = states.clone();
                    corrupt[0].1 = ValueKey::Function(vec![]);
                    assert!(!model.validate_safety_records(&corrupt, &kind)?);
                    corrupt = states.clone();
                    corrupt[0].0 = 2;
                    assert!(!model.validate_safety_records(&corrupt, &kind)?);
                    assert!(!model.validate_safety_records(
                        &states,
                        &ViolationKind::Invariant("Good".into())
                    )?);
                    Ok(())
                },
            )
            .unwrap();
        }
        let json = serde_json::to_string(&observed).unwrap();
        assert_eq!(observed, serde_json::from_str(&json).unwrap());
    }
}

#[test]
fn first_violation_precedes_later_deadlock_or_evaluation_error() {
    for next in [
        "x = 0 /\\ x' = 2",
        "IF x = 0 THEN x' = 2 ELSE x' = 1 \\div 0",
    ] {
        let directory = fixture(
            "Early",
            &format!(
                "---- MODULE Early ----\nEXTENDS Integers\nVARIABLE x\nInit == x \\in {{0, 1}}\nNext == {next}\nSafe == x < 2\n====\n"
            ),
            "INIT Init\nNEXT Next\nINVARIANT Safe\n",
        );
        assert_eq!(
            compare_model(directory.path(), "Early").outcome,
            Outcome::InvariantViolated
        );
    }
}

#[test]
fn faithful_visitors_stop_within_generators_and_preserve_callback_errors() {
    use std::ops::ControlFlow;
    use tla_bridge::model::{ModelError, Stage};
    use tla_engine::TransitionSystem;
    let spec = r#"---- MODULE Streaming ----
EXTENDS Naturals, TLC
VARIABLE x
Init == \E i \in 0..1:
    IF i = 0 THEN x = 0 ELSE Assert(FALSE, "late init") /\ x = 1
Next == \E i \in 1..2:
    IF i = 1 THEN x' = 1 ELSE Assert(FALSE, "late next") /\ x' = 2
Safe == x = 0
===="#;
    let dir = fixture("Streaming", spec, "INIT Init\nNEXT Next\nINVARIANT Safe");
    with_model(
        ModelRequest::new(dir.path().join("Streaming.tla")),
        |model| {
            let system = model.system()?;
            let mut first = None;
            assert!(
                system
                    .visit_initial(&mut |state| {
                        first = Some(state);
                        Ok(ControlFlow::Break(()))
                    })?
                    .is_break()
            );
            let first = first.unwrap();
            let mut count = 0;
            assert!(
                system
                    .visit_successors(&first, &mut |transition| {
                        count += 1;
                        assert_eq!(transition.state.level(), 2);
                        assert_eq!(system.invariant(&transition.state)?, Some("Safe".into()));
                        Ok(ControlFlow::Break(()))
                    })?
                    .is_break()
            );
            assert_eq!(count, 1);
            use tla_bridge::checker::ValueKey;
            let record =
                |x| ValueKey::Function(vec![(ValueKey::String("x".into()), ValueKey::Int(x))]);
            assert!(model.validate_safety_records(
                &[(1, record(0)), (2, record(1))],
                &ViolationKind::Invariant("Safe".into()),
            )?);
            let expected = ModelError {
                stage: Stage::Properties,
                code: None,
                message: "callback failed".into(),
            };
            assert_eq!(
                system.visit_initial(&mut |_| Err(expected.clone())),
                Err(expected.clone())
            );
            assert_eq!(
                system.visit_successors(&first, &mut |_| Err(expected.clone())),
                Err(expected)
            );
            assert!(
                system
                    .initial_states()
                    .err()
                    .unwrap()
                    .message
                    .contains("late init")
            );
            assert!(
                system
                    .successors(&first)
                    .err()
                    .unwrap()
                    .message
                    .contains("late next")
            );
            Ok(())
        },
    )
    .unwrap();
}
