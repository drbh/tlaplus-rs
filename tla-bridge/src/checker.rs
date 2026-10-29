use std::{cell::Cell, marker::PhantomData};
use tlaplus_engine::TransitionSystem;
use tla_sany_semantic::NodeId;
use tla_tlc_eval::{NamedAction, TlcState, context::Context};
use tla_tlc_value::{TlcError, Value};

use crate::model::{Model, ModelError, Stage, State};
use crate::values::{ValueKey, value_key};

pub struct System<'model> {
    model: &'model Model,
    serial: PhantomData<Cell<()>>,
}

impl Model {
    pub(crate) fn cache_seed(&self, id: NodeId) -> Value {
        self.initial_context.borrow().cache_seed(id)
    }
    /// Replays raw state records against this configured model. This checks a
    /// finite safety witness, not minimality or liveness/fairness.
    pub fn validate_safety_records(
        &self,
        records: &[(i64, ValueKey)],
        kind: &tlaplus_engine::ViolationKind,
    ) -> Result<bool, ModelError> {
        use tlaplus_engine::ViolationKind;
        if records.is_empty()
            || records
                .iter()
                .enumerate()
                .any(|(i, (n, _))| *n != i as i64 + 1)
        {
            return Ok(false);
        }
        let record_key = |state: &State<'_>| {
            value_key(&self.tool.state_record(&state.raw))
                .map_err(|e| crate::model::evaluation_error(Stage::Canonicalization, e))
        };
        let system = self.safety_adapter();
        let mut current = None;
        let _ = system.visit_initial(&mut |state| {
            if record_key(&state)? == records[0].1 {
                current = Some(state);
                return Ok(std::ops::ControlFlow::Break(()));
            }
            Ok(std::ops::ControlFlow::Continue(()))
        })?;
        let Some(mut current) = current else {
            return Ok(false);
        };
        let mut previous = None;
        for (index, (_, key)) in records.iter().enumerate().skip(1) {
            if !self.state_allowed(&current)? {
                return Ok(false);
            }
            let mut next = None;
            let _ = system.visit_successors(&current, &mut |transition| {
                if record_key(&transition.state)? == *key {
                    next = Some(transition.state);
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                Ok(std::ops::ControlFlow::Continue(()))
            })?;
            let Some(next) = next else {
                return Ok(false);
            };
            if (index + 1 < records.len() || *kind == ViolationKind::Deadlock)
                && !self.step_allowed(&current, &next)?
            {
                return Ok(false);
            }
            previous = Some(current);
            current = next;
        }
        let (properties, name, from, to) = match kind {
            ViolationKind::Invariant(name) => (
                &self.processed.invariants,
                name,
                &current.raw,
                &TlcState::Empty,
            ),
            ViolationKind::InitialProperty(name) if records.len() == 1 => (
                &self.processed.implied_inits,
                name,
                &current.raw,
                &TlcState::Empty,
            ),
            ViolationKind::ActionProperty(name) if previous.is_some() => (
                &self.processed.implied_actions,
                name,
                &previous.as_ref().unwrap().raw,
                &current.raw,
            ),
            ViolationKind::Deadlock => {
                return Ok(self.state_allowed(&current)? && self.successors(&current)?.is_empty());
            }
            _ => return Ok(false),
        };
        let Some(property) = properties.iter().find(|(_, candidate)| candidate == name) else {
            return Ok(false);
        };
        Ok(self
            .property_failure(std::slice::from_ref(property), from, to)?
            .is_some())
    }

