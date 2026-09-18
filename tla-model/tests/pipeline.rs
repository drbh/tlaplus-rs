use std::num::NonZeroUsize;
use tla_engine::{
    Completion, Options, TransitionSystem, explore, explore_parallel, validate_trace,
};
use tla_ir::{Builtin as B, Expr, Level, Literal, Program};
use tla_model::{BoundModel, Value};

fn counter() -> BoundModel {
    let mut p = Program {
        variables: vec!["x".into()],
        ..Program::default()
    };
    let x = p.push(
        Expr::Variable {
            slot: 0,
            primed: false,
        },
        Level::State,
    );
    let next = p.push(
        Expr::Variable {
            slot: 0,
            primed: true,
        },
        Level::Action,
    );
    let zero = p.push(Expr::Literal(Literal::Int(0)), Level::Constant);
    let one = p.push(Expr::Literal(Literal::Int(1)), Level::Constant);
    let three = p.push(Expr::Literal(Literal::Int(3)), Level::Constant);
    let init = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![x, zero],
        },
        Level::State,
    );
    let sum = p.push(
        Expr::Apply {
            op: B::Add,
            args: vec![x, one],
        },
        Level::State,
    );
    let modulo = p.push(
        Expr::Apply {
            op: B::Mod,
            args: vec![sum, three],
        },
        Level::State,
    );
    let action = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![next, modulo],
        },
        Level::Action,
    );
    let mut model = BoundModel::new(p);
    model.checks.init.push(init);
    model.checks.actions.push(("Next".into(), action));
    model
}

#[test]
fn new_evaluator_runs_identically_with_multiple_workers() {
    let model = counter();
    let options = Options {
        record_graph: true,
        ..Options::default()
    };
    let serial = explore(&model, &options).unwrap();
    assert_eq!(serial.completion, Completion::Complete);
    assert_eq!(serial.nodes.len(), 3);
    for workers in [1, 2, 4] {
        let parallel =
            explore_parallel(&model, &options, NonZeroUsize::new(workers).unwrap()).unwrap();
        assert_eq!(
            parallel.nodes.iter().map(|n| &n.state).collect::<Vec<_>>(),
            serial.nodes.iter().map(|n| &n.state).collect::<Vec<_>>()
        );
        assert_eq!(parallel.edges, serial.edges);
    }
}

#[test]
fn invariant_witness_and_constraint_pruning() {
    let mut model = counter();
    let two = model
        .program
        .push(Expr::Literal(Literal::Int(2)), Level::Constant);
    let invariant = model.program.push(
        Expr::Apply {
            op: B::Lt,
            args: vec![0, two],
        },
        Level::State,
    );
    model.checks.invariants.push(("Bound".into(), invariant));
    let result = explore(&model, &Options::default()).unwrap();
    let Completion::Violated(violation) = result.completion else {
        panic!("expected violation");
    };
    assert_eq!(
        violation
            .trace
            .states
            .iter()
            .map(|state| state.values.clone())
            .collect::<Vec<_>>(),
        vec![
            vec![Value::Int(0)],
            vec![Value::Int(1)],
            vec![Value::Int(2)]
        ]
    );
    assert!(validate_trace(&model, &violation).unwrap());
    model.checks.invariants.clear();
    model.checks.state_constraints.push(invariant);
    let result = explore(&model, &Options::default()).unwrap();
    assert_eq!(result.nodes.len(), 2);
    assert_eq!(result.completion, Completion::Complete);
}

#[test]
fn generating_membership_preserves_choices_and_requires_complete_states() {
    let mut model = counter();
    let set = model.program.push(
        Expr::Literal(Literal::Set(vec![Literal::Int(2), Literal::Int(4)])),
        Level::Constant,
    );
    let initial = model.program.push(
        Expr::Apply {
            op: B::In,
            args: vec![0, set],
        },
        Level::State,
    );
    model.checks.init = vec![initial];
    assert_eq!(
        model
            .initial_states()
            .unwrap()
            .into_iter()
            .map(|state| state.values)
            .collect::<Vec<_>>(),
        vec![vec![Value::Int(2)], vec![Value::Int(4)]]
    );
    model.checks.init.clear();
    assert!(model.initial_states().is_err());
}

