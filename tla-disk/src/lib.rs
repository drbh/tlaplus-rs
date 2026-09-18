//! Exploration with the state graph on disk, for models whose reachable
//! states exceed memory. [`Store`] keeps nodes in an embedded database and
//! [`explore`] drives the same search as the engine through it.
pub mod store;
pub use store::Store;
mod codec;
pub use codec::{CodecError, StateCodec, ValueCodec};
use std::{fmt, time::Instant};
use store::Node;
use tla_engine::{
    Completion, Options, Statistics, Trace, TransitionSystem, Violation, ViolationKind,
};

/// How states and their keys are laid out in the store.
pub trait Codec<S, K> {
    fn state(&self, state: &S) -> Result<Vec<u8>, CodecError>;
    fn key(&self, key: &K) -> Result<Vec<u8>, CodecError>;
    fn decode(&self, bytes: &[u8]) -> Result<S, CodecError>;
}

#[derive(Debug)]
pub enum Error<E> {
    Evaluation(E),
    Storage(store::Error),
    Codec(CodecError),
    Configuration(String),
    /// The store and the search disagree; a bug, not a user error.
    Internal(&'static str),
    /// A feature disk exploration does not implement.
    Unsupported(&'static str),
}
impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
            Self::Codec(e) => e.fmt(f),
            Self::Configuration(e) => e.fmt(f),
            Self::Internal(what) => write!(f, "internal error: {what}"),
            Self::Unsupported(what) => f.write_str(what),
        }
    }
}
impl<E: fmt::Debug + fmt::Display> std::error::Error for Error<E> {}
impl<E> From<store::Error> for Error<E> {
    fn from(e: store::Error) -> Self {
        Self::Storage(e)
    }
}

#[derive(Debug)]
pub struct Report<S> {
    pub completion: Completion<S>,
    pub distinct: u64,
    pub statistics: Statistics,
}

/// Breadth-first exploration with the state graph in `store`, which must be
/// empty. States are expanded in the order the store assigned them, so the
/// store doubles as the queue. `Report.distinct` is the number of states
/// stored, which excludes states a constraint rejected.
pub fn explore<T: TransitionSystem, C: Codec<T::State, T::Key>>(
    system: &T,
    codec: &C,
    store: &Store,
    options: &Options,
) -> Result<Report<T::State>, Error<T::Error>> {
    if !store.is_empty()? {
        return Err(Error::Configuration(
            "disk exploration requires an empty store".into(),
        ));
    }
    if options.record_graph {
        return Err(Error::Unsupported(
            "disk graph recording is not implemented",
        ));
    }
    let mut search = Search {
        system,
        codec,
        store,
        options,
        stats: Statistics::default(),
        start: Instant::now(),
    };
    if let Some(completion) = search.initial()? {
        return search.finish(completion);
    }
    let mut from = 0;
    loop {
        let end = store.len()?;
        if from == end {
            return search.finish(Completion::Complete);
        }
        search.stats.peak_frontier = search.stats.peak_frontier.max((end - from) as usize);
        for id in from..end {
            if let Some(completion) = search.expand(id)? {
                return search.finish(completion);
            }
        }
        from = end;
    }
}

/// What became of a generated state.
enum Admission {
    /// Rejected by a constraint, or already stored.
    Skipped,
    Stored,
    Violated(ViolationKind),
    /// Not stored: the state limit was reached.
    Limit,
}

struct Search<'a, T, C> {
    system: &'a T,
    codec: &'a C,
    store: &'a Store,
    options: &'a Options,
    stats: Statistics,
    start: Instant,
}

