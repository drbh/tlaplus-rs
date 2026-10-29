use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tlaplus_bridge::RunOutcome;
use tlaplus_bridge::corpus::Corpus;
use tlaplus_bridge::model::ModelInfo;
use tlaplus_bridge::recording::{self as mp, Record};
use tlaplus_bridge::tool::{extract_messages, parse_stream};
use tlaplus_bridge::{
    Comparison, ErrorCategory, ModelRequest, ObservedRun, Outcome, PropertyKind, RunEnv, TraceEnd,
    Verdict, compare, from_outcome, observe_faithful, run_faithful, with_model,
};

const COUNTER: &str = include_str!("../../tests/fixtures/Counter.tla");

fn fixture(module: &str, spec: &str, cfg: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(format!("{module}.tla")), spec).unwrap();
    fs::write(dir.path().join(format!("{module}.cfg")), cfg).unwrap();
    dir
}

fn run(dir: &Path, module: &str, tool_mode: bool) -> ObservedRun {
    let mut args = [
        "-workers",
        "1",
        "-fp",
        "0",
        "-seed",
        "1",
        "-noGenerateSpecTE",
        "-metadir",
    ]
    .map(str::to_owned)
    .to_vec();
    args.push(dir.join("states").to_str().unwrap().to_string());
    if tool_mode {
        args.push("-tool".into());
    }
    args.push(module.into());
    observe_faithful(
        args,
        RunEnv {
            user_dir: dir.into(),
            max_heap_bytes: Some(64 << 20),
            max_direct_memory_bytes: Some(16 << 20),
            ..RunEnv::default()
        },
    )
    .unwrap()
}

fn info(dir: &Path, module: &str) -> ModelInfo {
    with_model(ModelRequest::new(dir.join(format!("{module}.tla"))), |m| {
        Ok(m.info().clone())
    })
    .unwrap()
}

#[test]
fn completed_run_establishes_configured_verdicts_but_early_stop_does_not() {
    let dir = fixture(
        "Counter",
        COUNTER,
        "INIT Init\nNEXT Next\nINVARIANT Safe\nPROPERTY Init",
    );
    let inventory = info(dir.path(), "Counter");
    let result = run(dir.path(), "Counter", true);
    let verdicts = result.verdicts_for(&inventory);
    assert_eq!(result.verdicts(), Some(verdicts.clone()));
    assert_eq!(verdicts.len(), 2);
    assert!(verdicts.iter().all(|v| v.verdict == Verdict::Satisfied));
    assert!(
        result.properties.is_empty(),
        "no negative observations on success"
    );
    assert!(result.counterexamples.is_empty());
    assert!(result.error.is_none());

    fs::write(
        dir.path().join("Counter.cfg"),
        "INIT Init\nNEXT Next\nINVARIANTS Broken Safe",
    )
    .unwrap();
    let inventory = info(dir.path(), "Counter");
    let result = run(dir.path(), "Counter", true);
    let verdicts = result.verdicts_for(&inventory);
    assert_eq!(result.verdicts(), Some(verdicts.clone()));
    assert_eq!(
        verdicts
            .iter()
            .find(|v| v.property.name == "Broken")
            .unwrap()
            .verdict,
        Verdict::Violated
    );
    assert_eq!(
        verdicts
            .iter()
            .find(|v| v.property.name == "Safe")
            .unwrap()
            .verdict,
        Verdict::NotDetermined
    );
    assert_eq!(result.properties.len(), 1);
    let trace = &result.counterexamples[0];
    assert_eq!(trace.properties[0].name, "Broken");
    assert_eq!(
        trace
            .states
            .iter()
            .map(|s| s.display.as_str())
            .collect::<Vec<_>>(),
        ["x = 0\n", "x = 1\n", "x = 2\n"]
    );
    assert_eq!(trace.end, TraceEnd::Finite);
    trace.check_structure().unwrap();
    let encoded = serde_json::to_string(&result).unwrap();
    assert_eq!(
        serde_json::from_str::<ObservedRun>(&encoded).unwrap(),
        result
    );
}

