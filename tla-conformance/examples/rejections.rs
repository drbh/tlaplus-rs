pub mod support;

use serde::{Deserialize, Serialize};
use std::{fs, num::NonZeroUsize, time::Duration};
use tlaplus_bridge::{values::ValueKey, with_register_model as with_model};
use tlaplus_conformance::{Case, Execution, Request, execute_suite};
use tlaplus_engine::{explore, explore_parallel};
use tlaplus_model::State;
use tlaplus_observe::{Diagnosed, Failure};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StateKey {
    level: i32,
    values: Vec<ValueKey>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Witness {
    phase: String,
    state: Option<StateKey>,
    next: Option<StateKey>,
    category: Option<String>,
    detail: String,
}
impl Witness {
    fn matches(&self, other: &Self) -> bool {
        self.category.is_some() && self == other
    }
}
#[derive(Debug, Deserialize, Serialize)]
struct Comparison {
    expected: Witness,
    serial: Witness,
    parallel: Witness,
    error_answers_equal: bool,
}
#[derive(Debug, Deserialize, Serialize)]
struct Measurement {
    comparison: Option<Comparison>,
    failure: Option<String>,
}
impl Measurement {
    fn matches(&self) -> bool {
        self.failure.is_none()
            && self
                .comparison
                .as_ref()
                .is_some_and(|c| c.error_answers_equal)
    }
}
use support::{key, rejection};

fn native(error: Failure<State, tlaplus_model::Error>) -> Result<Witness, String> {
    let state = |state: State| -> Result<StateKey, String> {
        Ok(StateKey {
            level: i32::try_from(state.depth).map_err(|e| e.to_string())?,
            values: state.iter().map(key).collect(),
        })
    };
    let (category, detail) = rejection(&error.error.0, false);
    Ok(Witness {
        phase: format!("{:?}", error.phase),
        state: error.state.map(state).transpose()?,
        next: error.next.map(state).transpose()?,
        category,
        detail,
    })
}
fn measure(request: Request) -> Result<Comparison, String> {
    let (request, options) = support::configuration(&request)?;
    let (model, options, expected) = with_model(request, move |faithful| {
        let model = faithful.lower()?;
        let options = tlaplus_engine::Options {
            check_deadlock: options.check_deadlock && faithful.info().check_deadlock,
            ..options
        };
        let failure = match explore(&Diagnosed(&faithful.safety_adapter()), &options) {
            Err(failure) => failure,
            Ok(_) => return Ok((model, options, None)),
        };
        let state = |s: tlaplus_bridge::model::State<'_>| -> Result<_, tlaplus_bridge::ModelError> {
            Ok(StateKey {
                level: s.level(),
                values: faithful.state_values(&s)?,
            })
        };
        let (category, detail) = rejection(&failure.error.message, true);
        let expected = Witness {
            phase: format!("{:?}", failure.phase),
            state: failure.state.map(state).transpose()?,
            next: failure.next.map(state).transpose()?,
            category,
            detail,
        };
        Ok((model, options, Some(expected)))
    })
    .map_err(|e| e.to_string())?;
    let expected = expected.ok_or("faithful exploration did not reject; use the safety runner")?;
    let mut parallel_model = model.clone();
    parallel_model.runtime.strings = std::sync::Arc::new(std::sync::Mutex::new(
        model
            .runtime
            .strings
            .lock()
            .map_err(|_| "string table poisoned")?
            .clone(),
    ));
    let serial = explore(&Diagnosed(&model), &options)
        .err()
        .ok_or("native serial exploration did not reject")?;
    let parallel = explore_parallel(
        &Diagnosed(&parallel_model),
        &options,
        NonZeroUsize::new(4).unwrap(),
    )
    .err()
    .ok_or("native parallel exploration did not reject")?;
    let (serial, parallel) = (native(serial)?, native(parallel)?);
    let error_answers_equal = expected.matches(&serial) && expected.matches(&parallel);
    Ok(Comparison {
        expected,
        serial,
        parallel,
        error_answers_equal,
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 {
        let request: Request = serde_json::from_slice(&fs::read(&args[0])?)?;
        let result = match measure(request) {
            Ok(comparison) => Measurement {
                comparison: Some(comparison),
                failure: None,
            },
            Err(failure) => Measurement {
                comparison: None,
                failure: Some(failure),
            },
        };
        fs::write(&args[1], serde_json::to_vec_pretty(&result)?)?;
        return Ok(());
    }
    if args.len() != 3 {
        return Err("usage: rejections PLAN.json REPORT.json SECONDS".into());
    }
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(&args[0])?)?;
    let seconds = args[2].to_string_lossy().parse()?;
    let results = execute_suite::<Measurement>(
        &std::env::current_exe()?,
        &cases,
        Duration::from_secs(seconds),
        |id, result| {
            let matched = matches!(result, Execution::Completed(m) if m.matches());
            eprintln!("{}: {id}", if matched { "match" } else { "unresolved" });
        },
    )?;
    let matched = results
        .values()
        .filter(|r| matches!(r, Execution::Completed(m) if m.matches()))
        .count();
    fs::write(&args[1], serde_json::to_vec_pretty(&results)?)?;
    eprintln!(
        "{matched}/{} cases match error category, detail, phase and state with 1/4 workers",
        results.len()
    );
    if matched != results.len() {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unclassified_errors_and_different_witnesses_never_match() {
        let mut a = Witness {
            phase: "Successors".into(),
            state: Some(StateKey {
                level: 1,
                values: vec![ValueKey::Int(0)],
            }),
            next: None,
            category: None,
            detail: "same error".into(),
        };
        assert!(!a.matches(&a));
        a.category = Some("assertion".into());
        assert!(a.matches(&a));
        let mut b = a.clone();
        b.detail = "other assertion".into();
        assert!(!a.matches(&b));
        b = a.clone();
        b.phase = "Invariant".into();
        assert!(!a.matches(&b));
        b = a.clone();
        b.state.as_mut().unwrap().level = 2;
        assert!(!a.matches(&b));
        b = a.clone();
        b.state.as_mut().unwrap().values = vec![ValueKey::Int(1)];
        assert!(!a.matches(&b));
    }
    #[test]
    fn recognized_rejections_match_at_each_exploration_phase() {
        for (config, phase) in [
            ("AssertionInit", "InitialStates"),
            ("AssertionNext", "Successors"),
            ("AssertionInvariant", "Invariant"),
            ("AssertionConstraint", "StateConstraint"),
            ("AssertionAction", "ActionProperty"),
            ("AssertionUnclassified", "Successors"),
            ("EmptyHead", "InitialStates"),
            ("EmptyTail", "Successors"),
            ("EmptyChoose", "Invariant"),
            ("EmptyCase", "StateConstraint"),
            ("InfiniteQuantifier", "Invariant"),
            ("InfiniteSequenceFilter", "Successors"),
            ("InfiniteInitial", "InitialStates"),
            ("DivisionZero", "Successors"),
            ("ModulusZero", "Invariant"),
            ("ModulusNegative", "StateConstraint"),
            ("IncompleteInit", "InitialStates"),
            ("IncompleteNext", "Successors"),
            ("InfinitePowerView", "Canonicalization"),
            ("InfinitePowerPrint", "Invariant"),
            ("InfiniteCardinality", "Invariant"),
            ("InvalidTuple", "Invariant"),
            ("InvalidRecord", "Invariant"),
            ("InvalidInterval", "Invariant"),
            ("InvalidFunction", "Invariant"),
            ("IntegerCast", "Invariant"),
            ("IntegerHigherCast", "Invariant"),
            ("NaturalCast", "Invariant"),
            ("NaturalHigherCast", "Invariant"),
            ("BooleanStringIf", "Invariant"),
            ("BooleanStringNot", "Invariant"),
            ("BooleanModelIf", "Invariant"),
            ("BooleanModelNot", "Invariant"),
            ("BooleanMemberIf", "Invariant"),
            ("BooleanRenderIf", "Invariant"),
            ("BooleanInitialIf", "InitialStates"),
            ("BooleanNextIf", "Successors"),
            ("BooleanEnabledIf", "Invariant"),
            ("BooleanFunctionIf", "Successors"),
            ("SequenceBadBound", "Invariant"),
            ("SequenceBadLen", "Invariant"),
            ("SequenceBadAppend", "Invariant"),
            ("SequenceBadTail", "Invariant"),
            ("RuntimeMissing", "Invariant"),
            ("RuntimeMissingIndex", "Invariant"),
            ("RuntimeMissingName", "Invariant"),
        ] {
            let request = Request {
                args: vec![
                    "-config".into(),
                    config.into(),
                    if config.starts_with("Assertion") {
                        "AssertionFailures"
                    } else if config.starts_with("Incomplete") {
                        "IncompleteStates"
                    } else if config.starts_with("Invalid") {
                        "InvalidSelection"
                    } else if config.starts_with("Integer") {
                        "IntegerComparisons"
                    } else if config.starts_with("Natural") {
                        "NaturalComparisons"
                    } else if config.starts_with("Boolean") {
                        "BooleanContexts"
                    } else if config.starts_with("SequenceBad") {
                        "SequenceBounds"
                    } else if config.starts_with("RuntimeMissing") {
                        "RuntimeReads"
                    } else if config.starts_with("InfinitePower") || config == "InfiniteCardinality"
                    {
                        "InfiniteOperators"
                    } else {
                        "RejectionDomains"
                    }
                    .into(),
                ],
                user_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../tests/fixtures"),
                library_paths: vec![],
                community_modules: false,
                system_properties: vec![],
                engine: false,
            };
            let result = measure(request).unwrap_or_else(|error| panic!("{config}: {error}"));
            assert_eq!(result.expected.phase, phase);
            if config.starts_with("Incomplete") {
                assert_eq!(
                    result.expected.category.as_deref(),
                    Some("incomplete_state")
                );
                assert_eq!(result.serial.detail, "x, y");
            }
            assert_eq!(
                result.error_answers_equal,
                config != "AssertionUnclassified",
                "{config}: {result:?}"
            );
        }
    }
}