    /// The faithful session adapter cannot be used by the parallel evaluator.
    /// ```compile_fail
    /// tlaplus_bridge::with_model(tlaplus_bridge::ModelRequest::new("Counter.tla"), |model| {
    ///     tlaplus_engine::explore_parallel(&model.system()?, &Default::default(),
    ///         std::num::NonZeroUsize::new(2).unwrap())?;
    ///     Ok(())
    /// });
    /// ```
    pub fn system(&self) -> Result<System<'_>, ModelError> {
        self.adapter(true)
    }

    /// Safety checks only. Temporal properties stay undecided for liveness.
    pub fn safety_adapter(&self) -> System<'_> {
        self.adapter(false).expect("safety adapter")
    }

    fn adapter(&self, reject_temporal: bool) -> Result<System<'_>, ModelError> {
        if reject_temporal && !self.info().temporal_properties.is_empty() {
            return Err(ModelError::message(
                Stage::Properties,
                "the safety engine cannot check temporal properties",
            ));
        }
        Ok(System {
            model: self,
            serial: PhantomData,
        })
    }

    pub fn state_values(&self, state: &State<'_>) -> Result<Vec<ValueKey>, ModelError> {
        self.owns(state)?;
        state
            .raw
            .values()
            .iter()
            .map(|value| value_key(value.as_ref().expect("complete state")))
            .collect::<Result<_, _>>()
            .map_err(|e| crate::model::evaluation_error(Stage::Canonicalization, e))
    }

    pub fn canonical_key(&self, state: &State<'_>) -> Result<Vec<ValueKey>, ModelError> {
        self.owns(state)?;
        let calculate = || -> Result<_, TlcError> {
            let values: Vec<_> = state
                .raw
                .values()
                .iter()
                .map(|v| v.clone().expect("complete state"))
                .collect();
            let mut minimum = values.clone();
            let mut changed = false;
            for permutation in self.processed.symmetry_perms.iter().flatten() {
                let candidate = values
                    .iter()
                    .map(|v| v.permute(permutation))
                    .collect::<Result<Vec<_>, _>>()?;
                for (left, right) in candidate.iter().zip(&minimum) {
                    match left.compare_to(right)? {
                        0 => continue,
                        n if n < 0 => {
                            minimum = candidate;
                            changed = true;
                            break;
                        }
                        _ => break,
                    }
                }
            }
            if let Some(view) = self.processed.view {
                for value in &values {
                    value.deep_normalize()?;
                }
                let canonical = if changed {
                    let mut canonical = self.tool.empty_state();
                    for (name, value) in self.processed.variables.iter().zip(minimum) {
                        canonical.bind(&self.tool.name(*name), value)?;
                    }
                    canonical
                } else {
                    state.raw.clone()
                };
                let view =
                    self.tool
                        .eval(view, &Context::empty(), &canonical, &TlcState::Empty, 0)?;
                Ok(vec![value_key(&view)?])
            } else {
                minimum.iter().map(value_key).collect()
            }
        };
        calculate().map_err(|e| crate::model::evaluation_error(Stage::Canonicalization, e))
    }

    fn property_failure(
        &self,
        properties: &[NamedAction],
        from: &TlcState,
        to: &TlcState,
    ) -> Result<Option<String>, ModelError> {
        for (property, name) in properties {
            if !self
                .tool
                .is_valid_action(property, from, to)
                .map_err(|e| crate::model::evaluation_error(Stage::Properties, e))?
            {
                return Ok(Some(name.clone()));
            }
        }
        Ok(None)
    }
}

impl<'model> TransitionSystem for System<'model> {
    type State = State<'model>;
    type Key = Vec<ValueKey>;
    type Error = ModelError;