#[test]
fn extracts_initial_action_and_deadlock_counterexamples() {
    for (cfg, name, kind, length) in [
        (
            "INIT Init\nNEXT Next\nINVARIANT InitiallyBroken",
            Some("InitiallyBroken"),
            PropertyKind::Invariant,
            1,
        ),
        (
            "INIT Init\nNEXT Next\nPROPERTY InitiallyBroken",
            Some("InitiallyBroken"),
            PropertyKind::Initial,
            1,
        ),
        (
            "SPECIFICATION Spec\nPROPERTY NoChange",
            Some("NoChange"),
            PropertyKind::Action,
            2,
        ),
        ("INIT Init\nNEXT Stop", None, PropertyKind::Invariant, 4),
    ] {
        let dir = fixture("Counter", COUNTER, cfg);
        let result = run(dir.path(), "Counter", true);
        assert_eq!(result.counterexamples.len(), 1);
        let trace = &result.counterexamples[0];
        trace.check_structure().unwrap();
        assert_eq!(trace.states.len(), length);
        assert_eq!(trace.end, TraceEnd::Finite);
        if let Some(name) = name {
            assert_eq!(result.properties[0].property.name, name);
            assert_eq!(result.properties[0].property.kind, kind);
            assert_eq!(result.properties[0].verdict, Verdict::Violated);
            assert!(!result.deadlock_reported);
        } else {
            assert!(result.deadlock_reported);
            assert!(result.properties.is_empty());
            assert!(trace.properties.is_empty());
        }
    }
}

#[test]
fn extracts_stuttering_and_back_edge_lassos_in_both_recorder_layouts() {
    let dir = fixture(
        "Counter",
        COUNTER,
        "SPECIFICATION Spec\nPROPERTY EventuallyAlwaysZero",
    );
    let result = run(dir.path(), "Counter", true);
    assert_eq!(result.properties[0].property.kind, PropertyKind::Temporal);
    assert_eq!(result.properties[0].property.name, "EventuallyAlwaysZero");
    let trace = &result.counterexamples[0];
    trace.check_structure().unwrap();
    assert!(matches!(trace.end, TraceEnd::Stuttering { .. }));

    let spec = "---- MODULE Cycle ----\nEXTENDS Naturals\nVARIABLE x\nInit == x = 0\nNext == x' = 1 - x\nSpec == Init /\\ [][Next]_x /\\ WF_x(Next)\nEventuallyTwo == <>(x = 2)\n====";
    for tool_mode in [true, false] {
        let dir = fixture("Cycle", spec, "SPECIFICATION Spec\nPROPERTY EventuallyTwo");
        let result = run(dir.path(), "Cycle", tool_mode);
        assert_eq!(result.properties[0].property.name, "EventuallyTwo");
        let trace = &result.counterexamples[0];
        trace.check_structure().unwrap();
        assert!(matches!(trace.end, TraceEnd::BackTo { .. }), "{trace:?}");
        assert!(trace.states.iter().any(|s| s.display == "x = 0\n"));
        assert!(trace.states.iter().any(|s| s.display == "x = 1\n"));
    }
}

#[test]
fn distinguishes_evaluation_failure_from_a_false_property() {
    let spec = COUNTER.replace("Broken == x < 2", "Broken == 1 \\div (1 - x) = 1");
    let dir = fixture("Counter", &spec, "INIT Init\nNEXT Next\nINVARIANT Broken");
    let result = run(dir.path(), "Counter", true);
    assert_eq!(result.properties[0].property.name, "Broken");
    assert_eq!(result.properties[0].verdict, Verdict::EvaluationError);
    assert_eq!(
        result.error.as_ref().unwrap().category,
        ErrorCategory::Evaluation
    );

    let spec = COUNTER.replace("Broken == x < 2", "Broken == 1 \\div x = 0");
    fs::write(dir.path().join("Counter.tla"), spec).unwrap();
    let inventory = info(dir.path(), "Counter");
    let result = run(dir.path(), "Counter", true);
    assert!(result.properties.is_empty(), "{result:?}");
    assert_eq!(
        result.error.as_ref().unwrap().category,
        ErrorCategory::Evaluation
    );
    assert_eq!(
        result.verdicts_for(&inventory)[0].verdict,
        Verdict::NotDetermined
    );

    fs::write(
        dir.path().join("Counter.cfg"),
        "INIT Init\nNEXT Next\nINVARIANT Missing",
    )
    .unwrap();
    assert_eq!(
        run(dir.path(), "Counter", true).error.unwrap().category,
        ErrorCategory::Configuration
    );
    fs::write(
        dir.path().join("Counter.tla"),
        "---- MODULE Counter ----\nVARIABLE\n====",
    )
    .unwrap();
    assert_eq!(
        run(dir.path(), "Counter", true).error.unwrap().category,
        ErrorCategory::ParseOrSemantic
    );
}

