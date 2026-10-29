use std::fs;
use std::path::Path;
use std::time::Duration;

use tlaplus_bridge::{
    Answers, Comparison, Mode, ModelRequest, ObservedRun, Outcome, RunEnv, compare,
    observe_faithful, with_model,
};
use tlaplus_conformance::{Case, Execution, Request, examples, execute, execute_suite};
use tlaplus_engine::{Completion, Options, ViolationKind, explore, validate_trace};

#[derive(serde::Deserialize, serde::Serialize)]
struct EngineOutput {
    summary: Answers,
    trace: Option<serde_json::Value>,
    error: Option<String>,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [command, models, recordings, output] = args.as_slice()
        && command == "plan-examples"
    {
        let cases = examples(Path::new(models), Path::new(recordings))?;
        serde_json::to_writer_pretty(fs::File::create(output)?, &cases)?;
        return Ok(());
    }
    if let [command, models, recordings, baseline, output, seconds] = args.as_slice()
        && command == "fast-safety"
    {
        let baseline: std::collections::BTreeMap<String, Execution<ObservedRun>> =
            serde_json::from_reader(fs::File::open(baseline)?)?;
        let mut cases = examples(Path::new(models), Path::new(recordings))?;
        cases.retain(|case| match baseline.get(&case.id) {
            Some(Execution::Completed(run)) => {
                matches!(
                    run.summary.outcome,
                    Outcome::Success | Outcome::InvariantViolated | Outcome::Rejected
                ) && !run
                    .configured_properties
                    .iter()
                    .flatten()
                    .any(|p| p.kind == tlaplus_bridge::PropertyKind::Temporal)
            }
            _ => false,
        });
        return suite(cases, output, seconds, true);
    }
    if let [command, models, recordings, output, seconds] = args.as_slice()
        && (command == "examples" || command == "compare-examples")
    {
        let cases = examples(Path::new(models), Path::new(recordings))?;
        return suite(cases, output, seconds, command == "compare-examples");
    }
    if let [command, plan, output, seconds] = args.as_slice()
        && command == "suite"
    {
        let cases: Vec<Case> = serde_json::from_reader(fs::File::open(plan)?)?;
        return suite(cases, output, seconds, true);
    }
    let [input, output] = args.as_slice() else {
        return Err("usage: faithful REQUEST.json RESULT.json".into());
    };
    let request: Request = serde_json::from_reader(fs::File::open(input)?)?;
    if request.engine {
        let result = engine(&request);
        serde_json::to_writer(fs::File::create(output)?, &result)?;
        return Ok(());
    }
    let mut args = vec![
        "-noGenerateSpecTE".into(),
        "-metadir".into(),
        request
            .user_dir
            .join("oracle-states")
            .to_string_lossy()
            .into_owned(),
    ];
    args.extend(request.args);
    let observed = observe_faithful(
        args,
        RunEnv {
            user_dir: request.user_dir,
            community_modules: request.community_modules,
            system_properties: request.system_properties,
            max_heap_bytes: Some(256 << 20),
            max_direct_memory_bytes: Some(64 << 20),
            ..RunEnv::default()
        },
    )?;
    serde_json::to_writer(fs::File::create(Path::new(output))?, &observed)?;
    Ok(())
}

fn suite(
    mut cases: Vec<Case>,
    output: &str,
    seconds: &str,
    comparison: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(filter) = std::env::var("REWRITE_FILTER") {
        cases.retain(|case| case.id.contains(&filter));
    }
    let timeout = Duration::from_secs(seconds.parse()?);
    let results =
        execute_suite::<ObservedRun>(&std::env::current_exe()?, &cases, timeout, |id, result| {
            let status = match result {
                Execution::Completed(_) => "completed",
                Execution::TimedOut => "timed out",
                Execution::Failed { .. } => "failed",
            };
            eprintln!("{status}: {id}");
        })?;
    if comparison {
        let mut reports = std::collections::BTreeMap::new();
        let mut matched = 0;
        for mut case in cases {
            case.request.engine = true;
            let engine =
                execute::<EngineOutput>(&std::env::current_exe()?, &case.request, timeout)?;
            let faithful = &results[&case.id];
            let equal = match (faithful, &engine) {
                (Execution::Completed(left), Execution::Completed(right)) => {
                    compare(&left.summary, &right.summary) == Comparison::Match
                }
                _ => false,
            };
            matched += usize::from(equal);
            eprintln!(
                "{}: {}",
                if equal { "match" } else { "unresolved" },
                case.id
            );
            reports.insert(case.id, serde_json::json!({"faithful": faithful, "engine": engine, "summary_matches": equal}));
        }
        serde_json::to_writer_pretty(fs::File::create(output)?, &reports)?;
        if matched != reports.len() {
            return Err(format!(
                "{matched}/{} summaries match; see the report",
                reports.len()
            )
            .into());
        }
        return Ok(());
    }
    serde_json::to_writer_pretty(fs::File::create(output)?, &results)?;
    if results
        .values()
        .any(|result| !matches!(result, Execution::Completed(_)))
    {
        return Err("suite contains incomplete runs; see the report".into());
    }
    Ok(())
}

