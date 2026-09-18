use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tla_oracle::corpus::{Corpus, compare_corpora};
use tla_oracle::tool::{Message, extract_messages, parse_stream};
use tla_oracle::{Answers, Comparison, Mode, Outcome, compare};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tlaplus-rs-explore")
}

fn read_gzip(path: &Path) -> Value {
    let mut text = String::new();
    flate2::read::GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn extracts_every_examples_recording_and_checks_recorded_counts() {
    for directory in ["examples", "examples-workers4"] {
        let dir = root().join("tests/tlc-oracle/results").join(directory);
        let corpus = Corpus::read(&dir).unwrap();
        assert_eq!(
            corpus.answers.len(),
            233,
            "fixture inventory changed; review coverage"
        );
        let mut successes = 0;
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "gz") {
                continue;
            }
            let data = read_gzip(&path);
            let answer = &corpus.answers[data["cfg"].as_str().unwrap()];
            // These independent counts were extracted when recording Java.
            for (field, observed) in [
                ("init_states", answer.initial_distinct),
                ("distinct", answer.distinct),
                ("depth", answer.depth),
            ] {
                if let Some(recorded) = data["counts"][field].as_u64() {
                    assert_eq!(observed, Some(recorded), "{}: {field}", path.display());
                }
            }
            if data["run"]["run_status"] != "completed" {
                assert_eq!(answer.outcome, Outcome::Unknown);
            }
            if let Some(messages) = data.get("messages") {
                let messages: Vec<Message> = serde_json::from_value(messages.clone()).unwrap();
                // Exercise actual Java bodies through the raw stream parser too.
                let stream: String = messages
                    .iter()
                    .map(|m| {
                        format!(
                            "@!@!@STARTMSG {}:{} @!@!@\n{}\n@!@!@ENDMSG {} @!@!@\n",
                            m.code, m.severity, m.text, m.code
                        )
                    })
                    .collect();
                assert_eq!(parse_stream(&stream).unwrap(), messages);
                let exit = if data["run"]["run_status"] == "completed" {
                    data["run"]["returncode"].as_i64().map(|s| s as i32)
                } else {
                    None
                };
                assert_eq!(
                    &extract_messages(&parse_stream(&stream).unwrap(), exit).unwrap(),
                    answer
                );
            }
            if answer.outcome == Outcome::Success {
                successes += 1;
                assert_eq!(compare(answer, answer), Comparison::Match);
            }
        }
        assert!(
            successes > 100,
            "expected substantial successful BFS coverage"
        );
        eprintln!(
            "{directory}: {} recordings, {successes} successful BFS summaries",
            corpus.answers.len()
        );
    }
}

#[test]
fn extracts_every_junit_method_and_checks_parameter_counts() {
    let path = root().join("tests/tlc-oracle/manifests/java_tests.json");
    let corpus = Corpus::read(&path).unwrap();
    assert_eq!(
        corpus.answers.len(),
        607,
        "fixture inventory changed; review coverage"
    );
    let data: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    for case in data["cases"].as_array().unwrap() {
        for method in case["methods"].as_array().into_iter().flatten() {
            let id = format!(
                "{}#{}",
                case["class"].as_str().unwrap(),
                method["method"].as_str().unwrap()
            );
            let answer = &corpus.answers[&id];
            let records = method["records"].as_array().unwrap();
            for (code, index, observed) in [(2199, 1, answer.distinct), (2194, 0, answer.depth)] {
                if let Some(record) = records.iter().rev().find(|r| r["code"] == code) {
                    let expected: u64 = record["params"][index]
                        .as_str()
                        .unwrap()
                        .replace(',', "")
                        .parse()
                        .unwrap();
                    assert_eq!(observed, Some(expected), "{id}, code {code}");
                }
            }
        }
    }
    assert!(
        corpus
            .answers
            .values()
            .any(|a| a.outcome == Outcome::Deadlock)
    );
    assert!(
        corpus
            .answers
            .values()
            .any(|a| a.outcome == Outcome::TemporalPropertyViolated)
    );
    assert!(
        corpus
            .answers
            .values()
            .any(|a| a.outcome == Outcome::AssumptionViolated)
    );
    eprintln!("JUnit: {} method recordings", corpus.answers.len());
}

#[test]
fn report_and_cli_fail_on_missing_different_or_inconclusive_cases() {
    let success = Answers {
        mode: Mode::Bfs,
        outcome: Outcome::Success,
        initial_distinct: Some(1),
        distinct: Some(4),
        depth: Some(4),
    };
    let expected = Corpus {
        answers: [
            ("same".into(), success.clone()),
            ("missing".into(), success.clone()),
        ]
        .into(),
    };
    let actual = Corpus {
        answers: [
            (
                "same".into(),
                Answers {
                    depth: Some(3),
                    ..success.clone()
                },
            ),
            ("extra".into(), success.clone()),
        ]
        .into(),
    };
    let report = compare_corpora(&expected, &actual);
    assert!(!report.is_match());
    assert_eq!(report.missing, ["missing"]);
    assert_eq!(report.unexpected, ["extra"]);
    assert_eq!(report.differences.len(), 1);
    assert!(!compare_corpora(&Corpus::default(), &Corpus::default()).is_match());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("answers.json");
    fs::write(&path, serde_json::to_string(&expected).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oracle"))
        .args(["compare", path.to_str().unwrap(), path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let changed = dir.path().join("changed.json");
    fs::write(&changed, serde_json::to_string(&actual).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oracle"))
        .args(["compare", path.to_str().unwrap(), changed.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let unknown = Corpus {
        answers: [("unknown".into(), Answers::default())].into(),
    };
    fs::write(&changed, serde_json::to_string(&unknown).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oracle"))
        .args([
            "compare",
            changed.to_str().unwrap(),
            changed.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["inconclusive"][0], "unknown");
    let output = Command::new(env!("CARGO_BIN_EXE_oracle"))
        .arg("extract")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Corpus>(&output.stdout)
            .unwrap()
            .answers,
        expected.answers
    );
}

#[test]
fn rejects_bad_artifacts_and_duplicate_case_ids() {
    for json in [
        "{}",
        "{\"cases\":[{\"class\":\"C\",\"methods\":[{\"method\":\"m\"},{\"method\":\"m\"}]}]}",
    ] {
        assert!(Corpus::from_json(json).is_err());
    }
    let empty = tempfile::tempdir().unwrap();
    assert!(Corpus::read(empty.path()).is_err());
}