fn record(code: i32, params: &[&str]) -> Record {
    Record {
        code,
        params: params.iter().map(|s| s.to_string()).collect(),
    }
}

fn outcome(records: Vec<Record>) -> RunOutcome {
    RunOutcome {
        exit_status: 12,
        error_code: mp::TLC_INVARIANT_VIOLATED_BEHAVIOR,
        records,
        unsupported: Vec::new(),
    }
}

#[test]
fn preserves_separate_traces_and_rejects_damaged_record_shapes() {
    let result = from_outcome(&outcome(vec![
        record(mp::TLC_INVARIANT_VIOLATED_BEHAVIOR, &["A"]),
        record(mp::TLC_BEHAVIOR_UP_TO_THIS_POINT, &[]),
        record(mp::TLC_STATE_PRINT2, &["x = 0\n", "1"]),
        record(mp::TLC_STATE_PRINT2, &["x = 1\n", "2"]),
        record(mp::TLC_INVARIANT_VIOLATED_BEHAVIOR, &["B"]),
        record(mp::TLC_BEHAVIOR_UP_TO_THIS_POINT, &[]),
        record(mp::TLC_STATE_PRINT2, &["x = 0\n", "1"]),
    ]))
    .unwrap();
    assert_eq!(result.counterexamples.len(), 2);
    assert_eq!(result.counterexamples[0].properties[0].name, "A");
    assert_eq!(result.counterexamples[1].properties[0].name, "B");
    for trace in &result.counterexamples {
        trace.check_structure().unwrap();
    }
    for bad in [
        record(mp::TLC_INVARIANT_VIOLATED_BEHAVIOR, &[]),
        record(mp::TLC_STATE_PRINT2, &["x = 0", "bad"]),
    ] {
        assert!(from_outcome(&outcome(vec![bad])).is_err());
    }
    let mut trace = result.counterexamples[0].clone();
    trace.end = TraceEnd::BackTo { ordinal: 3 };
    assert!(trace.check_structure().is_err());
    trace.end = TraceEnd::Stuttering { ordinal: 2 };
    assert!(trace.check_structure().is_err());
    trace.end = TraceEnd::Finite;
    trace.states[1].ordinal = 8;
    assert!(trace.check_structure().is_err());
}

#[test]
fn extracts_named_observations_and_traces_from_all_junit_records() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tlaplus-rs-explore/tests/tlc-oracle/manifests/java_tests.json");
    let data: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let mut methods = 0;
    let mut traces = 0;
    for case in data["cases"].as_array().unwrap() {
        for method in case["methods"].as_array().into_iter().flatten() {
            let records = method["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| Record {
                    code: r["code"].as_i64().unwrap() as i32,
                    params: r["params"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p.as_str().unwrap_or("null").to_string())
                        .collect(),
                })
                .collect();
            // The manifest records exit status, not the authoritative run error.
            // This test checks extraction only, not inferred error classification.
            let outcome = RunOutcome {
                exit_status: method["actual_exit_status"].as_i64().unwrap_or(-1) as i32,
                error_code: 0,
                records,
                unsupported: Vec::new(),
            };
            let observed = from_outcome(&outcome)
                .unwrap_or_else(|e| panic!("{}#{}: {e}", case["class"], method["method"]));
            methods += 1;
            traces += observed.counterexamples.len();
        }
    }
    assert_eq!(methods, 607);
    assert!(traces > 100);
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tlaplus-rs-explore")
}

fn check(module: &str, spec: &str, cfg: &str, outcome: Outcome, exit: i32) -> tlaplus_bridge::Answers {
    let scratch = tempfile::tempdir().unwrap();
    fs::write(scratch.path().join(format!("{module}.tla")), spec).unwrap();
    fs::write(scratch.path().join(format!("{module}.cfg")), cfg).unwrap();
    let output = Arc::new(Mutex::new(String::new()));
    let captured = output.clone();
    let env = RunEnv {
        user_dir: scratch.path().into(),
        printer: Some(Arc::new(move |text| {
            let mut out = captured.lock().unwrap();
            out.push_str(text);
            out.push('\n');
        })),
        max_heap_bytes: Some(64 << 20),
        max_direct_memory_bytes: Some(16 << 20),
        ..RunEnv::default()
    };
    let args = vec![
        "-tool".into(),
        "-workers".into(),
        "1".into(),
        "-fp".into(),
        "0".into(),
        "-seed".into(),
        "1".into(),
        "-metadir".into(),
        scratch.path().join("states").to_str().unwrap().into(),
        "-config".into(),
        module.into(),
        module.into(),
    ];
    let answer = run_faithful(args, env).unwrap();
    assert_eq!(answer.outcome, outcome, "{}", output.lock().unwrap());
    let messages = parse_stream(&output.lock().unwrap()).unwrap();
    assert_eq!(extract_messages(&messages, Some(exit)).unwrap(), answer);
    answer
}

