use std::{fmt, num::NonZeroUsize, ops::ControlFlow};
use tlaplus_engine::{Transition, TransitionSystem};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    PrepareParallel,
    InitialStates,
    Successors,
    Canonicalization,
    SameState,
    StateConstraint,
    ActionConstraint,
    Invariant,
    InitialProperty,
    ActionProperty,
}

#[derive(Clone)]
pub struct Failure<S, E> {
    pub phase: Phase,
    pub state: Option<S>,
    pub next: Option<S>,
    pub error: E,
}
impl<S, E: fmt::Debug> fmt::Debug for Failure<S, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Failure")
            .field("phase", &self.phase)
            .field("error", &self.error)
            .finish()
    }
}
impl<S, E: fmt::Display> fmt::Display for Failure<S, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.phase, self.error)
    }
}
impl<S, E: std::error::Error + 'static> std::error::Error for Failure<S, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

pub struct Diagnosed<'a, T>(pub &'a T);
impl<T: TransitionSystem> Diagnosed<'_, T> {
    fn call<R>(
        &self,
        phase: Phase,
        state: Option<&T::State>,
        next: Option<&T::State>,
        result: Result<R, T::Error>,
    ) -> Result<R, Failure<T::State, T::Error>> {
        result.map_err(|error| Failure {
            phase,
            state: state.cloned(),
            next: next.cloned(),
            error,
        })
    }
}
impl<T: TransitionSystem> TransitionSystem for Diagnosed<'_, T>
where
    T::Error: Clone,
{
    type State = T::State;
    type Key = T::Key;
    type Error = Failure<T::State, T::Error>;

    fn prepare_parallel(&self, workers: NonZeroUsize) -> Result<(), Self::Error> {
        self.call(
            Phase::PrepareParallel,
            None,
            None,
            self.0.prepare_parallel(workers),
        )
    }
    fn prefetch_successors(&self) -> bool {
        self.0.prefetch_successors()
    }
    fn parallel_callbacks(&self) -> bool {
        self.0.parallel_callbacks()
    }
    fn initial_states(&self) -> Result<Vec<T::State>, Self::Error> {
        self.call(Phase::InitialStates, None, None, self.0.initial_states())
    }
    fn successors(&self, state: &T::State) -> Result<Vec<Transition<T::State>>, Self::Error> {
        self.call(
            Phase::Successors,
            Some(state),
            None,
            self.0.successors(state),
        )
    }
    fn visit_initial(
        &self,
        out: &mut impl FnMut(T::State) -> Result<ControlFlow<()>, Self::Error>,
    ) -> Result<ControlFlow<()>, Self::Error> {
        let mut callback_error = None;
        let result = self.0.visit_initial(&mut |state| {
            out(state).map_err(|error| {
                let source = error.error.clone();
                callback_error = Some(error);
                source
            })
        });
        if let Some(error) = callback_error {
            return Err(error);
        }
        self.call(Phase::InitialStates, None, None, result)
    }
    fn visit_successors(
        &self,
        state: &T::State,
        out: &mut impl FnMut(Transition<T::State>) -> Result<ControlFlow<()>, Self::Error>,
    ) -> Result<ControlFlow<()>, Self::Error> {
        let mut callback_error = None;
        let result = self.0.visit_successors(state, &mut |edge| {
            out(edge).map_err(|error| {
                let source = error.error.clone();
                callback_error = Some(error);
                source
            })
        });
        if let Some(error) = callback_error {
            return Err(error);
        }
        self.call(Phase::Successors, Some(state), None, result)
    }
    fn key(&self, state: &T::State) -> Result<T::Key, Self::Error> {
        self.call(
            Phase::Canonicalization,
            Some(state),
            None,
            self.0.key(state),
        )
    }
    fn same_state(&self, a: &T::State, b: &T::State) -> Result<bool, Self::Error> {
        self.call(Phase::SameState, Some(a), Some(b), self.0.same_state(a, b))
    }
    fn state_allowed(&self, state: &T::State) -> Result<bool, Self::Error> {
        self.call(
            Phase::StateConstraint,
            Some(state),
            None,
            self.0.state_allowed(state),
        )
    }
    fn step_allowed(&self, a: &T::State, b: &T::State) -> Result<bool, Self::Error> {
        self.call(
            Phase::ActionConstraint,
            Some(a),
            Some(b),
            self.0.step_allowed(a, b),
        )
    }
    fn invariant(&self, state: &T::State) -> Result<Option<String>, Self::Error> {
        self.call(Phase::Invariant, Some(state), None, self.0.invariant(state))
    }
    fn initial_property(&self, state: &T::State) -> Result<Option<String>, Self::Error> {
        self.call(
            Phase::InitialProperty,
            Some(state),
            None,
            self.0.initial_property(state),
        )
    }
    fn action_property(&self, a: &T::State, b: &T::State) -> Result<Option<String>, Self::Error> {
        self.call(
            Phase::ActionProperty,
            Some(a),
            Some(b),
            self.0.action_property(a, b),
        )
    }
}
