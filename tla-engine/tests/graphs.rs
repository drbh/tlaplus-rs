use std::convert::Infallible;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Barrier;

use tla_engine::{
    Completion, Options, Transition, TransitionSystem, ViolationKind, explore, explore_parallel,
    validate_trace,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Key(usize);
impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0u8.hash(state);
    }
}

struct Graph {
    prefetch: bool,
    roots: Vec<usize>,
    edges: Vec<Vec<usize>>,
    bad: Option<usize>,
    bad_action: Option<(usize, usize)>,
    excluded: Option<usize>,
    blocked: Option<(usize, usize)>,
    fail: Option<usize>,
    late_failure: bool,
    prefetched: std::sync::atomic::AtomicUsize,
}

impl Graph {
    fn new(edges: Vec<Vec<usize>>) -> Self {
        Self {
            prefetch: true,
            roots: vec![0],
            edges,
            bad: None,
            bad_action: None,
            excluded: None,
            blocked: None,
            fail: None,
            late_failure: false,
            prefetched: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl TransitionSystem for Graph {
    type State = usize;
    type Key = Key;
    type Error = &'static str;
    fn prefetch_successors(&self) -> bool {
        self.prefetch
    }
    fn same_state(&self, left: &usize, right: &usize) -> Result<bool, Self::Error> {
        Ok(left == right)
    }
    fn initial_states(&self) -> Result<Vec<usize>, Self::Error> {
        Ok(self.roots.clone())
    }
    fn successors(&self, &state: &usize) -> Result<Vec<Transition<usize>>, Self::Error> {
        self.prefetched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fail == Some(state) || self.late_failure {
            return Err("evaluation failed");
        }
        Ok(self.edges[state]
            .iter()
            .enumerate()
            .map(|(action, &state)| Transition { action, state })
            .collect())
    }
    fn visit_successors(
        &self,
        state: &usize,
        out: &mut impl FnMut(Transition<usize>) -> Result<std::ops::ControlFlow<()>, Self::Error>,
    ) -> Result<std::ops::ControlFlow<()>, Self::Error> {
        if self.fail == Some(*state) {
            return Err("evaluation failed");
        }
        for (action, &state) in self.edges[*state].iter().enumerate() {
            if out(Transition { action, state })?.is_break() {
                return Ok(std::ops::ControlFlow::Break(()));
            }
        }
        if self.late_failure {
            return Err("late evaluation failed");
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }
    fn key(&self, &state: &usize) -> Result<Key, Self::Error> {
        Ok(Key(state))
    }
    fn state_allowed(&self, &state: &usize) -> Result<bool, Self::Error> {
        Ok(self.excluded != Some(state))
    }
    fn step_allowed(&self, &from: &usize, &to: &usize) -> Result<bool, Self::Error> {
        Ok(self.blocked != Some((from, to)))
    }
    fn invariant(&self, &state: &usize) -> Result<Option<String>, Self::Error> {
        Ok((self.bad == Some(state)).then(|| "Safe".into()))
    }
    fn action_property(&self, &from: &usize, &to: &usize) -> Result<Option<String>, Self::Error> {
        Ok((self.bad_action == Some((from, to))).then(|| "Step".into()))
    }
}

#[test]
fn early_violation_does_not_prefetch_the_whole_frontier() {
    let mut graph = Graph::new(vec![vec![1001]; 1002]);
    graph.edges[0] = (1..=1000).collect();
    graph.bad = Some(1001);
    let serial = explore(&graph, &Options::default()).unwrap();
    let parallel =
        explore_parallel(&graph, &Options::default(), NonZeroUsize::new(4).unwrap()).unwrap();
    assert_eq!(serial.completion, parallel.completion);
    assert_eq!(serial.statistics.generated, parallel.statistics.generated);
    assert!(graph.prefetched.load(std::sync::atomic::Ordering::Relaxed) <= 65);
    graph.bad = None;
    let options = Options {
        record_graph: true,
        ..Options::default()
    };
    let serial = explore(&graph, &options).unwrap();
    let parallel = explore_parallel(&graph, &options, NonZeroUsize::new(4).unwrap()).unwrap();
    assert_eq!(parallel.completion, Completion::Complete);
    assert_eq!(serial.edges, parallel.edges);
    assert_eq!(serial.nodes.len(), parallel.nodes.len());
    assert!(parallel.nodes.iter().all(|node| node.expanded));
    assert_eq!(serial.statistics.depth, parallel.statistics.depth);
    assert_eq!(
        serial.statistics.peak_frontier,
        parallel.statistics.peak_frontier
    );
}

#[test]
fn collisions_duplicates_and_parent_paths_preserve_exact_graphs() {
    let mut g = Graph::new(vec![vec![1, 1, 2], vec![3], vec![3, 0], vec![3]]);
    g.roots = vec![0, 0];
    let options = Options {
        record_graph: true,
        ..Options::default()
    };
    let report = explore(&g, &options).unwrap();
    assert_eq!(report.completion, Completion::Complete);
    assert_eq!(
        report.nodes.iter().map(|n| n.state).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(report.statistics.initial_distinct, 1);
    assert_eq!(report.statistics.depth, 3);
    assert_eq!(report.edges.len(), 7);
    assert_eq!(report.trace_to(3).states, [0, 1, 3]);
    assert_eq!(report.trace_to(3).actions, [0, 0]);
}

#[test]
fn earlier_property_failure_wins_over_later_deadlock() {
    let mut g = Graph::new(vec![vec![2], vec![], vec![2]]);
    g.roots = vec![0, 1];
    g.bad = Some(2);
    let report = explore(&g, &Options::default()).unwrap();
    let Completion::Violated(violation) = report.completion else {
        panic!("missing violation")
    };
    assert_eq!(violation.kind, ViolationKind::Invariant("Safe".into()));
    assert_eq!(violation.trace.states, [0, 2]);
    let parallel =
        explore_parallel(&g, &Options::default(), NonZeroUsize::new(4).unwrap()).unwrap();
    assert_eq!(parallel.completion, Completion::Violated(violation.clone()));
    assert!(validate_trace(&g, &violation).unwrap());
}

#[test]
fn properties_are_checked_on_pruned_states_and_seen_edges() {
    let mut g = Graph::new(vec![vec![1], vec![0]]);
    g.bad = Some(1);
    g.excluded = Some(1);
    let options = Options::default();
    let Completion::Violated(mut violation) = explore(&g, &options).unwrap().completion else {
        panic!()
    };
    assert_eq!(violation.kind, ViolationKind::Invariant("Safe".into()));
    assert!(validate_trace(&g, &violation).unwrap());
    violation.trace.actions[0] = 99;
    assert!(!validate_trace(&g, &violation).unwrap());
    g.bad = None;
    assert_eq!(
        explore(&g, &options).unwrap().completion,
        Completion::Complete
    );
    g.excluded = None;
    g.bad_action = Some((1, 0));
    let Completion::Violated(violation) = explore(&g, &options).unwrap().completion else {
        panic!()
    };
    assert_eq!(violation.trace.states, [0, 1, 0]);
    assert!(validate_trace(&g, &violation).unwrap());
    g.blocked = Some((1, 0));
    assert!(matches!(
        explore(&g, &options).unwrap().completion,
        Completion::Violated(_)
    ));
}

#[test]
fn limits_errors_and_empty_initial_sets_never_become_false_successes() {
    let mut g = Graph::new(vec![vec![1], vec![1]]);
    let options = Options {
        max_states: NonZeroUsize::new(1),
        ..Options::default()
    };
    assert_eq!(
        explore(&g, &options).unwrap().completion,
        Completion::StateLimit
    );
    g.fail = Some(0);
    assert_eq!(
        explore(&g, &Options::default()).unwrap_err(),
        "evaluation failed"
    );
    g.roots.clear();
    let report = explore(&g, &Options::default()).unwrap();
    assert_eq!(report.completion, Completion::Complete);
    assert!(report.nodes.is_empty());
    assert_eq!(report.statistics.depth, 0);
}

#[test]
fn serial_and_parallel_match_independent_all_pairs_distances() {
    let mut seed = 17u64;
    for n in 1..35 {
        let mut edges = vec![Vec::new(); n];
        let mut distance = vec![vec![n + 1; n]; n];
        for (i, (links, row)) in edges.iter_mut().zip(&mut distance).enumerate() {
            row[i] = 0;
            for (j, entry) in row.iter_mut().enumerate() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                if seed >> 60 < 3 {
                    links.push(j);
                    *entry = (*entry).min(1);
                }
            }
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    distance[i][j] = distance[i][j].min(distance[i][k] + distance[k][j]);
                }
            }
        }
        let mut g = Graph::new(edges);
        let options = Options {
            check_deadlock: false,
            record_graph: true,
            ..Options::default()
        };
        let serial = explore(&g, &options).unwrap();
        let expected: Vec<_> = (0..n).filter(|&i| distance[0][i] <= n).collect();
        let mut actual: Vec<_> = serial.nodes.iter().map(|node| node.state).collect();
        actual.sort();
        assert_eq!(actual, expected);
        assert_eq!(
            serial.statistics.depth,
            expected.iter().map(|&i| distance[0][i] + 1).max().unwrap()
        );
        for (i, node) in serial.nodes.iter().enumerate() {
            let trace = serial.trace_to(i);
            assert_eq!(trace.states.len(), distance[0][node.state] + 1);
            assert!(
                trace
                    .states
                    .windows(2)
                    .all(|p| g.edges[p[0]].contains(&p[1]))
            );
        }
        for workers in [1, 2, 4] {
            let parallel =
                explore_parallel(&g, &options, NonZeroUsize::new(workers).unwrap()).unwrap();
            assert_eq!(parallel.completion, serial.completion);
            assert_eq!(parallel.statistics.depth, serial.statistics.depth);
            assert_eq!(parallel.edges, serial.edges);
        }
        if let Some(&bad) = expected.iter().skip(1).next_back() {
            g.bad = Some(bad);
            let serial = explore(&g, &options).unwrap();
            let parallel = explore_parallel(&g, &options, NonZeroUsize::new(4).unwrap()).unwrap();
            assert_eq!(parallel.completion, serial.completion);
            let Completion::Violated(violation) = serial.completion else {
                panic!("missing invariant violation")
            };
            assert_eq!(violation.trace.states.len(), distance[0][bad] + 1);
            assert!(validate_trace(&g, &violation).unwrap());
        }
    }
}

#[test]
fn parallel_expansion_really_runs_concurrently() {
    struct Concurrent(Barrier);
    impl TransitionSystem for Concurrent {
        type State = usize;
        type Key = usize;
        type Error = Infallible;
        fn same_state(&self, left: &usize, right: &usize) -> Result<bool, Infallible> {
            Ok(left == right)
        }
        fn initial_states(&self) -> Result<Vec<usize>, Infallible> {
            Ok(vec![0, 1, 2, 3])
        }
        fn key(&self, &state: &usize) -> Result<usize, Infallible> {
            Ok(state)
        }
        fn successors(&self, &state: &usize) -> Result<Vec<Transition<usize>>, Infallible> {
            self.0.wait();
            Ok(vec![Transition { state, action: 0 }])
        }
    }
    let report = explore_parallel(
        &Concurrent(Barrier::new(4)),
        &Options::default(),
        NonZeroUsize::new(4).unwrap(),
    )
    .unwrap();
    assert_eq!(report.nodes.len(), 4);
    assert_eq!(report.completion, Completion::Complete);
}

#[test]
fn early_violation_precedes_generation_errors_in_serial_and_parallel() {
    let mut g = Graph::new(vec![vec![2], vec![], vec![2]]);
    g.roots = vec![0, 1];
    g.bad = Some(2);
    for late_failure in [false, true] {
        g.late_failure = late_failure;
        g.fail = Some(1);
        for workers in [1, 4] {
            let report =
                explore_parallel(&g, &Options::default(), NonZeroUsize::new(workers).unwrap())
                    .unwrap();
            let Completion::Violated(violation) = report.completion else {
                panic!("missing violation")
            };
            assert_eq!(violation.kind, ViolationKind::Invariant("Safe".into()));
            assert_eq!(violation.trace.states, [0, 2]);
            assert_eq!(report.statistics.generated, 3);
            assert!(validate_trace(&g, &violation).unwrap());
        }
    }
    g.bad = None;
    for workers in [1, 4] {
        assert_eq!(
            explore_parallel(&g, &Options::default(), NonZeroUsize::new(workers).unwrap())
                .unwrap_err(),
            "late evaluation failed"
        );
    }
}

#[test]
fn replay_coverage_marks_only_finished_expansions() {
    for workers in [1, 4] {
        let run = |g: &Graph, options: &Options| {
            if workers == 1 {
                explore(g, options)
            } else {
                explore_parallel(g, options, NonZeroUsize::new(workers).unwrap())
            }
            .unwrap()
        };
        let mut g = Graph::new(vec![vec![1, 2], vec![1], vec![2]]);
        let complete = run(&g, &Options::default());
        assert!(matches!(complete.completion, Completion::Complete));
        assert!(complete.nodes.iter().all(|node| node.expanded));
        g.bad = Some(2);
        let violation = run(&g, &Options::default());
        assert!(matches!(violation.completion, Completion::Violated(_)));
        assert!(violation.nodes.iter().all(|node| !node.expanded));
        g.bad = None;
        let limited = run(
            &g,
            &Options {
                max_states: NonZeroUsize::new(2),
                ..Options::default()
            },
        );
        assert!(matches!(limited.completion, Completion::StateLimit));
        assert!(limited.nodes.iter().all(|node| !node.expanded));
        let deadlock = run(&Graph::new(vec![vec![]]), &Options::default());
        assert!(matches!(deadlock.completion, Completion::Violated(_)));
        assert!(deadlock.nodes[0].expanded);
    }
}

#[test]
fn streaming_witness_replay_is_bounded_and_rejects_forged_traces() {
    let mut graph = Graph::new(vec![vec![1], vec![2], vec![]]);
    graph.prefetch = false;
    graph.bad = Some(2);
    let report =
        explore_parallel(&graph, &Options::default(), NonZeroUsize::new(4).unwrap()).unwrap();
    let Completion::Violated(mut violation) = report.completion else {
        panic!("missing invariant violation");
    };
    assert!(validate_trace(&graph, &violation).unwrap());
    violation.trace.states[1] = 0;
    assert!(!validate_trace(&graph, &violation).unwrap());
    violation.trace.states[1] = 1;
    violation.trace.actions[0] = 99;
    assert!(!validate_trace(&graph, &violation).unwrap());
    graph.bad = None;
    let Completion::Violated(deadlock) = explore(&graph, &Options::default()).unwrap().completion
    else {
        panic!("missing deadlock");
    };
    assert!(validate_trace(&graph, &deadlock).unwrap());
    graph.fail = Some(1);
    let forged = tla_engine::Violation {
        kind: ViolationKind::Invariant("Safe".into()),
        trace: tla_engine::Trace {
            states: vec![0, 1],
            actions: vec![0],
        },
    };
    assert!(!validate_trace(&graph, &forged).unwrap());
}

#[test]
fn continuing_records_first_violations_and_completes_exploration() {
    // 0 -> 1 -> 2 -> 3 -> 1, and 0 -> 3; state 3 violates Safe; (2, 3) violates Step.
    let mut graph = Graph::new(vec![vec![1, 3], vec![2], vec![3], vec![1]]);
    graph.bad = Some(3);
    graph.bad_action = Some((2, 3));
    let options = Options {
        continue_after_violation: true,
        ..Options::default()
    };
    for workers in [1, 4] {
        for prefetch in [true, false] {
            graph.prefetch = prefetch;
            let report =
                explore_parallel(&graph, &options, NonZeroUsize::new(workers).unwrap()).unwrap();
            assert_eq!(report.completion, Completion::Complete);
            assert_eq!(report.nodes.len(), 4);
            let found: Vec<_> = report
                .violations
                .iter()
                .map(|v| (v.kind.clone(), v.trace.states.clone()))
                .collect();
            assert_eq!(
                found,
                [
                    (ViolationKind::Invariant("Safe".into()), vec![0, 3]),
                    (
                        ViolationKind::ActionProperty("Step".into()),
                        vec![0, 1, 2, 3]
                    ),
                ]
            );
            for violation in &report.violations {
                assert!(validate_trace(&graph, violation).unwrap());
            }
        }
    }
    graph.roots = vec![3];
    let report = explore(&graph, &options).unwrap();
    assert_eq!(report.violations.len(), 2);
    assert_eq!(report.violations[0].trace.states, [3]);
    assert_eq!(report.violations[1].trace.states, [3, 1, 2, 3]);
    assert_eq!(report.nodes.len(), 3);
    graph.edges[2].clear();
    let report = explore(&graph, &options).unwrap();
    assert!(matches!(
        report.completion,
        Completion::Violated(v) if v.kind == ViolationKind::Deadlock
    ));
    let stopped = explore(&graph, &Options::default()).unwrap();
    assert!(stopped.violations.is_empty());
    assert!(matches!(
        stopped.completion,
        Completion::Violated(v) if v.kind == ViolationKind::Invariant("Safe".into())
    ));
}
