mod codec;
pub use codec::{StateCodec, ValueCodec};
use std::{fmt, time::Instant};
use tla_engine::{
    Completion, Options, Statistics, Trace, TransitionSystem, Violation, ViolationKind,
};
use tla_store::{Node, Store};

pub trait Codec<S, K> {
    fn state(&self, state: &S) -> Result<Vec<u8>, String>;
    fn key(&self, key: &K) -> Result<Vec<u8>, String>;
    fn decode(&self, bytes: &[u8]) -> Result<S, String>;
}

#[derive(Debug)]
pub enum Error<E> {
    Evaluation(E),
    Storage(tla_store::Error),
    Codec(String),
    Configuration(String),
}
impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
            Self::Codec(e) | Self::Configuration(e) => e.fmt(f),
        }
    }
}
impl<E: fmt::Debug + fmt::Display> std::error::Error for Error<E> {}
impl<E> From<tla_store::Error> for Error<E> {
    fn from(e: tla_store::Error) -> Self {
        Self::Storage(e)
    }
}

#[derive(Debug)]
pub struct Report<S> {
    pub completion: Completion<S>,
    pub distinct: u64,
    pub statistics: Statistics,
}

pub fn explore<T: TransitionSystem, C: Codec<T::State, T::Key>>(
    system: &T,
    codec: &C,
    store: &Store,
    options: &Options,
) -> Result<Report<T::State>, Error<T::Error>> {
    use Error::{Codec, Evaluation};
    if !store.is_empty()? {
        return Err(Error::Configuration(
            "disk exploration requires an empty store".into(),
        ));
    }
    if options.record_graph {
        return Err(Error::Configuration(
            "disk graph recording is not implemented".into(),
        ));
    }
    let start = Instant::now();
    let mut stats = Statistics::default();
    let finish = |mut statistics: Statistics, completion| -> Result<Report<_>, Error<T::Error>> {
        statistics.elapsed = start.elapsed();
        Ok(Report {
            completion,
            distinct: store.len()?,
            statistics,
        })
    };
    let mut initial_completion = None;
    stream(
        |out| system.visit_initial(&mut |state| out(state)),
        |state| {
            stats.generated += 1;
            let allowed = system.state_allowed(&state).map_err(Evaluation)?;
            let key = allowed
                .then(|| {
                    system
                        .key(&state)
                        .map_err(Evaluation)
                        .and_then(|k| codec.key(&k).map_err(Codec))
                })
                .transpose()?;
            if let Some(key) = &key
                && store.find(key)?.is_some()
            {
                return Ok(std::ops::ControlFlow::Continue(()));
            }
            let kind = match system.invariant(&state).map_err(Evaluation)? {
                Some(name) => Some(ViolationKind::Invariant(name)),
                None => system
                    .initial_property(&state)
                    .map_err(Evaluation)?
                    .map(ViolationKind::InitialProperty),
            };
            if let Some(kind) = kind {
                initial_completion = Some(Completion::Violated(Violation {
                    kind,
                    trace: Trace {
                        states: vec![state],
                        actions: vec![],
                    },
                }));
                return Ok(std::ops::ControlFlow::Break(()));
            }
            if let Some(key) = key {
                if limited(store, options)? {
                    initial_completion = Some(Completion::StateLimit);
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                store.insert(
                    &key,
                    &Node {
                        state: codec.state(&state).map_err(Codec)?,
                        parent: None,
                        depth: 1,
                    },
                )?;
                stats.initial_distinct += 1;
                stats.depth = 1;
            }
            Ok(std::ops::ControlFlow::Continue(()))
        },
    )?;
    if let Some(completion) = initial_completion {
        return finish(stats, completion);
    }
    let mut from = 0;
    loop {
        let end = store.len()?;
        if from == end {
            return finish(stats, Completion::Complete);
        }
        stats.peak_frontier = stats.peak_frontier.max((end - from) as usize);
        for id in from..end {
            let mut pending = None;
            let (popped, node) = store
                .pop()?
                .ok_or_else(|| Error::Configuration("disk queue ended prematurely".into()))?;
            if popped != id {
                return Err(Error::Configuration("disk queue order changed".into()));
            }
            let state = codec.decode(&node.state).map_err(Codec)?;
            let mut produced = false;
            stream(
                |out| system.visit_successors(&state, &mut |transition| out(transition)),
                |transition| {
                    produced = true;
                    stats.generated += 1;
                    let allowed = system
                        .state_allowed(&transition.state)
                        .map_err(Evaluation)?
                        && system
                            .step_allowed(&state, &transition.state)
                            .map_err(Evaluation)?;
                    let key = allowed
                        .then(|| {
                            system
                                .key(&transition.state)
                                .map_err(Evaluation)
                                .and_then(|k| codec.key(&k).map_err(Codec))
                        })
                        .transpose()?;
                    let seen = key
                        .as_ref()
                        .map(|key| store.find(key))
                        .transpose()?
                        .flatten()
                        .is_some();
                    let invariant = if seen {
                        None
                    } else {
                        system.invariant(&transition.state).map_err(Evaluation)?
                    };
                    let kind = match invariant {
                        Some(name) => Some(ViolationKind::Invariant(name)),
                        None => system
                            .action_property(&state, &transition.state)
                            .map_err(Evaluation)?
                            .map(ViolationKind::ActionProperty),
                    };
                    if let Some(kind) = kind {
                        let mut trace = trace(store, codec, id)?;
                        trace.states.push(transition.state);
                        trace.actions.push(transition.action);
                        pending = Some(Completion::Violated(Violation { kind, trace }));
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                    if let Some(key) = key
                        && !seen
                    {
                        if limited(store, options)? {
                            pending = Some(Completion::StateLimit);
                            return Ok(std::ops::ControlFlow::Break(()));
                        }
                        store.insert(
                            &key,
                            &Node {
                                state: codec.state(&transition.state).map_err(Codec)?,
                                parent: Some((id, transition.action as u64)),
                                depth: node.depth + 1,
                            },
                        )?;
                        stats.depth = stats.depth.max(node.depth as usize + 1);
                    }
                    Ok(std::ops::ControlFlow::Continue(()))
                },
            )?;
            if let Some(completion) = pending {
                return finish(stats, completion);
            }
            if options.check_deadlock && !produced {
                return finish(
                    stats,
                    Completion::Violated(Violation {
                        kind: ViolationKind::Deadlock,
                        trace: trace(store, codec, id)?,
                    }),
                );
            }
        }
        from = end;
    }
}
fn limited<E>(store: &Store, options: &Options) -> Result<bool, Error<E>> {
    Ok(options
        .max_states
        .is_some_and(|n| store.len().is_ok_and(|count| count >= n.get() as u64)))
}
fn trace<S, K, E>(store: &Store, codec: &impl Codec<S, K>, id: u64) -> Result<Trace<S>, Error<E>> {
    let nodes = store.trace(id)?;
    let states = nodes
        .iter()
        .map(|(_, n)| codec.decode(&n.state).map_err(Error::Codec))
        .collect::<Result<_, _>>()?;
    let actions = nodes
        .iter()
        .filter_map(|(_, n)| n.parent.map(|(_, action)| action as usize))
        .collect();
    Ok(Trace { states, actions })
}

fn stream<S, E>(
    generate: impl FnOnce(
        &mut dyn FnMut(S) -> Result<std::ops::ControlFlow<()>, E>,
    ) -> Result<std::ops::ControlFlow<()>, E>,
    mut consume: impl FnMut(S) -> Result<std::ops::ControlFlow<()>, Error<E>>,
) -> Result<(), Error<E>> {
    let mut failure = None;
    let _ = generate(&mut |state| match consume(state) {
        Ok(flow) => Ok(flow),
        Err(error) => {
            failure = Some(error);
            Ok(std::ops::ControlFlow::Break(()))
        }
    })
    .map_err(Error::Evaluation)?;
    failure.map_or(Ok(()), Err)
}