    fn initial_states(&self) -> Result<Vec<Self::State>, Self::Error> {
        self.model.initial_states()
    }
    fn successors(
        &self,
        state: &Self::State,
    ) -> Result<Vec<tlaplus_engine::Transition<Self::State>>, Self::Error> {
        self.model
            .successors(state)?
            .into_iter()
            .map(|transition| {
                Ok(tlaplus_engine::Transition {
                    action: usize::try_from(transition.action.0).map_err(|_| {
                        ModelError::message(Stage::Successors, "negative action identifier")
                    })?,
                    state: transition.state,
                })
            })
            .collect()
    }
    fn visit_initial(&self, out: &mut impl FnMut(Self::State) -> VisitResult) -> VisitResult {
        self.model.begin_initial();
        let mut visit =
            Visit::new(|raw| out(self.model.complete_state(raw, Stage::InitialStates)?));
        let result = self.model.tool.get_init_states(&mut visit);
        visit.finish(result, Stage::InitialStates)
    }
    fn visit_successors(
        &self,
        state: &Self::State,
        out: &mut impl FnMut(tlaplus_engine::Transition<Self::State>) -> VisitResult,
    ) -> VisitResult {
        self.model.owns(state)?;
        self.model.begin_successors(state)?;
        for action in &self.model.processed.actions {
            let id = usize::try_from(action.id).map_err(|_| {
                ModelError::message(Stage::Successors, "negative action identifier")
            })?;
            let mut visit = Visit::new(|mut raw: TlcState| {
                raw.set_predecessor(&state.raw);
                raw.set_action(Some(action.id));
                out(tlaplus_engine::Transition {
                    action: id,
                    state: self.model.complete_state(raw, Stage::Successors)?,
                })
            });
            let result = self
                .model
                .tool
                .get_next_states(action, &state.raw, &mut visit);
            if visit.finish(result, Stage::Successors)?.is_break() {
                return Ok(std::ops::ControlFlow::Break(()));
            }
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }
    fn key(&self, state: &Self::State) -> Result<Self::Key, Self::Error> {
        self.model.canonical_key(state)
    }
    fn same_state(&self, left: &Self::State, right: &Self::State) -> Result<bool, Self::Error> {
        left.same_values(right)
    }
    fn state_allowed(&self, state: &Self::State) -> Result<bool, Self::Error> {
        self.model.state_allowed(state)
    }
    fn step_allowed(&self, from: &Self::State, to: &Self::State) -> Result<bool, Self::Error> {
        self.model.step_allowed(from, to)
    }
    fn invariant(&self, state: &Self::State) -> Result<Option<String>, Self::Error> {
        self.model.owns(state)?;
        self.model.property_failure(
            &self.model.processed.invariants,
            &state.raw,
            &TlcState::Empty,
        )
    }
    fn initial_property(&self, state: &Self::State) -> Result<Option<String>, Self::Error> {
        self.model.owns(state)?;
        self.model.property_failure(
            &self.model.processed.implied_inits,
            &state.raw,
            &TlcState::Empty,
        )
    }
    fn action_property(
        &self,
        from: &Self::State,
        to: &Self::State,
    ) -> Result<Option<String>, Self::Error> {
        self.model.owns(from)?;
        self.model.owns(to)?;
        self.model
            .property_failure(&self.model.processed.implied_actions, &from.raw, &to.raw)
    }
}

type VisitResult = Result<std::ops::ControlFlow<()>, ModelError>;
struct Visit<F> {
    out: F,
    stopped: Option<VisitResult>,
}
impl<F: FnMut(tla_tlc_eval::TlcState) -> VisitResult> Visit<F> {
    fn new(out: F) -> Self {
        Self { out, stopped: None }
    }
    fn emit(&mut self, state: tla_tlc_eval::TlcState) -> Result<(), TlcError> {
        if self.stopped.is_none() {
            match (self.out)(state) {
                Ok(std::ops::ControlFlow::Continue(())) => return Ok(()),
                stopped => self.stopped = Some(stopped),
            }
        }
        Err(TlcError::runtime("bridge visitor stopped"))
    }
    fn finish(self, result: Result<(), TlcError>, stage: Stage) -> VisitResult {
        self.stopped.unwrap_or_else(|| {
            result
                .map(|()| std::ops::ControlFlow::Continue(()))
                .map_err(|error| crate::model::evaluation_error(stage, error))
        })
    }
}
impl<F: FnMut(tla_tlc_eval::TlcState) -> VisitResult> tla_tlc_eval::nextstate::InitFunctor
    for Visit<F>
{
    fn add_element(&mut self, state: tla_tlc_eval::TlcState) -> Result<(), TlcError> {
        self.emit(state)
    }
}
impl<F: FnMut(tla_tlc_eval::TlcState) -> VisitResult> tla_tlc_eval::nextstate::NextFunctor
    for Visit<F>
{
    fn add_element(
        &mut self,
        _: &tla_tlc_eval::TlcState,
        _: &tla_tlc_eval::spec::Action,
        state: &tla_tlc_eval::TlcState,
    ) -> Result<(), TlcError> {
        self.emit(state.clone())
    }
}
