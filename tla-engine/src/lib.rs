//! Graph exploration over any [`TransitionSystem`]: breadth-first search for
//! reachable states, serial or parallel, checking invariants and properties as
//! states are generated and reporting the first violations with their traces.
//! The engine knows nothing about TLA+.
mod parallel;
mod serial;

use std::hash::Hash;
use std::num::NonZeroUsize;
use std::time::Duration;

pub use parallel::explore_parallel;
use serial::search;
pub use serial::validate_trace;

/// Exploration metadata travels with values but does not affect identity.
#[derive(Clone, Debug)]
pub struct Leveled<S> {
    pub values: S,
    pub depth: usize,
}
impl<S> From<S> for Leveled<S> {
    fn from(values: S) -> Self {
        Self { values, depth: 1 }
    }
}
impl<S> std::ops::Deref for Leveled<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.values
    }
}
impl<S> std::ops::DerefMut for Leveled<S> {
    fn deref_mut(&mut self) -> &mut S {
        &mut self.values
    }
}
impl<S: PartialEq> PartialEq for Leveled<S> {
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}
impl<S: Eq> Eq for Leveled<S> {}
impl<S: std::hash::Hash> std::hash::Hash for Leveled<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.values.hash(state);
    }
}
impl<S: IntoIterator> IntoIterator for Leveled<S> {
    type Item = S::Item;
    type IntoIter = S::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}
impl<'a, S> IntoIterator for &'a Leveled<S>
where
    &'a S: IntoIterator,
{
    type Item = <&'a S as IntoIterator>::Item;
    type IntoIter = <&'a S as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&self.values).into_iter()
    }
}
impl<A, S: FromIterator<A>> FromIterator<A> for Leveled<S> {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        S::from_iter(iter).into()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition<S> {
    pub action: usize,
    pub state: S,
}

/// Keys identify quotient states; `same_state` compares raw states for trace validation.
pub trait TransitionSystem {
    type State: Clone;
    type Key: Eq + Hash;
    type Error;

    /// False preserves callback interleaving and validates traces by bounded execution replay.
    fn prefetch_successors(&self) -> bool {
        true
    }

    /// Run streaming generation and its callbacks together on pool workers.
    fn parallel_callbacks(&self) -> bool {
        false
    }

    fn prepare_parallel(&self, _: NonZeroUsize) -> Result<(), Self::Error> {
        Ok(())
    }
    fn initial_states(&self) -> Result<Vec<Self::State>, Self::Error>;
    fn successors(&self, state: &Self::State) -> Result<Vec<Transition<Self::State>>, Self::Error>;
    fn visit_initial(
        &self,
        out: &mut impl FnMut(Self::State) -> Result<std::ops::ControlFlow<()>, Self::Error>,
    ) -> Result<std::ops::ControlFlow<()>, Self::Error> {
        for state in self.initial_states()? {
            if out(state)?.is_break() {
                return Ok(std::ops::ControlFlow::Break(()));
            }
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }
    fn visit_successors(
        &self,
        state: &Self::State,
        out: &mut impl FnMut(Transition<Self::State>) -> Result<std::ops::ControlFlow<()>, Self::Error>,
    ) -> Result<std::ops::ControlFlow<()>, Self::Error> {
        for transition in self.successors(state)? {
            if out(transition)?.is_break() {
                return Ok(std::ops::ControlFlow::Break(()));
            }
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }
    fn key(&self, state: &Self::State) -> Result<Self::Key, Self::Error>;
    fn same_state(&self, left: &Self::State, right: &Self::State) -> Result<bool, Self::Error>;
    fn state_allowed(&self, _: &Self::State) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn step_allowed(&self, _: &Self::State, _: &Self::State) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn invariant(&self, _: &Self::State) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }
    fn initial_property(&self, _: &Self::State) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }
    fn action_property(
        &self,
        _: &Self::State,
        _: &Self::State,
    ) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub check_deadlock: bool,
    pub max_states: Option<NonZeroUsize>,
    pub record_graph: bool,
    /// TLC's `-continue`: invariant and action-property violations are recorded in
    /// `Report::violations` and exploration proceeds; other violations still stop.
    pub continue_after_violation: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            check_deadlock: true,
            max_states: None,
            record_graph: false,
            continue_after_violation: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trace<S> {
    pub states: Vec<S>,
    pub actions: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    InitialProperty(String),
    Invariant(String),
    ActionProperty(String),
    Deadlock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Violation<S> {
    pub kind: ViolationKind,
    pub trace: Trace<S>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Completion<S> {
    Complete,
    Violated(Violation<S>),
    StateLimit,
}

#[derive(Clone, Debug)]
pub struct Node<S> {
    pub expanded: bool,
    pub state: S,
    pub parent: Option<(usize, usize)>,
    pub depth: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub action: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Statistics {
    pub initial_distinct: usize,
    pub generated: usize,
    pub depth: usize,
    pub peak_frontier: usize,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct Report<S> {
    pub completion: Completion<S>,
    /// The first violation of each property, in discovery order, when continuing.
    pub violations: Vec<Violation<S>>,
    pub nodes: Vec<Node<S>>,
    pub edges: Vec<Edge>,
    pub statistics: Statistics,
}

impl<S: Clone> Report<S> {
    pub fn trace_to(&self, mut index: usize) -> Trace<S> {
        let mut trace = Trace {
            states: Vec::new(),
            actions: Vec::new(),
        };
        loop {
            let node = &self.nodes[index];
            trace.states.push(node.state.clone());
            match node.parent {
                Some((parent, action)) => {
                    trace.actions.push(action);
                    index = parent;
                }
                None => break,
            }
        }
        trace.states.reverse();
        trace.actions.reverse();
        trace
    }
}

pub fn explore<T: TransitionSystem>(
    system: &T,
    options: &Options,
) -> Result<Report<T::State>, T::Error> {
    serial::explore(system, options)
}
