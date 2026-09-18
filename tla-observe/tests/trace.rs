use std::{convert::Infallible, num::NonZeroUsize};
use tla_engine::{Options, Transition, TransitionSystem, explore_parallel};
use tla_observe::Timeline;
struct Graph;
impl TransitionSystem for Graph {
    type State = usize;
    type Key = usize;
    type Error = Infallible;
    fn initial_states(&self) -> Result<Vec<usize>, Infallible> {
        Ok(vec![0])
    }
    fn successors(&self, s: &usize) -> Result<Vec<Transition<usize>>, Infallible> {
        Ok([(*s * 2 + 1) % 255, (*s * 2 + 2) % 255]
            .into_iter()
            .enumerate()
            .map(|(action, state)| Transition { action, state })
            .collect())
    }
    fn key(&self, s: &usize) -> Result<usize, Infallible> {
        Ok(*s)
    }
    fn same_state(&self, a: &usize, b: &usize) -> Result<bool, Infallible> {
        Ok(a == b)
    }
}
#[test]
fn traced_parallel_search_preserves_results_and_records_worker_counters() {
    let workers = NonZeroUsize::new(4).unwrap();
    let expected = explore_parallel(&Graph, &Options::default(), workers).unwrap();
    let timeline = Timeline::new();
    let actual = timeline
        .span("explore", || {
            explore_parallel(&timeline.measured(&Graph), &Options::default(), workers)
        })
        .unwrap();
    assert_eq!(actual.completion, expected.completion);
    assert_eq!(actual.nodes.len(), expected.nodes.len());
    let mut json = Vec::new();
    timeline.write(&mut json).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let counts = json["counters"].as_array().unwrap();
    assert_eq!(
        counts
            .iter()
            .filter(|c| c["stage"] == "successors")
            .map(|c| c["items"].as_u64().unwrap())
            .sum::<u64>(),
        510
    );
    assert!(
        json["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["ph"] == "X" && event["dur"].as_f64().unwrap() >= 0.0)
    );
    assert_eq!(json["droppedEvents"], 0);
}
