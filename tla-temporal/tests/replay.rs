use std::num::NonZeroUsize;
use tla_engine::{Options, explore};
use tla_ir::{Builtin as B, Expr, Level as L, Literal, Program};
use tla_model::BoundModel;
use tla_props::{Check, Formula};
use tla_temporal::behavior::{Atom, build, validate};

fn model() -> (BoundModel, Vec<Atom>) {
    let mut p = Program::default();
    p.variables.push("x".into());
    let x = p.push(
        Expr::Variable {
            slot: 0,
            primed: false,
        },
        L::State,
    );
    let y = p.push(
        Expr::Variable {
            slot: 0,
            primed: true,
        },
        L::Action,
    );
    let zero = p.push(Expr::Literal(Literal::Int(0)), L::Constant);
    let one = p.push(Expr::Literal(Literal::Int(1)), L::Constant);
    let init = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![x, zero],
        },
        L::State,
    );
    let next = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![y, one],
        },
        L::Action,
    );
    let is_one = p.push(
        Expr::Apply {
            op: B::Eq,
            args: vec![x, one],
        },
        L::State,
    );
    let changed = p.push(
        Expr::Apply {
            op: B::Ne,
            args: vec![x, y],
        },
        L::Action,
    );
    let taken = p.push(
        Expr::Apply {
            op: B::And,
            args: vec![next, changed],
        },
        L::Action,
    );
    let mut model = BoundModel::new(p);
    model.checks.init = vec![init];
    model.checks.actions = vec![("Next".into(), next)];
    (
        model,
        vec![
            Atom {
                predicate: is_one,
                enabled: false,
            },
            Atom {
                predicate: taken,
                enabled: true,
            },
            Atom {
                predicate: taken,
                enabled: false,
            },
        ],
    )
}

#[test]
fn replays_stuttering_counterexample_and_checks_fairness() {
    let (model, atoms) = model();
    let report = explore(
        &model,
        &Options {
            record_graph: true,
            ..Options::default()
        },
    )
    .unwrap();
    let graph = build(&model, &report, &atoms).unwrap();
    let property = Formula::atom(0).eventually();
    let limit = NonZeroUsize::new(1000).unwrap();
    let Check::Violated { lasso, .. } = tla_props::check(&graph, &[], &property, limit).unwrap()
    else {
        panic!("stuttering violates eventuality")
    };
    assert!(validate(&model, &report, &graph, &lasso, &atoms, &[], &property).unwrap());
    let fairness = Formula::weak_fairness(1, 2);
    assert!(
        !validate(
            &model,
            &report,
            &graph,
            &lasso,
            &atoms,
            std::slice::from_ref(&fairness),
            &property
        )
        .unwrap()
    );
    assert!(matches!(
        tla_props::check(&graph, &[fairness], &property, limit).unwrap(),
        Check::Holds { .. }
    ));
}

#[test]
fn forged_labels_and_states_fail_evaluator_replay() {
    let (model, atoms) = model();
    let mut report = explore(
        &model,
        &Options {
            record_graph: true,
            ..Options::default()
        },
    )
    .unwrap();
    let mut graph = build(&model, &report, &atoms).unwrap();
    let property = Formula::atom(0).always();
    let Check::Violated { lasso, .. } =
        tla_props::check(&graph, &[], &property, NonZeroUsize::new(1000).unwrap()).unwrap()
    else {
        panic!("initial x is zero")
    };
    assert!(validate(&model, &report, &graph, &lasso, &atoms, &[], &property).unwrap());
    let id = lasso.cycle[0];
    graph.edges[id].atoms.insert(999);
    assert!(!validate(&model, &report, &graph, &lasso, &atoms, &[], &property).unwrap());
    graph.edges[id].atoms.remove(&999);
    report.nodes[lasso.initial].state[0] = tla_model::Value::Int(77);
    assert!(!validate(&model, &report, &graph, &lasso, &atoms, &[], &property).unwrap());
}
