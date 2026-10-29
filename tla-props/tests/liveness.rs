use std::{collections::BTreeSet, num::NonZeroUsize};
use tlaplus_props::{Check, Edge, Formula as F, Graph, Lasso, check};

fn edge(from: usize, to: usize, atoms: &[usize]) -> Edge {
    Edge {
        from,
        to,
        atoms: atoms.iter().copied().collect(),
    }
}
fn verdict(graph: &Graph, assumptions: &[F], property: &F) -> bool {
    match check(
        graph,
        assumptions,
        property,
        NonZeroUsize::new(100_000).unwrap(),
    )
    .unwrap()
    {
        Check::Holds { .. } => true,
        Check::Violated { lasso, .. } => {
            assert!(!lasso.satisfies(graph, property).unwrap());
            for assumption in assumptions {
                assert!(lasso.satisfies(graph, assumption).unwrap());
            }
            false
        }
        Check::StateLimit { .. } => panic!("unexpected product limit"),
    }
}

#[test]
fn safety_eventuality_and_nested_temporal_formulas() {
    let graph = Graph {
        nodes: 2,
        initial: vec![0],
        edges: vec![edge(0, 1, &[]), edge(1, 1, &[0])],
    };
    assert!(verdict(&graph, &[], &F::atom(0).eventually()));
    assert!(!verdict(&graph, &[], &F::atom(0).always()));
    assert!(verdict(&graph, &[], &F::atom(0).always().eventually()));
    assert!(verdict(&graph, &[], &F::atom(0).eventually().always()));
    assert!(verdict(&graph, &[], &F::atom(0).next()));
    assert!(!verdict(
        &graph,
        &[],
        &F::atom(0).negate().eventually().always()
    ));
    let empty = Graph {
        nodes: 0,
        initial: vec![],
        edges: vec![],
    };
    assert!(verdict(&empty, &[], &F::False));
}

#[test]
fn fairness_excludes_only_the_unfair_cycles() {
    let graph = Graph {
        nodes: 2,
        initial: vec![0],
        edges: vec![edge(0, 0, &[0]), edge(0, 1, &[0, 1]), edge(1, 1, &[2])],
    };
    let eventual = F::atom(2).eventually();
    assert!(!verdict(&graph, &[], &eventual));
    assert!(verdict(&graph, &[F::weak_fairness(0, 1)], &eventual));
    assert!(verdict(&graph, &[F::strong_fairness(0, 1)], &eventual));
    let intermittent = Graph {
        nodes: 3,
        initial: vec![0],
        edges: vec![
            edge(0, 1, &[0]),
            edge(1, 0, &[]),
            edge(0, 2, &[0, 1]),
            edge(2, 2, &[2]),
        ],
    };
    assert!(!verdict(
        &intermittent,
        &[F::weak_fairness(0, 1)],
        &eventual
    ));
    assert!(verdict(
        &intermittent,
        &[F::strong_fairness(0, 1)],
        &eventual
    ));
}

#[test]
fn generalized_acceptance_visits_multiple_parts_of_an_scc() {
    let graph = Graph {
        nodes: 3,
        initial: vec![0],
        edges: vec![
            edge(0, 1, &[0]),
            edge(1, 0, &[]),
            edge(0, 2, &[1]),
            edge(2, 0, &[]),
        ],
    };
    let both = F::atom(0)
        .eventually()
        .always()
        .and(F::atom(1).eventually().always());
    assert!(!verdict(&graph, &[both], &F::False));
    assert!(verdict(&graph, &[F::atom(2).eventually()], &F::False));
}

#[test]
fn independent_lasso_evaluator_rejects_corruption_and_limits_do_not_pass() {
    let graph = Graph {
        nodes: 2,
        initial: vec![0],
        edges: vec![edge(0, 1, &[]), edge(1, 1, &[0])],
    };
    for lasso in [
        Lasso {
            initial: 0,
            prefix: vec![],
            cycle: vec![],
        },
        Lasso {
            initial: 0,
            prefix: vec![],
            cycle: vec![1],
        },
        Lasso {
            initial: 0,
            prefix: vec![0],
            cycle: vec![99],
        },
        Lasso {
            initial: 0,
            prefix: vec![],
            cycle: vec![0],
        },
    ] {
        assert!(lasso.satisfies(&graph, &F::True).is_err());
    }
    assert!(matches!(
        check(&graph, &[], &F::False, NonZeroUsize::new(1).unwrap()).unwrap(),
        Check::StateLimit { .. }
    ));
}

#[test]
fn tableau_matches_fixed_points_on_deterministic_random_lassos() {
    let mut seed = 781u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut formulas = vec![F::True, F::False, F::atom(0), F::atom(1)];
    for _ in 0..80 {
        let a = formulas[random() as usize % formulas.len()].clone();
        let b = formulas[random() as usize % formulas.len()].clone();
        formulas.push(match random() % 8 {
            0 => a.negate(),
            1 => a.and(b),
            2 => a.or(b),
            3 => a.until(b),
            4 => a.always(),
            5 => a.eventually(),
            6 => a.next(),
            _ => a.implies(b),
        });
    }
    for n in 1..9 {
        for _ in 0..8 {
            let loop_at = random() as usize % n;
            let graph = Graph {
                nodes: n,
                initial: vec![0],
                edges: (0..n)
                    .map(|i| Edge {
                        from: i,
                        to: if i + 1 == n { loop_at } else { i + 1 },
                        atoms: (0..2)
                            .filter(|_| random() & 1 != 0)
                            .collect::<BTreeSet<_>>(),
                    })
                    .collect(),
            };
            let lasso = Lasso {
                initial: 0,
                prefix: (0..loop_at).collect(),
                cycle: (loop_at..n).collect(),
            };
            for formula in &formulas {
                assert_eq!(
                    verdict(&graph, &[], formula),
                    lasso.satisfies(&graph, formula).unwrap(),
                    "{formula:?}"
                );
            }
        }
    }
}

#[test]
fn strong_fairness_refines_components_and_many_constraints_stay_small() {
    // In {0, 1} the action (atom 0) is enabled but never taken (atom 1), so the
    // component is unfair; without the enabled edges, the loop at 0 is fair.
    let graph = Graph {
        nodes: 2,
        initial: vec![0],
        edges: vec![
            edge(0, 0, &[]),
            edge(0, 1, &[]),
            edge(1, 0, &[0]),
            edge(1, 1, &[0]),
        ],
    };
    let never = F::atom(2).eventually();
    assert!(!verdict(&graph, &[F::strong_fairness(0, 1)], &never));
    // Weak fairness is violated in every cycle through 1, but not at 0 alone.
    assert!(!verdict(&graph, &[F::weak_fairness(0, 1)], &never));
    // Forty fairness constraints no longer multiply the tableau.
    let many: Vec<_> = (3..43).map(|i| F::weak_fairness(i, i + 100)).collect();
    let Check::Violated { product_states, .. } =
        check(&graph, &many, &never, NonZeroUsize::new(1000).unwrap()).unwrap()
    else {
        panic!("expected a fair counterexample");
    };
    assert!(product_states <= 4, "{product_states}");
}