#[test]
fn faithful_records_and_real_tool_stream_agree_on_small_models() {
    let spec = include_str!("../../tests/fixtures/Counter.tla");
    for (cfg, outcome, exit) in [
        ("INIT Init\nNEXT Next\nINVARIANT Safe", Outcome::Success, 0),
        (
            "INIT Init\nNEXT Next\nINVARIANT Broken",
            Outcome::InvariantViolated,
            12,
        ),
        (
            "INIT Init\nNEXT Next\nINVARIANT InitiallyBroken",
            Outcome::InvariantViolated,
            12,
        ),
        ("INIT Init\nNEXT Stop", Outcome::Deadlock, 11),
        (
            "SPECIFICATION Spec\nPROPERTY NoChange",
            Outcome::ActionPropertyViolated,
            13,
        ),
        (
            "SPECIFICATION Spec\nPROPERTY EventuallyAlwaysZero",
            Outcome::TemporalPropertyViolated,
            13,
        ),
        (
            "INIT Init\nNEXT Next\nINVARIANT Missing",
            Outcome::Unknown,
            151,
        ),
    ] {
        let answer = check("Counter", spec, cfg, outcome, exit);
        if outcome == Outcome::Success {
            assert_eq!(
                (answer.initial_distinct, answer.distinct, answer.depth),
                (Some(1), Some(4), Some(4))
            );
        }
    }
    check(
        "Assume",
        "---- MODULE Assume ----\nASSUME FALSE\n====",
        "",
        Outcome::AssumptionViolated,
        10,
    );
    check(
        "Bad",
        "---- MODULE Bad ----\nVARIABLE\n====",
        "",
        Outcome::Rejected,
        150,
    );
}

#[test]
fn faithful_hourclock_matches_recorded_java_without_external_checkouts() {
    let answer = check(
        "HourClock",
        include_str!("../../tests/fixtures/HourClock.tla"),
        include_str!("../../tests/fixtures/HourClock.cfg"),
        Outcome::Success,
        0,
    );
    let java = Corpus::read(&root().join("tests/tlc-oracle/results/examples/specifications__SpecifyingSystems__HourClock__HourClock.cfg.json.gz")).unwrap();
    let expected = &java.answers["specifications/SpecifyingSystems/HourClock/HourClock.cfg"];
    assert_eq!(compare(expected, &answer), Comparison::Match);
    assert_eq!(
        (answer.initial_distinct, answer.distinct, answer.depth),
        (Some(12), Some(12), Some(1))
    );
}

#[test]
fn captures_inventory_without_repeating_assumptions() {
    let spec = COUNTER.replace(
        "EXTENDS Naturals",
        "EXTENDS Naturals, TLC\nASSUME PrintT(\"inventory-loaded\")",
    );
    let dir = fixture("Counter", &spec, "INIT Init\nNEXT Next\nINVARIANT Safe");
    let captured = Arc::new(Mutex::new(String::new()));
    let output = captured.clone();
    let result = observe_faithful(
        vec![
            "-noGenerateSpecTE".into(),
            "-metadir".into(),
            dir.path().join("states").to_string_lossy().into(),
            "Counter".into(),
        ],
        RunEnv {
            user_dir: dir.path().into(),
            printer: Some(Arc::new(move |text| output.lock().unwrap().push_str(text))),
            max_heap_bytes: Some(64 << 20),
            max_direct_memory_bytes: Some(16 << 20),
            ..RunEnv::default()
        },
    )
    .unwrap();
    let properties = result.verdicts().unwrap();
    assert_eq!(properties.len(), 1);
    assert_eq!(properties[0].property.name, "Safe");
    assert_eq!(properties[0].verdict, Verdict::Satisfied);
    assert_eq!(
        captured.lock().unwrap().matches("inventory-loaded").count(),
        1
    );
}
