//! Development-only access to the faithful evaluator and its run observations.

pub mod checker;
pub mod lower;
pub mod model;

use std::collections::{BTreeMap, HashMap};

use tla_ir::ExprId;
use tla_model::BoundModel;
use tla_oracle::ValueKey;
use tla_tlc_eval::context::Context;
use tla_tlc_eval::mp::{self, Record};
use tla_tlc_value::{TlcError, Value};

pub use model::{Model, ModelError, with_loaded_model, with_model, with_register_model};
pub use tla_oracle::{Answers, Comparison, Difference, Mode, Outcome, compare, corpus, tool};
pub use tla_tlc::args::{RunMode as FaithfulRunMode, TlcArgs as FaithfulArgs};
pub use tla_tlc::{RunEnv, RunOutcome};
pub use tla_tlc_eval::mp as recording;

#[derive(Clone, Default)]
pub(crate) struct LowerCache {
    pub operators: std::collections::HashMap<tla_sany_semantic::NodeId, usize>,
    pub names: std::collections::HashMap<String, usize>,
    pub functions: std::collections::HashMap<tla_sany_semantic::NodeId, tla_ir::ExprId>,
}

pub fn run_faithful(args: Vec<String>, env: RunEnv) -> Result<Answers, String> {
    model::ensure_outside_session().map_err(|e| e.to_string())?;
    let outcome = tla_tlc::run(args, env);
    if !outcome.unsupported.is_empty() {
        return Err(format!(
            "faithful port unsupported: {:?}",
            outcome.unsupported
        ));
    }
    tool::extract_records(
        outcome
            .records
            .iter()
            .map(|r| (r.code, r.params.as_slice())),
        Some(outcome.exit_status),
    )
}

pub use tla_oracle::{
    Counterexample, ErrorCategory, ObservedRun, Property, PropertyKind, PropertyObservation,
    ReportedState, RunError, TraceEnd, Verdict,
};

/// Like `run_faithful`, with named verdict evidence and full reported trace bodies.
/// Unsupported features and unclassified failures remain visible in the result.
pub fn observe_faithful(args: Vec<String>, env: RunEnv) -> Result<ObservedRun, String> {
    crate::model::ensure_outside_session().map_err(|e| e.to_string())?;
    let (outcome, captured) = tla_tlc::driver::run_inspected(args, env, |tool| {
        let model = tool.processed_spec();
        let inventory = [
            (PropertyKind::Invariant, model.invariants),
            (PropertyKind::Initial, model.implied_inits),
            (PropertyKind::Action, model.implied_actions),
            (PropertyKind::Temporal, model.implied_temporals),
        ]
        .into_iter()
        .flat_map(|(kind, properties)| {
            properties
                .into_iter()
                .map(move |(_, name)| Property { kind, name })
        })
        .collect();
        let states = tla_tlc::te::take_captured_states().map(|states| {
            states
                .into_iter()
                .map(|(ordinal, record)| {
                    checker::value_key(&record)
                        .map(|value| (ordinal, value))
                        .map_err(|e| e.to_string())
                })
                .collect()
        });
        (inventory, states)
    });
    let mut observed = from_outcome(&outcome)?;
    if let Some((inventory, states)) = captured {
        observed.configured_properties = Some(inventory);
        if observed.summary.mode == Mode::Bfs {
            observed.typed_states = states;
        }
    }
    Ok(observed)
}