impl<T: TransitionSystem, C: Codec<T::State, T::Key>> Search<'_, T, C> {
    fn finish(
        &self,
        completion: Completion<T::State>,
    ) -> Result<Report<T::State>, Error<T::Error>> {
        let mut statistics = self.stats.clone();
        statistics.elapsed = self.start.elapsed();
        Ok(Report {
            completion,
            distinct: self.store.len()?,
            statistics,
        })
    }

    /// Generates and admits the initial states. `Some` ends exploration early.
    fn initial(&mut self) -> Result<Option<Completion<T::State>>, Error<T::Error>> {
        use Error::Evaluation;
        let system = self.system;
        let mut completion = None;
        stream(
            |out| system.visit_initial(&mut |state| out(state)),
            |state| {
                let allowed = system.state_allowed(&state).map_err(Evaluation)?;
                let admission = self.admit(&state, allowed, None, |seen| {
                    // An initial state seen before was checked when first stored.
                    if seen {
                        return Ok(None);
                    }
                    Ok(match system.invariant(&state)? {
                        Some(name) => Some(ViolationKind::Invariant(name)),
                        None => system
                            .initial_property(&state)?
                            .map(ViolationKind::InitialProperty),
                    })
                })?;
                Ok(match admission {
                    Admission::Violated(kind) => {
                        let trace = Trace {
                            states: vec![state],
                            actions: vec![],
                        };
                        completion = Some(Completion::Violated(Violation { kind, trace }));
                        std::ops::ControlFlow::Break(())
                    }
                    Admission::Limit => {
                        completion = Some(Completion::StateLimit);
                        std::ops::ControlFlow::Break(())
                    }
                    Admission::Stored | Admission::Skipped => std::ops::ControlFlow::Continue(()),
                })
            },
        )?;
        Ok(completion)
    }

    /// Pops state `id` from the queue and admits its successors. `Some` ends
    /// exploration early: a violation, a deadlock, or the state limit.
    fn expand(&mut self, id: u64) -> Result<Option<Completion<T::State>>, Error<T::Error>> {
        use Error::{Codec, Evaluation};
        let (system, codec, store) = (self.system, self.codec, self.store);
        let (popped, node) = store
            .pop()?
            .ok_or(Error::Internal("disk queue ended prematurely"))?;
        if popped != id {
            return Err(Error::Internal("disk queue order changed"));
        }
        let state = codec.decode(&node.state).map_err(Codec)?;
        let mut produced = false;
        let mut completion = None;
        stream(
            |out| system.visit_successors(&state, &mut |transition| out(transition)),
            |transition| {
                produced = true;
                let allowed = system
                    .state_allowed(&transition.state)
                    .map_err(Evaluation)?
                    && system
                        .step_allowed(&state, &transition.state)
                        .map_err(Evaluation)?;
                let parent = Some((id, transition.action as u64, node.depth + 1));
                let admission = self.admit(&transition.state, allowed, parent, |seen| {
                    // A state seen before has had its invariants checked;
                    // the step into it still has to satisfy action properties.
                    let invariant = if seen {
                        None
                    } else {
                        system.invariant(&transition.state)?
                    };
                    Ok(match invariant {
                        Some(name) => Some(ViolationKind::Invariant(name)),
                        None => system
                            .action_property(&state, &transition.state)?
                            .map(ViolationKind::ActionProperty),
                    })
                })?;
                Ok(match admission {
                    Admission::Violated(kind) => {
                        let mut trace = trace(store, codec, id)?;
                        trace.states.push(transition.state);
                        trace.actions.push(transition.action);
                        completion = Some(Completion::Violated(Violation { kind, trace }));
                        std::ops::ControlFlow::Break(())
                    }
                    Admission::Limit => {
                        completion = Some(Completion::StateLimit);
                        std::ops::ControlFlow::Break(())
                    }
                    Admission::Stored | Admission::Skipped => std::ops::ControlFlow::Continue(()),
                })
            },
        )?;
        if completion.is_none() && self.options.check_deadlock && !produced {
            completion = Some(Completion::Violated(Violation {
                kind: ViolationKind::Deadlock,
                trace: trace(store, codec, id)?,
            }));
        }
        Ok(completion)
    }

    /// Admits one generated state. `violation` is consulted for every
    /// generated state, allowed or not, as the in-memory engine does, and is
    /// told whether the state was seen before. A disallowed or seen state is
    /// then skipped; otherwise the state is stored under its parent unless
    /// the state limit has been reached. `parent` is the parent's store id,
    /// the action taken, and the new state's depth.
    fn admit(
        &mut self,
        state: &T::State,
        allowed: bool,
        parent: Option<(u64, u64, u64)>,
        violation: impl FnOnce(bool) -> Result<Option<ViolationKind>, T::Error>,
    ) -> Result<Admission, Error<T::Error>> {
        use Error::{Codec, Evaluation};
        self.stats.generated += 1;
        let key = allowed
            .then(|| {
                self.system
                    .key(state)
                    .map_err(Evaluation)
                    .and_then(|k| self.codec.key(&k).map_err(Codec))
            })
            .transpose()?;
        let seen = key
            .as_ref()
            .map(|key| self.store.find(key))
            .transpose()?
            .flatten()
            .is_some();
        if let Some(kind) = violation(seen).map_err(Evaluation)? {
            return Ok(Admission::Violated(kind));
        }
        let Some(key) = key.filter(|_| !seen) else {
            return Ok(Admission::Skipped);
        };
        if limited(self.store, self.options)? {
            return Ok(Admission::Limit);
        }
        let depth = parent.map_or(1, |(_, _, depth)| depth);
        self.store.insert(
            &key,
            &Node {
                state: self.codec.state(state).map_err(Codec)?,
                parent: parent.map(|(id, action, _)| (id, action)),
                depth,
            },
        )?;
        if parent.is_none() {
            self.stats.initial_distinct += 1;
        }
        self.stats.depth = self.stats.depth.max(depth as usize);
        Ok(Admission::Stored)
    }
}

fn limited<E>(store: &Store, options: &Options) -> Result<bool, Error<E>> {
    match options.max_states {
        Some(limit) => Ok(store.len()? >= limit.get() as u64),
        None => Ok(false),
    }
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
