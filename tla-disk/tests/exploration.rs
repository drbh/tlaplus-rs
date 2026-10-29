use std::{convert::Infallible, num::NonZeroUsize};
use tlaplus_disk::store::Store;
use tlaplus_disk::{Codec, CodecError, ValueCodec};
use tlaplus_engine::{Completion, Options, Transition, TransitionSystem, ViolationKind};
use tlaplus_values::Value;

struct Graph {
    size: usize,
    deadlock: bool,
    bad: Option<usize>,
    prune: Option<usize>,
}
impl TransitionSystem for Graph {
    type State = usize;
    type Key = usize;
    type Error = Infallible;
    fn initial_states(&self) -> Result<Vec<usize>, Infallible> {
        Ok(vec![0, 0])
    }
    fn successors(&self, state: &usize) -> Result<Vec<Transition<usize>>, Infallible> {
        if self.deadlock && *state == self.size - 1 {
            return Ok(vec![]);
        }
        Ok([(*state + 1) % self.size, (*state + 2) % self.size]
            .into_iter()
            .enumerate()
            .map(|(action, state)| Transition { action, state })
            .collect())
    }
    fn key(&self, state: &usize) -> Result<usize, Infallible> {
        Ok(*state)
    }
    fn same_state(&self, a: &usize, b: &usize) -> Result<bool, Infallible> {
        Ok(a == b)
    }
    fn state_allowed(&self, state: &usize) -> Result<bool, Infallible> {
        Ok(Some(*state) != self.prune)
    }
    fn invariant(&self, state: &usize) -> Result<Option<String>, Infallible> {
        Ok((Some(*state) == self.bad).then(|| "Safe".into()))
    }
}
struct Numbers;
impl Codec<usize, usize> for Numbers {
    fn state(&self, n: &usize) -> Result<Vec<u8>, CodecError> {
        Ok(n.to_le_bytes().to_vec())
    }
    fn key(&self, n: &usize) -> Result<Vec<u8>, CodecError> {
        self.state(n)
    }
    fn decode(&self, bytes: &[u8]) -> Result<usize, CodecError> {
        Ok(usize::from_le_bytes(
            bytes.try_into().map_err(|_| CodecError::Truncated)?,
        ))
    }
}

#[test]
fn disk_bfs_matches_memory_counts_depth_and_shortest_witnesses() {
    for (size, deadlock, bad, prune, cap) in [
        (300, false, None, None, None),
        (30, true, None, None, None),
        (30, false, Some(0), None, None),
        (30, false, Some(17), None, None),
        (30, false, Some(17), Some(17), None),
        (30, false, None, Some(17), None),
        (30, false, None, None, NonZeroUsize::new(4)),
    ] {
        let graph = Graph {
            size,
            deadlock,
            bad,
            prune,
        };
        let options = Options {
            max_states: cap,
            ..Options::default()
        };
        let expected = tlaplus_engine::explore(&graph, &options).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(
            dir.path().join("states.redb"),
            NonZeroUsize::new(32 << 10).unwrap(),
        )
        .unwrap();
        let actual = tlaplus_disk::explore(&graph, &Numbers, &store, &options).unwrap();
        assert_eq!(actual.completion, expected.completion);
        if let Completion::Violated(violation) = actual.completion {
            assert!(tlaplus_engine::validate_trace(&graph, &violation).unwrap());
        } else {
            assert_eq!(actual.distinct as usize, expected.nodes.len());
            assert_eq!(actual.statistics.depth, expected.statistics.depth);
            assert_eq!(
                actual.statistics.initial_distinct,
                expected.statistics.initial_distinct
            );
        }
    }
}

#[test]
fn first_violation_precedes_later_deadlock_or_error() {
    struct FirstViolation(bool);
    impl TransitionSystem for FirstViolation {
        type State = usize;
        type Key = usize;
        type Error = &'static str;
        fn initial_states(&self) -> Result<Vec<usize>, &'static str> {
            Ok(vec![0, 1])
        }
        fn successors(&self, s: &usize) -> Result<Vec<Transition<usize>>, &'static str> {
            if *s == 1 && self.0 {
                return Err("later evaluation error");
            }
            Ok(if *s == 0 {
                vec![Transition {
                    action: 0,
                    state: 2,
                }]
            } else {
                vec![]
            })
        }
        fn key(&self, s: &usize) -> Result<usize, &'static str> {
            Ok(*s)
        }
        fn same_state(&self, a: &usize, b: &usize) -> Result<bool, &'static str> {
            Ok(a == b)
        }
        fn invariant(&self, s: &usize) -> Result<Option<String>, &'static str> {
            Ok((*s == 2).then(|| "Bad".into()))
        }
    }
    for error in [false, true] {
        let system = FirstViolation(error);
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::open(dir.path().join("s.redb"), NonZeroUsize::new(4096).unwrap()).unwrap();
        let report = tlaplus_disk::explore(&system, &Numbers, &store, &Options::default()).unwrap();
        let memory = tlaplus_engine::explore(&system, &Options::default()).unwrap();
        assert_eq!(report.completion, memory.completion);
        assert_eq!(report.statistics.generated, 3);
        let Completion::Violated(v) = report.completion else {
            panic!("expected invariant violation")
        };
        assert_eq!(v.kind, ViolationKind::Invariant("Bad".into()));
        assert_eq!(v.trace.states, vec![0, 2]);
    }
}