fn engine(request: &Request) -> EngineOutput {
    let unknown = |error: String| EngineOutput {
        summary: Answers::default(),
        trace: None,
        error: Some(error),
    };
    let parsed = match tlaplus_bridge::FaithfulArgs::parse(&request.args) {
        Ok(parsed) => parsed,
        Err(error) => return unknown(error.0),
    };
    if parsed.dfid.is_some()
        || !matches!(parsed.run_mode, tlaplus_bridge::FaithfulRunMode::ModelCheck)
        || parsed.continuation
        || !request.system_properties.is_empty()
    {
        return unknown("the safety adapter does not implement these run options".into());
    }
    let Some(main) = parsed.main_file else {
        return unknown("missing module".into());
    };
    let mut model = ModelRequest::new(request.user_dir.join(&main).with_extension("tla"));
    let config = request
        .user_dir
        .join(parsed.config_file.as_ref().unwrap_or(&main));
    model.config = Some(if config.extension().is_some_and(|ext| ext == "tla") {
        config
    } else {
        config.with_extension("cfg")
    });
    model.seed = parsed.seed.unwrap_or(1);
    model.community_modules = request.community_modules;
    let result = with_model(model, move |model| {
        let system = model.system()?;
        let report = explore(
            &system,
            &Options {
                check_deadlock: parsed.deadlock && model.info().check_deadlock,
                max_states: std::num::NonZeroUsize::new(100_000),
                ..Options::default()
            },
        )?;
        let (outcome, trace) = match &report.completion {
            Completion::Complete => (Outcome::Success, None),
            Completion::StateLimit => return Ok(unknown("state limit reached".into())),
            Completion::Violated(violation) => {
                if !validate_trace(&system, violation)? {
                    return Ok(unknown("counterexample validation failed".into()));
                }
                let states = violation
                    .trace
                    .states
                    .iter()
                    .map(|s| model.state_values(s))
                    .collect::<Result<Vec<_>, _>>()?;
                let outcome = match &violation.kind {
                    ViolationKind::InitialProperty(_) => Outcome::InitialPropertyViolated,
                    ViolationKind::Invariant(_) => Outcome::InvariantViolated,
                    ViolationKind::ActionProperty(_) => Outcome::ActionPropertyViolated,
                    ViolationKind::Deadlock => Outcome::Deadlock,
                };
                (
                    outcome,
                    Some(
                        serde_json::json!({"kind": format!("{:?}", violation.kind), "states": states, "actions": violation.trace.actions}),
                    ),
                )
            }
        };
        Ok(EngineOutput {
            summary: Answers {
                mode: Mode::Bfs,
                outcome,
                initial_distinct: Some(report.statistics.initial_distinct as u64),
                distinct: Some(report.nodes.len() as u64),
                depth: Some(report.statistics.depth as u64),
            },
            trace,
            error: None,
        })
    });
    result.unwrap_or_else(|error| {
        let outcome = match error.code {
            Some(
                tlaplus_bridge::recording::TLC_PARSING_FAILED
                | tlaplus_bridge::recording::TLC_PARSING_FAILED2,
            ) => Outcome::Rejected,
            Some(tlaplus_bridge::recording::TLC_ASSUMPTION_FALSE) => Outcome::AssumptionViolated,
            _ => Outcome::Unknown,
        };
        EngineOutput {
            summary: Answers {
                mode: Mode::Bfs,
                outcome,
                ..Answers::default()
            },
            trace: None,
            error: Some(error.to_string()),
        }
    })
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