pub fn from_outcome(outcome: &RunOutcome) -> Result<ObservedRun, String> {
    let mut summary = tool::extract_records(
        outcome
            .records
            .iter()
            .map(|r| (r.code, r.params.as_slice())),
        Some(outcome.exit_status),
    )?;
    if !outcome.unsupported.is_empty() {
        summary.outcome = Outcome::Unknown;
    }
    let mut properties = BTreeMap::new();
    let mut counterexamples = Vec::new();
    let mut pending = Vec::new();
    let mut current: Option<Counterexample> = None;
    let mut deadlock_reported = false;
    for record in &outcome.records {
        let kind = match record.code {
            mp::TLC_INVARIANT_VIOLATED_INITIAL
            | mp::TLC_INVARIANT_VIOLATED_BEHAVIOR
            | mp::TLC_INVARIANT_EVALUATION_FAILED => Some(PropertyKind::Invariant),
            mp::TLC_PROPERTY_VIOLATED_INITIAL => Some(PropertyKind::Initial),
            mp::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR
            | mp::TLC_ACTION_PROPERTY_EVALUATION_FAILED => Some(PropertyKind::Action),
            mp::TLC_TEMPORAL_PROPERTY_VIOLATED => Some(PropertyKind::Temporal),
            _ => None,
        };
        if let Some(kind) = kind {
            let names = if kind == PropertyKind::Temporal {
                record.params.as_slice()
            } else {
                std::slice::from_ref(parameter(record, 0)?)
            };
            pending = names
                .iter()
                .map(|name| Property {
                    kind,
                    name: name.clone(),
                })
                .collect();
            let verdict = if matches!(
                record.code,
                mp::TLC_INVARIANT_EVALUATION_FAILED | mp::TLC_ACTION_PROPERTY_EVALUATION_FAILED
            ) {
                Verdict::EvaluationError
            } else {
                Verdict::Violated
            };
            for property in &pending {
                // A later evaluation failure does not erase an observed violation.
                properties
                    .entry(property.clone())
                    .and_modify(|old| {
                        if *old != Verdict::Violated {
                            *old = verdict;
                        }
                    })
                    .or_insert(verdict);
            }
        }
        match record.code {
            mp::TLC_INVARIANT_VIOLATED_INITIAL | mp::TLC_PROPERTY_VIOLATED_INITIAL => {
                finish_trace(&mut current, &mut counterexamples);
                counterexamples.push(Counterexample {
                    properties: pending.clone(),
                    states: vec![ReportedState {
                        ordinal: 1,
                        display: parameter(record, 1)?.clone(),
                    }],
                    end: TraceEnd::Finite,
                });
            }
            mp::TLC_DEADLOCK_REACHED => {
                deadlock_reported = true;
                pending.clear();
            }
            mp::TLC_BEHAVIOR_UP_TO_THIS_POINT | mp::TLC_COUNTER_EXAMPLE => {
                finish_trace(&mut current, &mut counterexamples);
            }
            mp::TLC_STATE_PRINT1 | mp::TLC_STATE_PRINT2 => {
                let ordinal = ordinal(record, 1)?;
                if ordinal == 1 {
                    finish_trace(&mut current, &mut counterexamples);
                }
                current
                    .get_or_insert_with(|| Counterexample {
                        properties: pending.clone(),
                        ..Counterexample::default()
                    })
                    .states
                    .push(ReportedState {
                        ordinal,
                        display: parameter(record, 0)?.clone(),
                    });
            }
            mp::TLC_STATE_PRINT3 | mp::TLC_BACK_TO_STATE => {
                // In -tool mode back-to-state uses printState's [stateInfo, n].
                // Outside -tool it uses printMessage's [n, action].
                let index = if record.code == mp::TLC_BACK_TO_STATE
                    && parameter(record, 0)?.parse::<i64>().is_ok()
                {
                    0
                } else {
                    1
                };
                let ordinal = ordinal(record, index)?;
                let trace = current.get_or_insert_with(|| Counterexample {
                    properties: pending.clone(),
                    ..Counterexample::default()
                });
                trace.end = if record.code == mp::TLC_STATE_PRINT3 {
                    TraceEnd::Stuttering { ordinal }
                } else {
                    TraceEnd::BackTo { ordinal }
                };
                finish_trace(&mut current, &mut counterexamples);
            }
            _ => {}
        }
    }
    finish_trace(&mut current, &mut counterexamples);
    let error = classify_error(outcome.error_code).map(|category| RunError {
        category,
        code: outcome.error_code,
        parameters: outcome
            .records
            .iter()
            .rev()
            .find(|r| r.code == outcome.error_code)
            .map(|r| r.params.clone())
            .unwrap_or_default(),
    });
    Ok(ObservedRun {
        typed_states: None,
        summary,
        configured_properties: None,
        properties: properties
            .into_iter()
            .map(|(property, verdict)| PropertyObservation { property, verdict })
            .collect(),
        counterexamples,
        error,
        unsupported: outcome.unsupported.clone(),
        exit_status: outcome.exit_status,
        deadlock_reported,
    })
}

