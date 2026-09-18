use std::{
    collections::HashMap,
    num::NonZeroUsize,
    ops::ControlFlow,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
};
use tla_bridge::{ModelRequest, Outcome, RunEnv, observe_faithful, with_model};
use tla_engine::{Completion, Options, Transition, TransitionSystem, explore_parallel};

#[derive(Default)]
struct Streaming {
    fail: Option<&'static str>,
    active: AtomicUsize,
    peak: AtomicUsize,
    contexts: Mutex<HashMap<ThreadId, usize>>,
    calls: AtomicUsize,
}

impl TransitionSystem for Streaming {
    type State = usize;
    type Key = usize;
    type Error = String;
    fn key(&self, state: &usize) -> Result<usize, String> {
        Ok(*state)
    }
    fn same_state(&self, a: &usize, b: &usize) -> Result<bool, String> {
        Ok(a == b)
    }
    fn prefetch_successors(&self) -> bool {
        false
    }
    fn parallel_callbacks(&self) -> bool {
        true
    }
    fn initial_states(&self) -> Result<Vec<usize>, String> {
        Ok((0..4).collect())
    }
    fn successors(&self, _: &usize) -> Result<Vec<Transition<usize>>, String> {
        panic!("stateful generation must stream without retries")
    }
    fn visit_successors(
        &self,
        state: &usize,
        output: &mut impl FnMut(Transition<usize>) -> Result<ControlFlow<()>, String>,
    ) -> Result<ControlFlow<()>, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if *state >= 4 {
            return Ok(ControlFlow::Continue(()));
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while self.peak.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
            thread::sleep(std::time::Duration::from_millis(1));
        }
        self.contexts
            .lock()
            .unwrap()
            .insert(thread::current().id(), state + 4);
        let flow = output(Transition {
            state: state + 4,
            action: 0,
        })?;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if flow.is_break() {
            return Ok(flow);
        }
        assert_eq!(
            self.contexts.lock().unwrap()[&thread::current().id()],
            state + 14
        );
        Ok(flow)
    }
    fn invariant(&self, state: &usize) -> Result<Option<String>, String> {
        if *state >= 4 {
            match self.fail {
                Some("evaluation") => return Err("evaluation failure".into()),
                Some("violation") => return Ok(Some("Safe".into())),
                _ => {}
            }
            let mut contexts = self.contexts.lock().unwrap();
            let current = contexts
                .get_mut(&thread::current().id())
                .ok_or("callback left its worker")?;
            if *current != *state {
                return Err("callback lost its generation context".into());
            }
            *current += 10;
        }
        Ok(None)
    }
}

#[test]
fn workers_stream_callbacks_before_generation_resumes_without_retries() {
    let system = Streaming::default();
    let options = Options {
        check_deadlock: false,
        ..Options::default()
    };
    let report = explore_parallel(&system, &options, NonZeroUsize::new(4).unwrap()).unwrap();
    assert_eq!(report.completion, Completion::Complete);
    assert_eq!(report.nodes.len(), 8);
    assert_eq!(system.calls.load(Ordering::Relaxed), 8);
    assert!(system.peak.load(Ordering::Relaxed) > 1);
    assert!(system.contexts.lock().unwrap().len() > 1);
}

#[test]
fn streaming_stops_on_callback_failures_without_retrying_or_marking_nodes_expanded() {
    let options = Options {
        check_deadlock: false,
        ..Options::default()
    };
    for fail in ["evaluation", "violation"] {
        let system = Streaming {
            fail: Some(fail),
            ..Streaming::default()
        };
        let result = explore_parallel(&system, &options, NonZeroUsize::new(4).unwrap());
        if fail == "evaluation" {
            assert_eq!(result.unwrap_err(), "evaluation failure");
        } else {
            let report = result.unwrap();
            assert!(matches!(report.completion, Completion::Violated(_)));
            assert_eq!(report.nodes.len(), 4);
            assert_eq!(report.statistics.generated, 5);
            assert!(report.nodes.iter().all(|node| !node.expanded));
        }
        assert!(system.calls.load(Ordering::Relaxed) <= 4);
    }
}

#[test]
fn native_worker_registers_preserve_generation_and_invariant_interleaving() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    let metadir = directory
        .join("../target")
        .join(format!("worker-registers-{}", std::process::id()));
    let expected = observe_faithful(
        [
            "-tool",
            "-workers",
            "4",
            "-metadir",
            metadir.to_str().unwrap(),
            "WorkerRegisters",
        ]
        .map(str::to_owned)
        .to_vec(),
        RunEnv {
            user_dir: directory.clone(),
            ..RunEnv::default()
        },
    )
    .unwrap();
    assert_eq!(expected.summary.outcome, Outcome::Success);
    let model = with_model(
        ModelRequest::new(directory.join("WorkerRegisters.tla")),
        |model| model.lower(),
    )
    .unwrap();
    let options = Options {
        check_deadlock: false,
        record_graph: true,
        ..Options::default()
    };
    for workers in [1, 2, 4, 4] {
        let report =
            explore_parallel(&model, &options, NonZeroUsize::new(workers).unwrap()).unwrap();
        assert_eq!(report.completion, Completion::Complete);
        assert_eq!(report.nodes.len(), 12);
        assert_eq!(report.edges.len(), 8);
        assert_eq!(expected.summary.distinct, Some(report.nodes.len() as u64));
        assert_eq!(expected.summary.depth, Some(report.statistics.depth as u64));
        for x in 1..=4 {
            for y in 0..=2 {
                assert!(report.nodes.iter().any(|node| node.state.values
                    == vec![tla_model::Value::Int(x), tla_model::Value::Int(y)]));
            }
        }
    }
}
