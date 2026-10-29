use std::{convert::Infallible, hint::black_box, num::NonZeroUsize, sync::Arc, time::Instant};
use tlaplus_disk::{Codec, CodecError};
use tlaplus_engine::{Options, Transition, TransitionSystem};

#[derive(Clone)]
struct State {
    id: usize,
    payload: Arc<[u8]>,
}
struct Model {
    states: usize,
    bytes: usize,
    work: usize,
}
impl Model {
    fn state(&self, id: usize) -> State {
        State {
            id,
            payload: vec![id as u8; self.bytes].into(),
        }
    }
}
impl TransitionSystem for Model {
    type State = State;
    type Key = usize;
    type Error = Infallible;
    fn initial_states(&self) -> Result<Vec<State>, Infallible> {
        Ok(vec![self.state(0)])
    }
    fn successors(&self, state: &State) -> Result<Vec<Transition<State>>, Infallible> {
        let mut value = state.id as u64;
        for _ in 0..self.work {
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
        }
        black_box(value);
        Ok([state.id * 2 + 1, state.id * 2 + 2]
            .into_iter()
            .enumerate()
            .map(|(action, id)| Transition {
                action,
                state: self.state(id % self.states),
            })
            .collect())
    }
    fn key(&self, state: &State) -> Result<usize, Infallible> {
        Ok(state.id)
    }
    fn same_state(&self, a: &State, b: &State) -> Result<bool, Infallible> {
        Ok(a.id == b.id && a.payload == b.payload)
    }
}
struct Encoding;
impl Codec<State, usize> for Encoding {
    fn state(&self, state: &State) -> Result<Vec<u8>, CodecError> {
        let mut bytes = state.id.to_le_bytes().to_vec();
        bytes.extend(state.payload.iter());
        Ok(bytes)
    }
    fn key(&self, key: &usize) -> Result<Vec<u8>, CodecError> {
        Ok(key.to_le_bytes().to_vec())
    }
    fn decode(&self, bytes: &[u8]) -> Result<State, CodecError> {
        let width = std::mem::size_of::<usize>();
        if bytes.len() < width {
            return Err(CodecError::Truncated);
        }
        Ok(State {
            id: usize::from_le_bytes(bytes[..width].try_into().unwrap()),
            payload: bytes[width..].into(),
        })
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [mode, count, bytes, work, workers] = args.as_slice() else {
        return Err("usage: scale memory|disk STATES PAYLOAD_BYTES WORK_ITERATIONS WORKERS".into());
    };
    let model = Model {
        states: count.parse()?,
        bytes: bytes.parse()?,
        work: work.parse()?,
    };
    if model.states == 0 {
        return Err("states must be positive".into());
    }
    let workers: NonZeroUsize = workers.parse()?;
    let options = Options::default();
    let start = Instant::now();
    let (distinct, depth, generated) = if mode == "disk" {
        if workers.get() != 1 {
            return Err("disk benchmark is serial".into());
        }
        let directory = tempfile::tempdir()?;
        let store = tlaplus_disk::Store::open(
            directory.path().join("states.redb"),
            NonZeroUsize::new(256 << 10).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        let report = tlaplus_disk::explore(&model, &Encoding, &store, &options)?;
        (
            report.distinct as usize,
            report.statistics.depth,
            report.statistics.generated,
        )
    } else if mode == "memory" {
        let report = tlaplus_engine::explore_parallel(&model, &options, workers)?;
        (
            report.nodes.len(),
            report.statistics.depth,
            report.statistics.generated,
        )
    } else {
        return Err("unknown mode".into());
    };
    assert_eq!(distinct, model.states);
    println!(
        "mode={mode} workers={workers} states={distinct} depth={depth} generated={generated} seconds={:.6} payload_bytes={}",
        start.elapsed().as_secs_f64(),
        model.states * model.bytes
    );
    Ok(())
}