#[test]
fn streaming_stops_before_later_branches_and_large_products() {
    use std::ops::ControlFlow;
    let mut model = counter();
    let zero = model
        .program
        .push(Expr::Literal(Literal::Int(0)), Level::Constant);
    let one = model
        .program
        .push(Expr::Literal(Literal::Int(1)), Level::Constant);
    let bad = model.program.push(
        Expr::Apply {
            op: B::Div,
            args: vec![one, zero],
        },
        Level::Constant,
    );
    let choice = model.program.push(
        Expr::Apply {
            op: B::Or,
            args: vec![model.checks.init[0], bad],
        },
        Level::State,
    );
    model.checks.init = vec![choice];
    assert!(model.initial_states().is_err());
    let mut seen = 0;
    assert!(
        model
            .visit_initial(&mut |_| {
                seen += 1;
                Ok(ControlFlow::Break(()))
            })
            .unwrap()
            .is_break()
    );
    assert_eq!(seen, 1);

    let mut p = Program {
        variables: vec!["a".into(), "b".into(), "c".into()],
        ..Program::default()
    };
    let domain = p.push(
        Expr::Literal(Literal::Set((0..1000).map(Literal::Int).collect())),
        Level::Constant,
    );
    let mut terms = Vec::new();
    for slot in 0..3 {
        let var = p.push(
            Expr::Variable {
                slot,
                primed: false,
            },
            Level::State,
        );
        terms.push(p.push(
            Expr::Apply {
                op: B::In,
                args: vec![var, domain],
            },
            Level::State,
        ));
    }
    let init = p.push(
        Expr::Apply {
            op: B::And,
            args: terms,
        },
        Level::State,
    );
    let mut product = BoundModel::new(p);
    product.checks.init = vec![init];
    let mut states = Vec::new();
    let _ = product
        .visit_initial(&mut |state| {
            states.push(state);
            Ok(if states.len() == 3 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            })
        })
        .unwrap();
    assert_eq!(states.len(), 3);
    assert_eq!(
        states[2].values,
        vec![Value::Int(0), Value::Int(0), Value::Int(2)]
    );
    for workers in [1, 2] {
        let report = explore_parallel(
            &product,
            &Options {
                max_states: NonZeroUsize::new(3),
                ..Options::default()
            },
            NonZeroUsize::new(workers).unwrap(),
        )
        .unwrap();
        assert!(matches!(report.completion, Completion::StateLimit));
        assert_eq!(report.nodes.len(), 3);
    }
}

#[test]
fn computed_constants_follow_dependencies_and_reject_cycles() {
    let mut program = Program {
        constants: vec!["A".into(), "B".into()],
        ..Program::default()
    };
    let a = program.push(Expr::Constant(0), Level::Constant);
    let b = program.push(Expr::Constant(1), Level::Constant);
    let one = program.push(Expr::Literal(Literal::Int(1)), Level::Constant);
    let sum = program.push(
        Expr::Apply {
            op: B::Add,
            args: vec![b, one],
        },
        Level::Constant,
    );
    assert_eq!(
        tla_model::materialize_constants(&program, &[(0, sum), (1, one)]).unwrap(),
        vec![Value::Int(2), Value::Int(1)]
    );
    assert!(tla_model::materialize_constants(&program, &[(0, b), (1, a)]).is_err());
    assert!(tla_model::materialize_constants(&program, &[(0, one)]).is_err());
}

#[test]
fn unused_string_operators_do_not_reject_parallel_exploration() {
    let mut model = counter();
    let text = model.program.push(
        Expr::Literal(Literal::String {
            ordinal: 1,
            text: "a".into(),
        }),
        Level::Constant,
    );
    model.program.push(
        Expr::Apply {
            op: B::Concat,
            args: vec![text, text],
        },
        Level::Constant,
    );
    assert!(explore_parallel(&model, &Options::default(), NonZeroUsize::new(2).unwrap()).is_ok());
    assert!(explore_parallel(&model, &Options::default(), NonZeroUsize::new(1).unwrap()).is_ok());
}

#[test]
fn constant_only_modules_have_no_implicit_empty_initial_state() {
    let model = BoundModel::new(Program::default());
    assert!(model.initial_states().unwrap().is_empty());
    assert!(
        explore(&model, &Options::default())
            .unwrap()
            .nodes
            .is_empty()
    );
    assert!(
        explore_parallel(&model, &Options::default(), NonZeroUsize::new(4).unwrap())
            .unwrap()
            .nodes
            .is_empty()
    );
}

