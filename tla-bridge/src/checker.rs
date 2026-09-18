use std::{cell::Cell, marker::PhantomData};
use tla_engine::TransitionSystem;
use tla_ir::{Builtin, ExprId, Program};
pub use tla_oracle::ValueKey;
use tla_sany_semantic::{NodeId, nodes::NodeData};
use tla_tlc_eval::{NamedAction, TlcState, context::Context};
use tla_tlc_value::{TlcError, Value};

use crate::model::{Model, ModelError, Stage, State};
pub(crate) use crate::value_key;

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
        kind: &tla_engine::ViolationKind,
    ) -> Result<bool, ModelError> {
        use tla_engine::ViolationKind;
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
                .map_err(|e| crate::evaluation_error(Stage::Canonicalization, e))
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
    /// tla_bridge::with_model(tla_bridge::ModelRequest::new("Counter.tla"), |model| {
    ///     tla_engine::explore_parallel(&model.system()?, &Default::default(),
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
            .map_err(|e| crate::evaluation_error(Stage::Canonicalization, e))
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
        calculate().map_err(|e| crate::evaluation_error(Stage::Canonicalization, e))
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
                .map_err(|e| crate::evaluation_error(Stage::Properties, e))?
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
    ) -> Result<Vec<tla_engine::Transition<Self::State>>, Self::Error> {
        self.model
            .successors(state)?
            .into_iter()
            .map(|transition| {
                Ok(tla_engine::Transition {
                    action: usize::try_from(transition.action.0).map_err(|_| {
                        ModelError::message(Stage::Successors, "negative action identifier")
                    })?,
                    state: transition.state,
                })
            })
            .collect()
    }
    fn visit_initial(
        &self,
        out: &mut impl FnMut(Self::State) -> crate::VisitResult,
    ) -> crate::VisitResult {
        self.model.begin_initial();
        let mut visit =
            crate::Visit::new(|raw| out(self.model.complete_state(raw, Stage::InitialStates)?));
        let result = self.model.tool.get_init_states(&mut visit);
        visit.finish(result, Stage::InitialStates)
    }
    fn visit_successors(
        &self,
        state: &Self::State,
        out: &mut impl FnMut(tla_engine::Transition<Self::State>) -> crate::VisitResult,
    ) -> crate::VisitResult {
        self.model.owns(state)?;
        self.model.begin_successors(state)?;
        for action in &self.model.processed.actions {
            let id = usize::try_from(action.id).map_err(|_| {
                ModelError::message(Stage::Successors, "negative action identifier")
            })?;
            let mut visit = crate::Visit::new(|mut raw: TlcState| {
                raw.set_predecessor(&state.raw);
                raw.set_action(Some(action.id));
                out(tla_engine::Transition {
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

pub fn from_faithful(value: &Value) -> Result<tla_values::Value, TlcError> {
    use tla_values::Value as New;
    let map_error = |e: tla_values::Error| TlcError::runtime(e.to_string());
    value.deep_normalize()?;
    Ok(match value {
        Value::Bool(v) => New::Bool(*v),
        Value::Int(v) => New::Int(*v),
        Value::Str(v) => New::string(v.val.as_str(), v.val.tok() as u32),
        Value::Model(v) => New::model(v.val.as_str(), v.val.tok() as u32),
        _ => {
            if let Some(f) = value.to_fcn_rcd()? {
                New::function(
                    f.domain_as_values()?
                        .iter()
                        .zip(f.values().iter())
                        .map(|(k, v)| Ok((from_faithful(k)?, from_faithful(v)?)))
                        .collect::<Result<_, TlcError>>()?,
                )
                .map_err(map_error)?
            } else if let Some(s) = value.to_set_enum()? {
                New::set(
                    s.elems()
                        .iter()
                        .map(from_faithful)
                        .collect::<Result<_, _>>()?,
                )
                .map_err(map_error)?
            } else {
                return Err(TlcError::runtime("value is not finite"));
            }
        }
    })
}

pub fn to_faithful(value: &tla_values::Value) -> Result<Value, TlcError> {
    use tla_values::Value as New;
    Ok(match value {
        New::Bool(v) => Value::Bool(*v),
        New::Int(v) => Value::Int(*v),
        New::String(a) | New::Model(a) => {
            let token = tla_tlc_value::UniqueString::of(&a.text);
            if token.tok() as u32 != a.ordinal {
                return Err(TlcError::runtime(
                    "atom ordinal belongs to another intern session",
                ));
            }
            if matches!(value, New::String(_)) {
                Value::string(&a.text)
            } else {
                tla_tlc_value::value::mv_make(&a.text)
            }
        }
        New::Set(v) => Value::set_enum(v.iter().map(to_faithful).collect::<Result<_, _>>()?, true),
        New::Function(v) => Value::fcn_rcd(
            v.iter()
                .map(|(k, _)| to_faithful(k))
                .collect::<Result<_, _>>()?,
            v.iter()
                .map(|(_, v)| to_faithful(v))
                .collect::<Result<_, _>>()?,
            true,
        ),
    })
}

pub fn intern_value(text: &str, model: bool) -> Result<tla_values::Value, TlcError> {
    from_faithful(&if model {
        tla_tlc_value::value::mv_make(text)
    } else {
        Value::string(text)
    })
}

pub fn with_value_session<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tla_tlc_eval::in_fresh_jvm(move || {
        tla_tlc_value::value::mv_init();
        f()
    })
}

impl Model {
    pub fn constant_expression(
        &self,
        name: &str,
    ) -> Result<(Program, ExprId, Result<tla_values::Value, String>), ModelError> {
        let graph = self.tool.graph();
        let id = graph
            .node_ids()
            .find(|id| {
                matches!(graph.node(*id).data, NodeData::OpDef(_))
                    && graph.name(*id).is_some_and(|value| *value == name)
            })
            .ok_or_else(|| ModelError::message(Stage::Load, "unknown expression definition"))?;
        let NodeData::OpDef(definition) = &self.tool.graph().node(id).data else {
            return Err(ModelError::message(
                Stage::Load,
                "expected expression definition",
            ));
        };
        let body = definition
            .body
            .ok_or_else(|| ModelError::message(Stage::Load, "missing expression body"))?;
        let mut lower = crate::lower::Lower::new(self);
        let expression = lower.node(
            body,
            &Context::empty(),
            &std::collections::HashMap::new(),
            0,
        )?;
        let program = lower.finish()?;
        let value = self
            .tool
            .eval(
                body,
                &Context::empty(),
                &TlcState::Empty,
                &TlcState::Empty,
                0,
            )
            .and_then(|value| crate::checker::from_faithful(&value))
            .map_err(|e| e.to_string());
        Ok((program, expression, value))
    }
}

impl Model {
    pub(crate) fn lower_symmetry(
        &self,
    ) -> Result<Vec<Vec<(tla_values::Value, tla_values::Value)>>, ModelError> {
        let mut result = Vec::new();
        for permutation in self.processed.symmetry_perms.iter().flatten() {
            let mut entries = Vec::new();
            for source in tla_tlc_value::value::mvs().unwrap_or_default() {
                if let Some(target) = permutation.get(&source) {
                    let source = from_faithful(&Value::Model(source))
                        .map_err(|e| ModelError::message(Stage::Canonicalization, e))?;
                    let target = from_faithful(&Value::Model(target))
                        .map_err(|e| ModelError::message(Stage::Canonicalization, e))?;
                    entries.push((source, target));
                }
            }
            result.push(entries);
        }
        Ok(result)
    }
}

pub(crate) use tla_model::literal;

impl crate::lower::Lower<'_> {
    pub(super) fn standard_builtin(&self, operator: NodeId, context: &Context) -> Option<Builtin> {
        let graph = self.model.tool.graph();
        let name = graph.name(operator)?;
        let mut op = Builtin::from_name(name.as_str())?;
        if let NodeData::OpDef(definition) = &graph.node(operator).data {
            let module = definition
                .originally_defined_in
                .and_then(|id| graph.name(id));
            op = op.in_module(module.as_ref().map(|name| name.as_str()))?;
        }
        let standard = match self.model.tool.lookup(operator, context, false) {
            tla_tlc_eval::context::Obj::Value(Value::Method(method)) => {
                let descriptor = method.op.description();
                if op == Builtin::Ge && descriptor.contains("tlc2.module.Naturals.GEQ(") {
                    op = Builtin::NaturalsGe;
                }
                true
            }
            tla_tlc_eval::context::Obj::Node(found) if found == operator => {
                graph.node(operator).kind == tla_sany_semantic::nodes::kinds::BUILT_IN_KIND
            }
            tla_tlc_eval::context::Obj::Value(value) => from_faithful(&value).is_err(),
            _ => false,
        };
        standard.then_some(op)
    }
    pub(super) fn method_origin(&mut self, id: ExprId, op: NodeId, context: &Context) -> ExprId {
        if let tla_tlc_eval::context::Obj::Value(Value::Method(method)) =
            self.model.tool.lookup(op, context, false)
        {
            self.program.methods.insert(id, method.op.description());
        }
        id
    }
}
