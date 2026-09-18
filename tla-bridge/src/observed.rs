//! Whole runs of the faithful checker and the observations parsed from them.

use crate::{Answers, Mode, ModelError, Outcome, RunEnv, RunOutcome, model, tool, values};
use std::collections::BTreeMap;
use std::fmt;
use tla_tlc_eval::mp::{self, Record};

/// Why a faithful run could not be observed.
#[derive(Debug)]
pub enum ObserveError {
    /// A faithful session was already active.
    Session(ModelError),
    /// The faithful port rejected features of the specification.
    Unsupported(Vec<String>),
    /// The run's messages could not be read as answers.
    Oracle(tla_oracle::Error),
    /// A recorded message lacks or mangles a parameter.
    Record(String),
}

impl fmt::Display for ObserveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(e) => e.fmt(f),
            Self::Unsupported(features) => write!(f, "faithful port unsupported: {features:?}"),
            Self::Oracle(e) => e.fmt(f),
            Self::Record(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ObserveError {}

impl From<tla_oracle::Error> for ObserveError {
    fn from(e: tla_oracle::Error) -> Self {
        Self::Oracle(e)
    }
}

pub fn run_faithful(args: Vec<String>, env: RunEnv) -> Result<Answers, ObserveError> {
    model::ensure_outside_session().map_err(ObserveError::Session)?;
    let outcome = tla_tlc::run(args, env);
    if !outcome.unsupported.is_empty() {
        return Err(ObserveError::Unsupported(outcome.unsupported));
    }
    Ok(tool::extract_records(
        outcome
            .records
            .iter()
            .map(|r| (r.code, r.params.as_slice())),
        Some(outcome.exit_status),
    )?)
}

pub use tla_oracle::{
    Counterexample, ErrorCategory, ObservedRun, Property, PropertyKind, PropertyObservation,
    ReportedState, RunError, TraceEnd, Verdict,
};

/// Like `run_faithful`, with named verdict evidence and full reported trace bodies.
/// Unsupported features and unclassified failures remain visible in the result.
pub fn observe_faithful(args: Vec<String>, env: RunEnv) -> Result<ObservedRun, ObserveError> {
    crate::model::ensure_outside_session().map_err(ObserveError::Session)?;
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
                    values::value_key(&record)
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

pub fn from_outcome(outcome: &RunOutcome) -> Result<ObservedRun, ObserveError> {
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

fn parameter(record: &Record, index: usize) -> Result<&String, ObserveError> {
    record.params.get(index).ok_or_else(|| {
        ObserveError::Record(format!(
            "missing parameter {index} in record {}",
            record.code
        ))
    })
}

fn ordinal(record: &Record, index: usize) -> Result<i64, ObserveError> {
    parameter(record, index)?.parse().map_err(|_| {
        ObserveError::Record(format!("invalid state ordinal in record {}", record.code))
    })
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
