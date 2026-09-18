pub mod support;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, num::NonZeroUsize, time::Duration};
use support::{Samples, configuration, intern_values, key, outcome, replay, trace_length};
use tla_bridge::{values::ValueKey, with_register_model as with_model};
use tla_conformance::{Case, Execution, Request, execute_suite};
use tla_disk::{Codec, StateCodec};
use tla_engine::{
    Completion, Options, Report, TransitionSystem, explore, explore_parallel, validate_trace,
};
use tla_model::{BoundModel, State};

#[derive(Debug, Deserialize, Serialize)]
struct Snapshots {
    version: u32,
    strings: std::collections::BTreeMap<String, u32>,
    request: Request,
    #[serde(
        serialize_with = "tla_conformance::snapshots::serialize",
        deserialize_with = "tla_conformance::snapshots::deserialize"
    )]
    samples: Samples,
}

#[derive(Debug, Deserialize, Serialize)]
struct Measurement {
    outcome: String,
    #[serde(default)]
    expected_outcome: String,
    #[serde(default)]
    trace_length: Option<usize>,
    #[serde(default)]
    expected_trace_length: Option<usize>,
    states: usize,
    depth: usize,
    initial: usize,
    #[serde(default)]
    safety_answers_equal: bool,
    faithful_states_equal: bool,
    successor_sets_equal: bool,
    #[serde(default)]
    successor_samples: usize,
    parallel_states_equal: bool,
    traces_valid: bool,
    failure: Option<String>,
    snapshots: Option<Snapshots>,
}
impl Measurement {
    fn matches(&self) -> bool {
        self.failure.is_none()
            && self.outcome != "state limit"
            && self.safety_answers_equal
            && self.successor_sets_equal
            && (!self.outcome.starts_with("success")
                || (self.faithful_states_equal && self.parallel_states_equal))
            && self.traces_valid
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 4 && args[0] == "--compare-recorded" {
        let measurements: std::collections::BTreeMap<String, Execution<Measurement>> =
            serde_json::from_slice(&fs::read(&args[1])?)?;
        let answers = measurements
            .into_iter()
            .map(|(id, result)| {
                let answer = match result {
                    Execution::Completed(m) if m.matches() => {
                        support::answers(&m.outcome, m.initial, m.states, m.depth)
                    }
                    _ => tla_bridge::Answers::default(),
                };
                (id, answer)
            })
            .collect();
        return support::compare_recorded(
            tla_bridge::corpus::Corpus { answers },
            args[2].as_ref(),
            args[3].as_ref(),
        );
    }
    if args.len() == 3 && args[0] == "--replay" {
        let measurement: Measurement =
            serde_json::from_reader(std::io::BufReader::new(fs::File::open(&args[1])?))?;
        let snapshots = measurement.snapshots.ok_or("report has no snapshots")?;
        if !matches!(snapshots.version, 2 | 3) {
            return Err("unsupported snapshot version".into());
        }
        let (request, _) = configuration(&snapshots.request)?;
        let model = with_model(request, |faithful| faithful.lower())?;
        *model
            .runtime
            .strings
            .lock()
            .map_err(|_| "string table poisoned")? = snapshots.strings;
        let matched = replay(&model, &snapshots.samples)?;
        fs::write(
            &args[2],
            serde_json::to_vec_pretty(&serde_json::json!({
                "states": snapshots.samples.len(), "successor_sets_equal": matched,
            }))?,
        )?;
        if !matched {
            return Err("saved-state successor mismatch".into());
        }
        return Ok(());
    }
    if args.len() == 2 {
        let request: Request = serde_json::from_slice(&fs::read(&args[0])?)?;
        let measurement = measure(request).unwrap_or_else(|failure| Measurement {
            outcome: "unresolved".into(),
            expected_outcome: String::new(),
            trace_length: None,
            expected_trace_length: None,
            states: 0,
            depth: 0,
            initial: 0,
            safety_answers_equal: false,
            faithful_states_equal: false,
            successor_sets_equal: false,
            successor_samples: 0,
            parallel_states_equal: false,
            traces_valid: false,
            failure: Some(failure),
            snapshots: None,
        });
        support::write_json(args[1].as_ref(), &measurement)?;
        return Ok(());
    }
    if args.len() != 3 {
        return Err("usage: new_pipeline PLAN.json REPORT.json SECONDS".into());
    }
    let mut cases: Vec<Case> = serde_json::from_slice(&fs::read(&args[0])?)?;
    if let Ok(filter) = std::env::var("REWRITE_FILTER") {
        cases.retain(|case| case.id.contains(&filter));
    }
    let seconds = args[2].to_string_lossy().parse()?;
    let mut results = execute_suite::<Measurement>(
        &std::env::current_exe()?,
        &cases,
        Duration::from_secs(seconds),
        |id, result| {
            eprintln!(
                "{}: {id}",
                match result {
                    Execution::Completed(m) if m.matches() => "match".into(),
                    Execution::Completed(m) => format!(
                        "unresolved ({})",
                        m.failure.as_deref().unwrap_or(&m.outcome)
                    ),
                    Execution::TimedOut => "timeout".into(),
                    Execution::Failed { exit_code, .. } => format!("failed ({exit_code:?})"),
                }
            );
        },
    )?;
    for case in &cases {
        if let Some(Execution::Completed(measurement)) = results.get_mut(&case.id)
            && let Some(snapshots) = &mut measurement.snapshots
        {
            snapshots.request = case.request.clone();
        }
    }
    support::write_json(args[1].as_ref(), &results)?;
    let matches = results
        .values()
        .filter(|result| matches!(result,Execution::Completed(m) if m.matches()))
        .count();
    eprintln!(
        "{matches}/{} cases match safety answers, successor sets and completed state sets with 1/4 workers",
        results.len()
    );
    if matches != results.len() {
        std::process::exit(1);
    }
    Ok(())
}
fn measure(request: Request) -> Result<Measurement, String> {
    let started = std::time::Instant::now();
    let stage = move |name: &str| {
        if std::env::var_os("REWRITE_STAGE_TRACE").is_some() {
            eprintln!("{name}: {:.3}s", started.elapsed().as_secs_f64());
        }
    };
    let (request_model, options) = configuration(&request)?;
    let faithful_options = options.clone();
    stage("faithful setup");
    let detached = with_model(request_model, move |faithful| {
        stage("lowering");
        let model = faithful.lower()?;
        let options = Options {
            check_deadlock: faithful_options.check_deadlock && faithful.info().check_deadlock,
            ..faithful_options
        };
        stage("faithful exploration");
        let report = explore(&faithful.safety_adapter(), &options)?;
        stage("faithful snapshots");
        let mut keys = BTreeSet::new();
        let mut samples = Vec::new();
        let mut pool = std::collections::HashSet::new();
        for node in &report.nodes {
            keys.insert(faithful.canonical_key(&node.state)?);
            if !node.expanded {
                continue;
            }
            let successors = faithful
                .successors(&node.state)?
                .into_iter()
                .map(|transition| {
                    faithful
                        .state_values(&transition.state)
                        .map(|values| intern_values(&mut pool, values))
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            let bytes = StateCodec
                .state(&tla_engine::Leveled {
                    values: node.state.detached_values()?,
                    depth: node.depth,
                })
                .map_err(|message| tla_bridge::model::ModelError {
                    stage: tla_bridge::model::Stage::InspectState,
                    code: None,
                    message,
                })?;
            samples.push((bytes, successors));
        }
        let expected = (
            outcome(&report),
            trace_length(&report),
            report.statistics.depth,
            report.statistics.initial_distinct,
            keys,
        );
        Ok((model, options, expected, samples))
    })
    .map_err(|e| e.to_string())?;
    let (
        model,
        options,
        (expected_outcome, expected_trace, expected_depth, expected_initial, expected_keys),
        samples,
    ) = detached;
    let mut parallel_model = model.clone();
    parallel_model.runtime.strings = std::sync::Arc::new(std::sync::Mutex::new(
        model
            .runtime
            .strings
            .lock()
            .map_err(|_| "string table poisoned")?
            .clone(),
    ));
    stage("native serial");
    let serial = explore(&model, &options).map_err(|e| e.to_string())?;
    stage("native parallel");
    let parallel = explore_parallel(&parallel_model, &options, NonZeroUsize::new(4).unwrap())
        .map_err(|e| e.to_string())?;
    stage("state-set comparison");
    let serial_keys = keys(&model, &serial)?;
    let parallel_keys = keys(&parallel_model, &parallel)?;
    let ordinals_equal = *model
        .runtime
        .strings
        .lock()
        .map_err(|_| "string table poisoned")?
        == *parallel_model
            .runtime
            .strings
            .lock()
            .map_err(|_| "string table poisoned")?;
    stage("successor replay");
    let successor_sets_equal = replay(&model, &samples)?;
    let actual_outcome = outcome(&serial);
    let faithful_states_equal = serial_keys == expected_keys
        && actual_outcome == expected_outcome
        && serial.statistics.depth == expected_depth
        && serial.statistics.initial_distinct == expected_initial;
    let safety_answers_equal = actual_outcome == expected_outcome
        && actual_outcome == outcome(&parallel)
        && trace_length(&serial) == expected_trace
        && trace_length(&parallel) == expected_trace;
    let parallel_states_equal = serial_keys == parallel_keys
        && ordinals_equal
        && serial.statistics.depth == parallel.statistics.depth
        && serial.statistics.initial_distinct == parallel.statistics.initial_distinct
        && actual_outcome == outcome(&parallel);
    let mut traces_valid = true;
    stage("witness replay");
    for (model, report) in [(&model, &serial), (&parallel_model, &parallel)] {
        let stopped = match &report.completion {
            Completion::Violated(violation) => Some(violation),
            _ => None,
        };
        for violation in report.violations.iter().chain(stopped) {
            traces_valid &= validate_trace(model, violation).map_err(|e| e.to_string())?;
        }
    }
    let snapshots = if std::env::var_os("REWRITE_SNAPSHOTS").is_some() {
        Some(Snapshots {
            version: 3,
            strings: model
                .runtime
                .strings
                .lock()
                .map_err(|_| "string table poisoned")?
                .clone(),
            request: request.clone(),
            samples: samples.clone(),
        })
    } else {
        None
    };
    stage("measurement complete");
    Ok(Measurement {
        outcome: actual_outcome,
        expected_outcome,
        trace_length: trace_length(&serial),
        expected_trace_length: expected_trace,
        states: serial.nodes.len(),
        depth: serial.statistics.depth,
        initial: serial.statistics.initial_distinct,
        safety_answers_equal,
        faithful_states_equal,
        successor_sets_equal,
        successor_samples: samples.len(),
        parallel_states_equal,
        traces_valid,
        failure: None,
        snapshots,
    })
}
fn keys(model: &BoundModel, report: &Report<State>) -> Result<BTreeSet<Vec<ValueKey>>, String> {
    report
        .nodes
        .iter()
        .map(|node| {
            model
                .key(&node.state)
                .map(|values| values.iter().map(key).collect())
                .map_err(|e| e.to_string())
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tla_bridge::ModelRequest;

    fn measurement(outcome: &str) -> Measurement {
        Measurement {
            outcome: outcome.into(),
            expected_outcome: outcome.into(),
            states: 1,
            depth: 1,
            initial: 1,
            trace_length: None,
            expected_trace_length: None,
            safety_answers_equal: true,
            faithful_states_equal: true,
            successor_sets_equal: true,
            successor_samples: 0,
            parallel_states_equal: true,
            traces_valid: true,
            failure: None,
            snapshots: None,
        }
    }

    #[test]
    fn random_callbacks_and_canonical_fingerprints_preserve_replay() {
        for config in [
            "RandomModel",
            "RandomModelView",
            "RandomModelSymmetry",
            "RandomModelFailure",
        ] {
            for seed in [1, 42] {
                let request = Request {
                    user_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures"),
                    args: vec![
                        "-config".into(),
                        config.into(),
                        "-seed".into(),
                        seed.to_string(),
                        "-fp".into(),
                        seed.to_string(),
                        "RandomModel".into(),
                    ],
                    system_properties: vec![],
                    library_paths: vec![],
                    community_modules: false,
                    engine: false,
                };
                let result = measure(request).unwrap();
                assert!(result.matches(), "{config}/{seed}: {result:?}");
            }
        }
    }
    #[test]
    fn matching_caps_are_not_completed_gates() {
        assert!(measurement("success").matches());
        assert!(!measurement("state limit").matches());
        let mut complete = measurement("success");
        complete.faithful_states_equal = false;
        assert!(!complete.matches());
    }

    #[test]
    fn violation_requires_equal_answers_and_valid_successors_and_traces() {
        let mut violation = measurement("Invariant(Safe)");
        violation.faithful_states_equal = false;
        violation.parallel_states_equal = false;
        assert!(violation.matches());
        violation.safety_answers_equal = false;
        assert!(!violation.matches());
        violation.safety_answers_equal = true;
        violation.traces_valid = false;
        assert!(!violation.matches());
        violation.traces_valid = true;
        violation.successor_sets_equal = false;
        assert!(!violation.matches());
    }
    #[test]
    fn forged_depth_cannot_validate_a_counterexample() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/Level.tla");
        let model = with_model(ModelRequest::new(path), |faithful| faithful.lower()).unwrap();
        let report = explore(&model, &Options::default()).unwrap();
        assert_eq!(report.completion, Completion::Complete);
        assert_eq!(
            report
                .nodes
                .iter()
                .map(|n| n.state.depth)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        let mut trace = report.trace_to(3);
        trace.states.last_mut().unwrap().depth = 1;
        let violation = tla_engine::Violation {
            kind: tla_engine::ViolationKind::Invariant("Inv".into()),
            trace,
        };
        assert!(!validate_trace(&model, &violation).unwrap());
    }
    #[test]
    fn standard_values_and_user_definitions_match_faithful() {
        for (name, states) in [
            ("AnyMembership", 2),
            ("AnyShadow", 2),
            ("ModelValues", 4),
            ("SubsetAssignment", 8),
            ("OperatorShadow", 2),
            ("UniversalAssignment", 2),
            ("RecursiveAssignment", 2),
            ("EnabledGuard", 2),
            ("ActionFunctions", 2),
            ("CaseActions", 3),
            ("EnabledInstances", 2),
            ("EnabledNested", 2),
            ("RecursiveProjection", 1),
            ("StateOperators", 2),
            ("Fingerprints", 1),
            ("PowerSetInclusion", 1),
            ("ConfiguredOperators", 3),
            ("RecursiveOperators", 2),
            ("UnexpandedSnapshot", 1),
            ("ChooseUnion", 1),
            ("AssertErrors", 2),
            ("AssumedRegisters", 3),
            ("GuardedAction", 1),
        ] {
            let request = Request {
                args: vec![name.into()],
                user_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures"),
                library_paths: vec![],
                community_modules: false,
                system_properties: vec![],
                engine: false,
            };
            let result = measure(request).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(result.matches(), "{name}: {result:?}");
            assert_eq!(result.states, states);
            if name == "UnexpandedSnapshot" {
                assert_eq!(result.successor_samples, 0);
            }
        }
    }
    #[test]
    fn action_functions_reject_invalid_initialization_and_domains() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
        for suffix in ["Invalid", "Domain"] {
            let mut request = ModelRequest::new(directory.join("ActionFunctions.tla"));
            request.config = Some(directory.join(format!("ActionFunctions{suffix}.cfg")));
            let (model, rejected) = with_model(request, |faithful| {
                let model = faithful.lower()?;
                let result = faithful
                    .initial_states()
                    .and_then(|states| faithful.successors(&states[0]).map(|_| ()));
                Ok((model, result.is_err()))
            })
            .unwrap();
            assert!(rejected, "faithful: {suffix}");
            assert!(
                model
                    .initial_states()
                    .and_then(|states| model.successors(&states[0]))
                    .is_err(),
                "native: {suffix}"
            );
        }
    }
}