#[test]
fn enabled_checks_partial_actions_without_applying_model_constraints() {
    let mut model = counter();
    let condition = model.program.push(
        Expr::Apply {
            op: B::Enabled,
            args: vec![model.checks.actions[0].1],
        },
        Level::State,
    );
    assert_eq!(
        model.evaluate(condition, &[Value::Int(0)], &[]).unwrap(),
        Value::Bool(true)
    );
    let false_id = model
        .program
        .push(Expr::Literal(Literal::Bool(false)), Level::Constant);
    model.checks.state_constraints.push(false_id);
    assert_eq!(
        model.evaluate(condition, &[Value::Int(0)], &[]).unwrap(),
        Value::Bool(true)
    );
    let disabled = model.program.push(
        Expr::Apply {
            op: B::Enabled,
            args: vec![false_id],
        },
        Level::State,
    );
    assert_eq!(
        model.evaluate(disabled, &[Value::Int(0)], &[]).unwrap(),
        Value::Bool(false)
    );
    let true_id = model
        .program
        .push(Expr::Literal(Literal::Bool(true)), Level::Constant);
    let partial = model.program.push(
        Expr::Apply {
            op: B::Enabled,
            args: vec![true_id],
        },
        Level::State,
    );
    assert_eq!(
        model.evaluate(partial, &[Value::Int(0)], &[]).unwrap(),
        Value::Bool(true)
    );
    let local = model.program.push(Expr::Local(0), Level::Constant);
    let assign = model.program.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![1, local],
        },
        Level::Action,
    );
    let enabled = model.program.push(
        Expr::Apply {
            op: B::Enabled,
            args: vec![assign],
        },
        Level::State,
    );
    model.program.operators.push(tla_ir::Operator {
        name: "Captured".into(),
        parameters: vec![0],
        body: enabled,
    });
    let call = model.program.push(
        Expr::Call {
            operator: 0,
            args: vec![true_id],
        },
        Level::State,
    );
    assert_eq!(
        model.evaluate(call, &[Value::Int(0)], &[]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn parallel_string_discovery_matches_serial_ordinals_from_fresh_tables() {
    let mut model = counter();
    model.program.variables.push("s".into());
    let p = &mut model.program;
    let text = p.push(
        Expr::Literal(Literal::String {
            ordinal: 1,
            text: "".into(),
        }),
        Level::Constant,
    );
    let s = p.push(
        Expr::Variable {
            slot: 1,
            primed: false,
        },
        Level::State,
    );
    let next_s = p.push(
        Expr::Variable {
            slot: 1,
            primed: true,
        },
        Level::Action,
    );
    let domain = p.push(
        Expr::Literal(Literal::Set(vec![
            Literal::Int(0),
            Literal::Int(1),
            Literal::Int(2),
        ])),
        Level::Constant,
    );
    let init_x = p.push(
        Expr::Apply {
            op: B::In,
            args: vec![0, domain],
        },
        Level::State,
    );
    let init_s = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![s, text],
        },
        Level::State,
    );
    model.checks.init = vec![init_x, init_s];
    let generated = p.push(
        Expr::Apply {
            op: B::ToString,
            args: vec![0],
        },
        Level::State,
    );
    let assign = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![next_s, generated],
        },
        Level::Action,
    );
    let next = p.push(
        Expr::Apply {
            op: B::And,
            args: vec![model.checks.actions[0].1, assign],
        },
        Level::Action,
    );
    model.checks.actions = vec![("Next".into(), next)];
    let fresh = || {
        let mut copy = model.clone();
        copy.runtime.strings =
            std::sync::Arc::new(std::sync::Mutex::new(copy.program.strings.clone()));
        copy
    };
    let serial = fresh();
    let expected = explore(&serial, &Options::default()).unwrap();
    for _ in 0..4 {
        let parallel = fresh();
        let actual = explore_parallel(
            &parallel,
            &Options::default(),
            NonZeroUsize::new(4).unwrap(),
        )
        .unwrap();
        assert_eq!(actual.completion, expected.completion);
        assert_eq!(
            actual
                .nodes
                .iter()
                .map(|n| (&n.state, n.depth, n.parent))
                .collect::<Vec<_>>(),
            expected
                .nodes
                .iter()
                .map(|n| (&n.state, n.depth, n.parent))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            *parallel.runtime.strings.lock().unwrap(),
            *serial.runtime.strings.lock().unwrap()
        );
    }
}