fn parameter(record: &Record, index: usize) -> Result<&String, String> {
    record
        .params
        .get(index)
        .ok_or_else(|| format!("missing parameter {index} in record {}", record.code))
}

fn ordinal(record: &Record, index: usize) -> Result<i64, String> {
    parameter(record, index)?
        .parse()
        .map_err(|_| format!("invalid state ordinal in record {}", record.code))
}

fn finish_trace(current: &mut Option<Counterexample>, out: &mut Vec<Counterexample>) {
    if let Some(trace) = current.take() {
        out.push(trace);
    }
}

fn classify_error(code: i32) -> Option<ErrorCategory> {
    use ErrorCategory as E;
    match code {
        0
        | mp::TLC_INVARIANT_VIOLATED_INITIAL
        | mp::TLC_INVARIANT_VIOLATED_BEHAVIOR
        | mp::TLC_PROPERTY_VIOLATED_INITIAL
        | mp::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR
        | mp::TLC_TEMPORAL_PROPERTY_VIOLATED
        | mp::TLC_DEADLOCK_REACHED
        | mp::TLC_ASSUMPTION_FALSE => None,
        mp::TLC_PARSING_FAILED | mp::TLC_PARSING_FAILED2 => Some(E::ParseOrSemantic),
        mp::TLC_ASSUMPTION_EVALUATION_ERROR => Some(E::AssumptionEvaluation),
        mp::SYSTEM_OUT_OF_MEMORY
        | mp::SYSTEM_OUT_OF_MEMORY_TOO_MANY_INIT
        | mp::SYSTEM_OUT_OF_MEMORY_LIVENESS
        | mp::SYSTEM_STACK_OVERFLOW => Some(E::Resource),
        mp::TLC_INVARIANT_EVALUATION_FAILED
        | mp::TLC_ACTION_PROPERTY_EVALUATION_FAILED
        | mp::TLC_MODULE_DIVISION_BY_ZERO
        | mp::TLC_MODULE_ARGUMENT_ERROR
        | mp::TLC_MODULE_ARGUMENT_ERROR_AN
        | mp::TLC_STATE_NOT_COMPLETELY_SPECIFIED_INITIAL
        | mp::TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT => Some(E::Evaluation),
        _ => Some(match tla_tlc::driver::exit_status(code) {
            75 | 76 => E::Evaluation,
            151 => E::Configuration,
            _ => E::Other,
        }),
    }
}

pub use tla_oracle::ModelRequest;

impl Model {
    #[rustfmt::skip]    pub fn random_state(&self) -> u64 { tla_tlc_value::random::with_random(|random| random.seed_state()) }

    pub(crate) fn capture_setup(&self) {
        self.initial_context.replace(self.tool.runtime_snapshot());
        self.initial_random
            .replace(tla_tlc_value::random::with_random(|r| r.clone()));
    }
    pub(crate) fn begin_initial(&self) {
        self.tool
            .restore_runtime(self.initial_context.borrow().clone());
        tla_tlc_value::random::set_current_state(None);
        let saved = self.initial_random.borrow().clone();
        tla_tlc_value::random::with_random(|r| *r = saved);
    }
}

