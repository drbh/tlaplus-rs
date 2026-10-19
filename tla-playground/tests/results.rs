//! Structured playground results preserve verification semantics and trace origins.
use serde_json::{Value, json};
use std::process::Command;

fn run(language: &str, source: &str, config: &str, limit: usize) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let request = dir.path().join("request.json");
    let response = dir.path().join("response.json");
    std::fs::write(
        &request,
        json!({"language":language,"source":source,"config":config,"maxStates":limit}).to_string(),
    )
    .unwrap();
    assert!(
        Command::new(env!("CARGO_BIN_EXE_tla-playground"))
            .arg(request)
            .arg(&response)
            .status()
            .unwrap()
            .success()
    );
    serde_json::from_slice(&std::fs::read(response).unwrap()).unwrap()
}

#[test]
fn both_languages_produce_the_jug_counterexample() {
    for (lang, source) in [
        ("tsl", include_str!("../../examples/16-puzzles/DieHard.tsl")),
        ("tla", include_str!("../../examples/16-puzzles/DieHard.tla")),
    ] {
        let result = run(lang, source, "", 500);
        assert_eq!(result["status"], "violated", "{result}");
        assert_eq!(result["trace"].as_array().unwrap().len(), 7);
        assert_eq!(result["graph"]["complete"], false);
        assert_eq!(result["trace"][6]["values"][0]["value"]["kind"], "function");
        if lang == "tsl" {
            assert!(result["trace"][1]["action"]["line"].as_u64().is_some());
            assert!(
                result["generated"]["source"]
                    .as_str()
                    .unwrap()
                    .contains("MODULE DieHard")
            );
        }
    }
}

#[test]
fn graph_is_not_just_a_discovery_tree_and_limits_are_not_passes() {
    let source = include_str!("../../examples/01-counter/Counter.tsl");
    let result = run("tsl", source, "", 500);
    assert_eq!(result["status"], "passed", "{result}");
    assert_eq!(result["graph"]["nodes"].as_array().unwrap().len(), 4);
    assert_eq!(result["graph"]["edges"].as_array().unwrap().len(), 4);
    assert!(
        result["graph"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["to"] == 0)
    );
    assert_eq!(result["graph"]["complete"], true);
    let limited = run("tsl", source, "", 2);
    assert_eq!(limited["status"], "incomplete", "{limited}");
    assert_eq!(limited["graph"]["complete"], false);
}

#[test]
fn liveness_cycles_diagnostics_and_explicit_configuration_survive() {
    let result = run(
        "tsl",
        include_str!("../../examples/17-modern-dsl/UnfairRetry.tsl"),
        "",
        500,
    );
    assert_eq!(result["status"], "violated", "{result}");
    assert!(result["loopBack"]["index"].as_u64().is_some());
    assert_eq!(result["graph"]["complete"], true);
    let error = run(
        "tsl",
        "spec Broken { state x: Int = true; action stay {} }",
        "",
        500,
    );
    assert_eq!(error["status"], "error");
    assert!(error["location"]["line"].as_u64().is_some());
    let source =
        "---- MODULE Test ----\nEXTENDS Integers\nVARIABLE x\nInit == x = 0\nNext == x' = x\n====";
    assert_eq!(
        run("tla", source, "INIT Init\nNEXT Next", 500)["status"],
        "passed"
    );
}
