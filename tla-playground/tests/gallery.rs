//! Puzzle answers are checked independently of the models' deliberate failures.
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf, process::Command};

const PUZZLES: &[(&str, usize)] = &[
    ("KnightsAndKnaves", 0),
    ("FeatureSelection", 0),
    ("MiniEinstein", 0),
    ("Cryptarithm", 0),
    ("MiniSudoku", 7),
    ("BridgeAndTorch", 5),
    ("WarehouseRobot", 4),
    ("StaffRota", 6),
    ("MiniFactory", 6),
];
fn source(name: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(root.join(format!("../examples/19-puzzle-gallery/{name}.tsl"))).unwrap()
}
fn run(source: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let request = dir.path().join("request.json");
    let response = dir.path().join("response.json");
    std::fs::write(
        &request,
        json!({"language":"tsl","source":source,"config":"","maxStates":500}).to_string(),
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
fn variable<'a>(state: &'a Value, name: &str) -> &'a Value {
    &state["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap()["value"]
}
fn sequence(value: &Value) -> Vec<&Value> {
    value["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| &pair[1])
        .collect()
}
fn numbers(value: &Value) -> Vec<i64> {
    sequence(value)
        .iter()
        .map(|v| v["data"].as_i64().unwrap())
        .collect()
}
fn without_goal(source: &str) -> String {
    source
        .lines()
        .filter(|l| {
            !l.trim().starts_with("invariant NoSolution")
                && !l.trim().starts_with("invariant NotSolved")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn all_gallery_examples_solve_within_the_default_playground_budget() {
    for (name, moves) in PUZZLES {
        let result = run(&source(name));
        assert_eq!(result["status"], "violated", "{name}: {result}");
        let message = result["message"].as_str().unwrap();
        assert!(
            message.contains("NoSolution") || message.contains("NotSolved"),
            "{name}: {message}"
        );
        let trace = result["trace"].as_array().unwrap();
        assert_eq!(trace.len(), moves + 1, "{name}");
        check_answer(name, trace.last().unwrap());
    }
}

fn check_answer(name: &str, state: &Value) {
    match name {
        "KnightsAndKnaves" => assert_eq!(
            sequence(variable(state, "knight"))
                .iter()
                .map(|v| v["data"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [false, true, false]
        ),
        "Cryptarithm" => {
            for (letter, digit) in [("digitI", 1), ("digitB", 9), ("digitL", 0)] {
                assert_eq!(variable(state, letter)["data"], digit);
            }
        }
        "MiniEinstein" => {
            assert_eq!(
                sequence(variable(state, "job"))
                    .iter()
                    .map(|v| v["data"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["Job::Doctor", "Job::Baker", "Job::Teacher"]
            );
            assert_eq!(
                sequence(variable(state, "pet"))
                    .iter()
                    .map(|v| v["data"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["Pet::Dog", "Pet::Cat", "Pet::Fish"]
            );
        }
        "FeatureSelection" => {
            let enabled: BTreeSet<_> = variable(state, "enabled")["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["data"].as_str().unwrap())
                .collect();
            assert_eq!(enabled.len(), 3);
            let cost: i32 = enabled
                .iter()
                .map(|f| {
                    if matches!(*f, "Feature::Encryption" | "Feature::Backup") {
                        1
                    } else {
                        2
                    }
                })
                .sum();
            assert!(cost <= 5);
            assert!(
                !(enabled.contains("Feature::Offline") && enabled.contains("Feature::CloudSync"))
            );
            assert!(
                !enabled.contains("Feature::Analytics") || enabled.contains("Feature::CloudSync")
            );
            assert!(
                !(enabled.contains("Feature::CloudSync") || enabled.contains("Feature::Backup"))
                    || enabled.contains("Feature::Encryption")
            );
        }
        "MiniSudoku" => check_sudoku(&numbers(variable(state, "board"))),
        "BridgeAndTorch" => {
            assert_eq!(variable(state, "elapsed")["data"], 17);
            assert!(
                sequence(variable(state, "side"))
                    .iter()
                    .all(|v| v["data"] == "Bank::Far")
            );
        }
        "WarehouseRobot" => assert_eq!(numbers(variable(state, "crate")), [3, 3]),
        "StaffRota" => {
            let shifts = sequence(variable(state, "shifts"));
            let shifts: Vec<_> = shifts.iter().map(|v| v["data"].as_str().unwrap()).collect();
            assert_eq!(shifts.len(), 6);
            assert!(shifts.windows(2).all(|p| p[0] != p[1]));
            for person in ["Staff::Alice", "Staff::Bob", "Staff::Cara"] {
                assert_eq!(shifts.iter().filter(|p| **p == person).count(), 2);
            }
            assert_ne!(shifts[0], "Staff::Bob");
            assert_ne!(shifts[3], "Staff::Alice");
            assert_ne!(shifts[5], "Staff::Cara");
        }
        "MiniFactory" => check_factory(&numbers(variable(state, "starts"))),
        _ => panic!("unknown puzzle"),
    }
}

fn check_sudoku(board: &[i64]) {
    let digits = BTreeSet::from([1, 2, 3, 4]);
    for row in 0..4 {
        assert_eq!(
            board[row * 4..row * 4 + 4]
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            digits
        );
    }
    for col in 0..4 {
        assert_eq!(
            (0..4).map(|r| board[r * 4 + col]).collect::<BTreeSet<_>>(),
            digits
        );
    }
    for top in [0, 2] {
        for left in [0, 2] {
            let mut block = BTreeSet::new();
            for r in top..top + 2 {
                for c in left..left + 2 {
                    block.insert(board[r * 4 + c]);
                }
            }
            assert_eq!(block, digits);
        }
    }
}
fn check_factory(starts: &[i64]) {
    let duration = [2, 1, 1, 1, 2, 2];
    let machine = [0, 1, 0, 1, 0, 1];
    for i in 0..6 {
        assert!(starts[i] >= 0 && starts[i] + duration[i] <= 5);
        if i >= 3 {
            assert!(starts[i] >= starts[i - 3] + duration[i - 3]);
        }
        for j in 0..i {
            if machine[i] == machine[j] {
                assert!(
                    starts[i] >= starts[j] + duration[j] || starts[j] >= starts[i] + duration[i]
                );
            }
        }
    }
}

#[test]
fn uniqueness_multiple_answers_and_optimality_are_exhaustively_checked() {
    for (name, count) in [
        ("KnightsAndKnaves", 1),
        ("MiniEinstein", 1),
        ("Cryptarithm", 1),
        ("FeatureSelection", 3),
    ] {
        let result = run(&without_goal(&source(name)));
        assert_eq!(result["status"], "passed", "{name}: {result}");
        assert_eq!(result["statistics"]["initial"], count, "{name}");
    }
    let sudoku = run(&without_goal(&source("MiniSudoku")));
    assert_eq!(sudoku["status"], "passed");
    let completed = sudoku["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| numbers(variable(n, "board")).iter().all(|v| *v != 0))
        .count();
    assert_eq!(completed, 1);
    for (name, from, to) in [
        ("BridgeAndTorch", "Budget: Int = 17", "Budget: Int = 16"),
        ("MiniFactory", "Deadline: Int = 5", "Deadline: Int = 4"),
    ] {
        let result = run(&source(name).replace(from, to));
        assert_eq!(result["status"], "passed", "{name}: {result}");
        assert_eq!(result["graph"]["complete"], true);
    }
}

#[test]
#[ignore = "requires JAVA and TLA2TOOLS_JAR"]
fn gallery_exports_agree_with_tlc() {
    let jar = std::env::var("TLA2TOOLS_JAR").expect("absolute path to tla2tools.jar");
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let mut cases: Vec<_> = PUZZLES
        .iter()
        .map(|(name, _)| (name.to_string(), source(name), 12))
        .collect();
    cases.push((
        "BridgeAndTorch".into(),
        source("BridgeAndTorch").replace("Budget: Int = 17", "Budget: Int = 16"),
        0,
    ));
    cases.push((
        "MiniFactory".into(),
        source("MiniFactory").replace("Deadline: Int = 5", "Deadline: Int = 4"),
        0,
    ));
    for (name, source, code) in cases {
        let artifact = tla_dsl::compile(&source).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(format!("{name}.tla")), artifact.tla).unwrap();
        std::fs::write(dir.path().join(format!("{name}.cfg")), artifact.config).unwrap();
        let result = Command::new(&java)
            .args([
                "-XX:+UseParallelGC",
                "-cp",
                &jar,
                "tlc2.TLC",
                "-workers",
                "1",
                &name,
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.status.code(), Some(code), "{name}: {text}");
        if code == 12 {
            assert!(
                text.contains("Invariant NoSolution is violated")
                    || text.contains("Invariant NotSolved is violated"),
                "{name}: {text}"
            );
        }
    }
}