pub(crate) fn operator_table(
    table: &tla_tlc_value::value::OpRcdValue,
) -> Result<tla_ir::Literal, ModelError> {
    let convert = |value: &tla_tlc_value::Value| {
        checker::from_faithful(value)
            .map(|value| checker::literal(&value))
            .map_err(|e| ModelError::message(model::Stage::Load, e))
    };
    let entries = table
        .domain()
        .into_iter()
        .zip(table.values())
        .map(|(args, value)| {
            let args = args.iter().map(convert).collect::<Result<_, _>>()?;
            Ok((tla_ir::Literal::Tuple(args), convert(&value)?))
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    let mut unique = Vec::new();
    for entry in entries {
        if !unique.iter().any(|(key, _)| key == &entry.0) {
            unique.push(entry);
        }
    }
    Ok(tla_ir::Literal::Function(unique))
}

pub(crate) fn value_key(value: &Value) -> Result<ValueKey, TlcError> {
    value.deep_normalize()?;
    Ok(match value {
        Value::Bool(value) => ValueKey::Bool(*value),
        Value::Int(value) => ValueKey::Int(*value),
        Value::Str(value) => ValueKey::String(value.val.as_str().into()),
        Value::Model(value) => ValueKey::Model(value.val.as_str().into()),
        _ => {
            if let Some(function) = value.to_fcn_rcd()? {
                let mut entries = function
                    .domain_as_values()?
                    .iter()
                    .zip(function.values().iter())
                    .map(|(key, value)| Ok((value_key(key)?, value_key(value)?)))
                    .collect::<Result<Vec<_>, TlcError>>()?;
                entries.sort();
                ValueKey::Function(entries)
            } else if let Some(set) = value.to_set_enum()? {
                let mut members = set
                    .elems()
                    .iter()
                    .map(value_key)
                    .collect::<Result<Vec<_>, _>>()?;
                members.sort();
                members.dedup();
                ValueKey::Set(members)
            } else {
                return Err(TlcError::runtime(format!(
                    "cannot canonicalize {}",
                    value.kind_string()
                )));
            }
        }
    })
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
    fn finish(self, result: Result<(), TlcError>, stage: model::Stage) -> VisitResult {
        self.stopped.unwrap_or_else(|| {
            result
                .map(|()| std::ops::ControlFlow::Continue(()))
                .map_err(|error| crate::evaluation_error(stage, error))
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

pub(crate) fn append_setup_diagnostics(
    error: &mut ModelError,
    records: &[tla_tlc_eval::mp::Record],
) {
    let details = records
        .iter()
        .rev()
        .take(4)
        .map(|r| tla_tlc_eval::mp::get_message(r.code, &r.params))
        .collect::<Vec<_>>();
    if !details.is_empty() {
        error
            .message
            .push_str(&format!(" [{}]", details.join(" | ")));
    }
}

impl Model {
    pub fn first_failed_assumption(&self) -> Option<(usize, Result<bool, ModelError>)> {
        self.processed
            .assumptions
            .iter()
            .enumerate()
            .find_map(|(index, (id, axiom))| {
                if *axiom {
                    return None;
                }
                match self.tool.is_valid_assumption(*id) {
                    Ok(true) => None,
                    result => Some((
                        index,
                        result.map_err(|error| {
                            crate::evaluation_error(model::Stage::Assumptions, error)
                        }),
                    )),
                }
            })
    }
}

impl Model {
    pub fn lower_assumptions(&self) -> Result<(BoundModel, Vec<(usize, ExprId)>), ModelError> {
        let mut lower = lower::Lower::new(self);
        let mut assumptions = Vec::new();
        for (index, (id, axiom)) in self.processed.assumptions.iter().enumerate() {
            if !axiom {
                assumptions.push((
                    index,
                    lower.node(*id, &Context::empty(), &HashMap::new(), 0)?,
                ));
            }
        }
        let program = lower.finish()?;
        program
            .validate()
            .map_err(|e| ModelError::message(model::Stage::Load, e))?;
        let mut model = BoundModel::new(program);
        self.seed_context(&mut model)?;
        Ok((model, assumptions))
    }
}

impl lower::Lower<'_> {
    pub(crate) fn builtin(
        &mut self,
        op: tla_ir::Builtin,
        args: Vec<ExprId>,
        literal_message: bool,
        level: tla_ir::Level,
    ) -> Result<ExprId, ModelError> {
        self.program
            .checked_apply(op, args, literal_message, level)
            .map_err(|e| ModelError::message(model::Stage::Load, e))
    }

    pub(crate) fn finish(mut self) -> Result<tla_ir::Program, ModelError> {
        self.program
            .specialize_calls()
            .map_err(|e| ModelError::message(model::Stage::Load, e))?;
        Ok(self.program)
    }
}

pub(crate) fn evaluation_error(stage: model::Stage, error: TlcError) -> ModelError {
    let (code, message) = tla_tlc_eval::error_code_and_message(&error);
    ModelError {
        stage,
        code: Some(code),
        message,
    }
}