#[test]
fn value_codec_is_lossless_and_rejects_malformed_inputs() {
    let states = vec![
        Value::Bool(true),
        Value::Int(i32::MIN),
        Value::string("雪", 41),
        Value::model("m", 7),
        Value::set(vec![Value::Int(3), Value::Int(1)]).unwrap(),
        Value::tuple(vec![Value::Bool(false), Value::string("", 0)]),
    ];
    let bytes = ValueCodec.state(&states).unwrap();
    assert_eq!(ValueCodec.decode(&bytes).unwrap(), states);
    for end in 0..bytes.len() {
        assert!(ValueCodec.decode(&bytes[..end]).is_err());
    }
    let mut corrupt = bytes.clone();
    corrupt.push(0);
    assert!(ValueCodec.decode(&corrupt).is_err());
    corrupt = bytes;
    corrupt[4] = 255;
    assert!(ValueCodec.decode(&corrupt).is_err());
    assert_ne!(
        ValueCodec.key(&vec![Value::model("m", 7)]).unwrap(),
        ValueCodec.key(&vec![Value::string("m", 7)]).unwrap()
    );
}

#[test]
fn disk_limit_stops_streaming_initial_generation() {
    struct Streaming;
    impl TransitionSystem for Streaming {
        type State = usize;
        type Key = usize;
        type Error = Infallible;
        fn initial_states(&self) -> Result<Vec<usize>, Infallible> {
            panic!("must use streaming")
        }
        fn successors(&self, _: &usize) -> Result<Vec<Transition<usize>>, Infallible> {
            panic!("limit precedes successors")
        }
        fn key(&self, state: &usize) -> Result<usize, Infallible> {
            Ok(*state)
        }
        fn same_state(&self, a: &usize, b: &usize) -> Result<bool, Infallible> {
            Ok(a == b)
        }
        fn visit_initial(
            &self,
            output: &mut impl FnMut(usize) -> Result<std::ops::ControlFlow<()>, Infallible>,
        ) -> Result<std::ops::ControlFlow<()>, Infallible> {
            for state in 0..usize::MAX {
                if output(state)?.is_break() {
                    return Ok(std::ops::ControlFlow::Break(()));
                }
            }
            unreachable!()
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(
        directory.path().join("states.redb"),
        NonZeroUsize::new(4096).unwrap(),
    )
    .unwrap();
    let report = tlaplus_disk::explore(
        &Streaming,
        &Numbers,
        &store,
        &Options {
            max_states: NonZeroUsize::new(3),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(matches!(report.completion, Completion::StateLimit));
    assert_eq!(report.distinct, 3);
    assert_eq!(report.statistics.generated, 4);
}

#[test]
fn state_codec_preserves_depth_without_changing_identity() {
    use tlaplus_disk::{Codec, StateCodec};
    let state = tlaplus_engine::Leveled {
        values: vec![tlaplus_values::Value::Int(7)],
        depth: 42,
    };
    let bytes = StateCodec.state(&state).unwrap();
    let decoded = StateCodec.decode(&bytes).unwrap();
    assert_eq!(decoded.depth, 42);
    assert_eq!(decoded, state);
    let mut earlier = state.clone();
    earlier.depth = 1;
    assert_eq!(earlier, state);
    assert_eq!(
        StateCodec.key(&earlier.values).unwrap(),
        StateCodec.key(&state.values).unwrap()
    );
    assert_ne!(StateCodec.state(&earlier).unwrap(), bytes);
    for n in 0..bytes.len() {
        assert!(StateCodec.decode(&bytes[..n]).is_err());
    }
    earlier.depth = 0;
    assert!(StateCodec.state(&earlier).is_err());
}
