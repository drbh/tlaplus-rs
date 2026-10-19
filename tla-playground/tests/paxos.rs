//! Bounded Paxos safety, non-vacuity, and the intentionally broken inheritance rule.
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    process::Command,
};
const GOOD: &str = include_str!("../../examples/20-paxos/Paxos.tsl");
const BROKEN: &str = include_str!("../../examples/20-paxos/BrokenPaxos.tsl");
fn run(source: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.json");
    let output = dir.path().join("output.json");
    std::fs::write(
        &input,
        json!({"language":"tsl","source":source,"config":"","maxStates":2000}).to_string(),
    )
    .unwrap();
    assert!(
        Command::new(env!("CARGO_BIN_EXE_tla-playground"))
            .arg(input)
            .arg(&output)
            .status()
            .unwrap()
            .success()
    );
    serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap()
}
fn variable<'a>(state: &'a Value, name: &str) -> &'a Value {
    &state["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap()["value"]
}
fn field<'a>(record: &'a Value, name: &str) -> &'a Value {
    &record["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pair| pair[0]["data"] == name)
        .unwrap()[1]["data"]
}
fn chosen(state: &Value) -> BTreeSet<String> {
    let mut voters: BTreeMap<(i64, String), BTreeSet<String>> = BTreeMap::new();
    for vote in variable(state, "votes")["data"].as_array().unwrap() {
        let key = (
            field(vote, "ballot").as_i64().unwrap(),
            field(vote, "value").as_str().unwrap().to_owned(),
        );
        voters
            .entry(key)
            .or_default()
            .insert(field(vote, "acceptor").as_str().unwrap().to_owned());
    }
    voters
        .into_iter()
        .filter(|(_, acceptors)| acceptors.len() >= 2)
        .map(|((_, value), _)| value)
        .collect()
}

#[test]
fn correct_paxos_exhausts_and_broken_paxos_chooses_two_values() {
    let good = run(GOOD);
    assert_eq!(good["status"], "passed", "{good}");
    assert_eq!(good["graph"]["complete"], true);
    assert_eq!(good["statistics"]["states"], 1696);
    let mut witnessed = BTreeSet::new();
    let mut inherited = false;
    for node in good["graph"]["nodes"].as_array().unwrap() {
        let choices = chosen(node);
        assert!(choices.len() <= 1);
        witnessed.extend(choices);
        let proposals = &variable(node, "proposals")["data"];
        inherited |=
            proposals[0][1]["data"] == "Value::Red" && proposals[1][1]["data"] == "Value::Red";
    }
    assert_eq!(
        witnessed,
        BTreeSet::from(["Value::Red".into(), "Value::Blue".into()])
    );
    assert!(
        inherited,
        "the higher ballot must sometimes inherit the lower ballot's value"
    );
    let broken = run(BROKEN);
    assert_eq!(broken["status"], "violated", "{broken}");
    assert_eq!(broken["message"], "Invariant Agreement is violated.");
    let trace = broken["trace"].as_array().unwrap();
    assert_eq!(trace.len(), 11);
    assert!(trace[..10].iter().all(|state| chosen(state).len() <= 1));
    assert_eq!(chosen(&trace[10]).len(), 2);
    assert!(
        variable(&trace[10], "promises")["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| field(p, "ballot") == 2
                && field(p, "acceptedBallot") == 1
                && field(p, "value") == "Value::Red")
    );
}

#[test]
fn broken_example_changes_only_the_documented_rule() {
    let expected=GOOD.replace("// Bounded single-decree Paxos: three acceptors, two competing ballots.","// Deliberately broken Paxos: a proposer ignores previously accepted values.")
        .replace("spec Paxos {","spec BrokenPaxos {")
        .replace("        if highest(replies).acceptedBallot == 0 { preferred(ballot) }\n        else { highest(replies).value }","        // BUG: a higher ballot must inherit the highest prior accepted value.\n        preferred(ballot)");
    assert_eq!(BROKEN, expected);
}

#[test]
#[ignore = "requires JAVA and TLA2TOOLS_JAR"]
fn both_paxos_exports_agree_with_tlc() {
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let jar = std::env::var("TLA2TOOLS_JAR").expect("absolute path to tla2tools.jar");
    for (source, code, message) in [
        (GOOD, 0, "1696 distinct states found"),
        (BROKEN, 12, "Invariant Agreement is violated"),
    ] {
        let artifact = tla_dsl::compile(source).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(format!("{}.tla", artifact.module)),
            artifact.tla,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(format!("{}.cfg", artifact.module)),
            artifact.config,
        )
        .unwrap();
        let output = Command::new(&java)
            .args([
                "-XX:+UseParallelGC",
                "-cp",
                &jar,
                "tlc2.TLC",
                "-workers",
                "1",
                &artifact.module,
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(code), "{text}");
        assert!(text.contains(message), "{text}");
    }
}
