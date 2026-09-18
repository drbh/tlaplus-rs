use crate::{
    Completion, Edge, Node, Options, Report, Statistics, Trace, Transition, TransitionSystem,
    Violation, ViolationKind,
};
use std::ops::ControlFlow;
use std::{collections::HashMap, num::NonZeroUsize, time::Instant};

fn replay_trace<T: TransitionSystem>(
    system: &T,
    expected: &Violation<T::State>,
) -> Result<bool, T::Error> {
    system.prepare_parallel(NonZeroUsize::MIN)?;
    let options = Options {
        check_deadlock: expected.kind == ViolationKind::Deadlock,
        continue_after_violation: true,
        ..Options::default()
    };
    let depth = expected.trace.states.len() + usize::from(options.check_deadlock);
    let replay = search(
        system,
        &options,
        |states| states.iter().map(|_| None).collect(),
        Some(depth),
        NonZeroUsize::MAX,
    )?;
    let stopped = match replay.completion {
        Completion::Violated(violation) => Some(violation),
        _ => None,
    };
    let mut found = replay.violations.into_iter().chain(stopped);
    let Some(actual) = found.find(|actual| actual.kind == expected.kind) else {
        return Ok(false);
    };
    if actual.trace.actions != expected.trace.actions
        || actual.trace.states.len() != expected.trace.states.len()
    {
        return Ok(false);
    }
    for (actual, expected) in actual.trace.states.iter().zip(&expected.trace.states) {
        if !system.same_state(actual, expected)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn explore<T: TransitionSystem>(
    system: &T,
    options: &Options,
) -> Result<Report<T::State>, T::Error> {
    system.prepare_parallel(NonZeroUsize::MIN)?;
    search(
        system,
        options,
        |states| states.iter().map(|_| None).collect(),
        None,
        NonZeroUsize::MAX,
    )
}

pub fn validate_trace<T: TransitionSystem>(
    system: &T,
    violation: &Violation<T::State>,
) -> Result<bool, T::Error> {
    let trace = &violation.trace;
    if trace.states.is_empty() || trace.actions.len() + 1 != trace.states.len() {
        return Ok(false);
    }
    if !system.prefetch_successors() {
        return replay_trace(system, violation);
    }
    let mut initial = false;
    let _ = system.visit_initial(&mut |state| {
        initial = system.same_state(&state, &trace.states[0])?;
        Ok(if initial {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        })
    })?;
    if !initial {
        return Ok(false);
    }
    for (i, pair) in trace.states.windows(2).enumerate() {
        if !system.state_allowed(&pair[0])? {
            return Ok(false);
        }
        let mut found = false;
        let _ = system.visit_successors(&pair[0], &mut |transition| {
            found = transition.action == trace.actions[i]
                && system.same_state(&transition.state, &pair[1])?;
            Ok(if found {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            })
        })?;
        if !found {
            return Ok(false);
        }
        if (i + 2 < trace.states.len() || violation.kind == ViolationKind::Deadlock)
            && !system.step_allowed(&pair[0], &pair[1])?
        {
            return Ok(false);
        }
    }
    let last = trace.states.last().unwrap();
    Ok(match &violation.kind {
        ViolationKind::InitialProperty(name) => {
            trace.states.len() == 1 && system.initial_property(last)?.as_ref() == Some(name)
        }
        ViolationKind::Invariant(name) => system.invariant(last)?.as_ref() == Some(name),
        ViolationKind::ActionProperty(name) => {
            trace.states.len() > 1
                && system
                    .action_property(&trace.states[trace.states.len() - 2], last)?
                    .as_ref()
                    == Some(name)
        }
        ViolationKind::Deadlock => {
            system.state_allowed(last)? && system.successors(last)?.is_empty()
        }
    })
}

pub(crate) struct Search<'a, T: TransitionSystem> {
    system: &'a T,
    options: &'a Options,
    started: Instant,
    visited: HashMap<T::Key, usize>,
    pub report: Report<T::State>,
    pub next: Vec<usize>,
    pub halted: bool,
    error: Option<T::Error>,
}

impl<'a, T: TransitionSystem> Search<'a, T> {
    pub fn new(system: &'a T, options: &'a Options) -> Result<Self, T::Error> {
        let mut search = Self {
            system,
            options,
            started: Instant::now(),
            visited: HashMap::new(),
            report: Report {
                completion: Completion::Complete,
                violations: Vec::new(),
                nodes: Vec::new(),
                edges: Vec::new(),
                statistics: Statistics::default(),
            },
            next: Vec::new(),
            halted: false,
            error: None,
        };
        let _ = system.visit_initial(&mut |state| search.initial(state))?;
        Ok(search)
    }

    fn stop(&mut self, completion: Completion<T::State>) -> ControlFlow<()> {
        self.report.completion = completion;
        self.halted = true;
        ControlFlow::Break(())
    }

    fn violate(&mut self, violation: Violation<T::State>) -> ControlFlow<()> {
        let recoverable = matches!(
            violation.kind,
            ViolationKind::Invariant(_) | ViolationKind::ActionProperty(_)
        );
        if !(recoverable && self.options.continue_after_violation) {
            return self.stop(Completion::Violated(violation));
        }
        if !self
            .report
            .violations
            .iter()
            .any(|v| v.kind == violation.kind)
        {
            self.report.violations.push(violation);
        }
        ControlFlow::Continue(())
    }

    pub fn fail(&mut self, error: T::Error) {
        if !self.halted {
            self.error = Some(error);
            self.halted = true;
        }
    }

    pub fn finish(mut self) -> Result<Report<T::State>, T::Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        self.report.statistics.elapsed = self.started.elapsed();
        Ok(self.report)
    }

    fn initial(&mut self, state: T::State) -> Result<ControlFlow<()>, T::Error> {
        self.report.statistics.generated += 1;
        let allowed = self.system.state_allowed(&state)?;
        let key = if allowed {
            Some(self.system.key(&state)?)
        } else {
            None
        };
        if key
            .as_ref()
            .is_some_and(|key| self.visited.contains_key(key))
        {
            return Ok(ControlFlow::Continue(()));
        }
        let initial = |kind| Violation {
            kind,
            trace: Trace {
                states: vec![state.clone()],
                actions: vec![],
            },
        };
        if let Some(name) = self.system.invariant(&state)?
            && self
                .violate(initial(ViolationKind::Invariant(name)))
                .is_break()
        {
            return Ok(ControlFlow::Break(()));
        }
        if let Some(name) = self.system.initial_property(&state)? {
            return Ok(self.stop(Completion::Violated(initial(
                ViolationKind::InitialProperty(name),
            ))));
        }
        if let Some(key) = key {
            if self
                .options
                .max_states
                .is_some_and(|n| self.report.nodes.len() >= n.get())
            {
                return Ok(self.stop(Completion::StateLimit));
            }
            let index = self.report.nodes.len();
            self.visited.insert(key, index);
            self.report.nodes.push(Node {
                expanded: false,
                state,
                parent: None,
                depth: 1,
            });
            self.next.push(index);
            self.report.statistics.initial_distinct += 1;
            self.report.statistics.depth = 1;
        }
        Ok(ControlFlow::Continue(()))
    }

    pub fn transition(
        &mut self,
        from: usize,
        transition: Transition<T::State>,
    ) -> Result<ControlFlow<()>, T::Error> {
        self.report.statistics.generated += 1;
        let current = &self.report.nodes[from].state;
        let state = &transition.state;
        let allowed =
            self.system.state_allowed(state)? && self.system.step_allowed(current, state)?;
        let key = if allowed {
            Some(self.system.key(state)?)
        } else {
            None
        };
        let seen = key.as_ref().and_then(|key| self.visited.get(key)).copied();
        let invariant = if seen.is_none() {
            self.system.invariant(state)?
        } else {
            None
        };
        let continuing = self.options.continue_after_violation;
        let action = if invariant.is_none() || continuing {
            self.system.action_property(current, state)?
        } else {
            None
        };
        let failures = [
            invariant.map(ViolationKind::Invariant),
            action.map(ViolationKind::ActionProperty),
        ];
        for kind in failures.into_iter().flatten() {
            let mut trace = self.report.trace_to(from);
            trace.states.push(transition.state.clone());
            trace.actions.push(transition.action);
            if self.violate(Violation { kind, trace }).is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        let Some(key) = key else {
            return Ok(ControlFlow::Continue(()));
        };
        let to = if let Some(to) = seen {
            to
        } else {
            if self
                .options
                .max_states
                .is_some_and(|n| self.report.nodes.len() >= n.get())
            {
                return Ok(self.stop(Completion::StateLimit));
            }
            let to = self.report.nodes.len();
            let depth = self.report.nodes[from].depth + 1;
            self.report.statistics.depth = self.report.statistics.depth.max(depth);
            self.visited.insert(key, to);
            self.report.nodes.push(Node {
                expanded: false,
                state: transition.state,
                parent: Some((from, transition.action)),
                depth,
            });
            self.next.push(to);
            to
        };
        if self.options.record_graph {
            self.report.edges.push(Edge {
                from,
                to,
                action: transition.action,
            });
        }
        Ok(ControlFlow::Continue(()))
    }

    pub fn expanded(&mut self, from: usize, produced: bool) {
        if self.halted {
            return;
        }
        self.report.nodes[from].expanded = true;
        if self.options.check_deadlock && !produced {
            let trace = self.report.trace_to(from);
            let _ = self.stop(Completion::Violated(Violation {
                kind: ViolationKind::Deadlock,
                trace,
            }));
        }
    }
}

pub(crate) fn search<T: TransitionSystem>(
    system: &T,
    options: &Options,
    mut expand: impl FnMut(&[T::State]) -> Vec<Option<Vec<Transition<T::State>>>>,
    max_depth: Option<usize>,
    batch_size: NonZeroUsize,
) -> Result<Report<T::State>, T::Error> {
    let mut search = Search::new(system, options)?;
    'levels: while !search.halted && !search.next.is_empty() {
        let frontier = std::mem::take(&mut search.next);
        if max_depth.is_some_and(|depth| search.report.nodes[frontier[0]].depth >= depth) {
            break;
        }
        search.report.statistics.peak_frontier =
            search.report.statistics.peak_frontier.max(frontier.len());
        for batch in frontier.chunks(batch_size.get()) {
            let states: Vec<_> = batch
                .iter()
                .map(|&i| search.report.nodes[i].state.clone())
                .collect();
            let layers = expand(&states);
            for ((from, current), transitions) in batch.iter().copied().zip(states).zip(layers) {
                let mut produced = false;
                let mut visit = |transition| {
                    produced = true;
                    search.transition(from, transition)
                };
                match transitions {
                    Some(transitions) => {
                        for transition in transitions {
                            if visit(transition)?.is_break() {
                                break;
                            }
                        }
                    }
                    None => {
                        let _ = system.visit_successors(&current, &mut visit)?;
                    }
                }
                search.expanded(from, produced);
                if search.halted {
                    break 'levels;
                }
            }
        }
    }
    search.finish()
}
