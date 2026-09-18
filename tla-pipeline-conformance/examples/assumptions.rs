pub mod support;

use serde::{Deserialize, Serialize};
use std::{fs, time::Duration};
use tla_bridge::with_loaded_model;
use tla_conformance::{Case, Execution, Request, execute_suite};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Verdict {
    Satisfied,
    False {
        index: usize,
    },
    Error {
        index: usize,
        category: Option<String>,
        detail: String,
    },
}
impl Verdict {
    fn from_result(index: usize, result: Result<bool, String>, faithful: bool) -> Self {
        match result {
            Ok(true) => Self::Satisfied,
            Ok(false) => Self::False { index },
            Err(error) => {
                let (category, detail) = support::rejection(&error, faithful);
                Self::Error {
                    index,
                    category,
                    detail,
                }
            }
        }
    }
    fn matches(&self, other: &Self) -> bool {
        self == other && !matches!(self, Self::Error { category: None, .. })
    }
}
#[derive(Debug, Deserialize, Serialize)]
struct Comparison {
    expected: Verdict,
    actual: Verdict,
    answers_equal: bool,
}
#[derive(Debug, Deserialize, Serialize)]
struct Measurement {
    comparison: Option<Comparison>,
    failure: Option<String>,
}
impl Measurement {
    fn matches(&self) -> bool {
        self.failure.is_none() && self.comparison.as_ref().is_some_and(|c| c.answers_equal)
    }
}
fn measure(request: Request) -> Result<Comparison, String> {
    let (request, _) = support::configuration(&request)?;
    let (model, assumptions, expected) = with_loaded_model(request, |faithful| {
        let (model, assumptions) = faithful.lower_assumptions()?;
        let expected = faithful
            .first_failed_assumption()
            .map_or(Verdict::Satisfied, |(index, result)| {
                Verdict::from_result(index, result.map_err(|e| e.message), true)
            });
        Ok((model, assumptions, expected))
    })
    .map_err(|e| e.to_string())?;
    let mut actual = Verdict::Satisfied;
    for (index, id) in assumptions {
        let result = model
            .evaluate(id, &[], &[])
            .and_then(|value| value.boolean().map_err(tla_model::Error::from))
            .map_err(|e| e.to_string());
        actual = Verdict::from_result(index, result, false);
        if actual != Verdict::Satisfied {
            break;
        }
    }
    let answers_equal = expected.matches(&actual);
    Ok(Comparison {
        expected,
        actual,
        answers_equal,
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
        return Err("usage: assumptions PLAN.json REPORT.json SECONDS".into());
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
        "{matched}/{} cases match assumption verdicts, indices and error details",
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
    use tla_bridge::{ModelRequest, model::Stage, with_model};
    fn fixture(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Assumptions.tla"),
            format!("---- MODULE Assumptions ----\nEXTENDS Integers, TLC\n{body}\n====\n"),
        )
        .unwrap();
        fs::write(dir.path().join("Assumptions.cfg"), "").unwrap();
        dir
    }
    fn request(dir: &std::path::Path) -> Request {
        Request {
            args: vec!["Assumptions".into()],
            user_dir: dir.into(),
            library_paths: vec![],
            community_modules: false,
            system_properties: vec![],
            engine: false,
        }
    }
    #[test]
    fn assumptions_preserve_first_failure_and_skip_axioms() {
        for (body, expected) in [
            ("AXIOM FALSE\nASSUME TRUE", Verdict::Satisfied),
            (
                "ASSUME FALSE\nASSUME Assert(FALSE, \"late\")",
                Verdict::False { index: 0 },
            ),
            (
                "AXIOM FALSE\nASSUME TRUE\nASSUME FALSE",
                Verdict::False { index: 2 },
            ),
            (
                "ASSUME Assert(FALSE, \"message\")",
                Verdict::Error {
                    index: 0,
                    category: Some("assertion".into()),
                    detail: "\"message\"".into(),
                },
            ),
            (
                "ASSUME (1 \\div 0) = 0",
                Verdict::Error {
                    index: 0,
                    category: Some("division_by_zero".into()),
                    detail: "division by zero".into(),
                },
            ),
        ] {
            let dir = fixture(body);
            let result = measure(request(dir.path())).unwrap();
            assert!(result.answers_equal, "{body}: {result:?}");
            assert_eq!(result.actual, expected);
        }
    }
    #[test]
    fn random_assumptions_share_the_setup_boundary_and_draw_sequence() {
        let dir = fixture("ASSUME RandomElement(1..10) > 2\nASSUME RandomElement(1..10) > 2");
        for seed in 1..20 {
            let mut request = request(dir.path());
            request
                .args
                .splice(0..0, ["-seed".into(), seed.to_string()]);
            let result = measure(request).unwrap();
            assert!(result.answers_equal, "seed {seed}: {result:?}");
        }
    }
    #[test]
    fn checked_sessions_still_reject_and_inspection_runs_assumptions_once() {
        let dir = fixture("ASSUME Print(\"first\", FALSE)\nASSUME Print(\"late\", FALSE)");
        let error = with_model::<()>(
            ModelRequest::new(dir.path().join("Assumptions.tla")),
            |_| panic!("checked sessions must reject false assumptions"),
        )
        .unwrap_err();
        assert_eq!(error.stage, Stage::Assumptions);
        with_loaded_model(
            ModelRequest::new(dir.path().join("Assumptions.tla")),
            |model| {
                let _ = model.lower_assumptions()?;
                assert!(model.take_output().is_empty());
                assert_eq!(model.first_failed_assumption(), Some((0, Ok(false))));
                let output = model.take_output();
                assert_eq!(output.len(), 1);
                assert!(output[0].contains("first"));
                Ok(())
            },
        )
        .unwrap();
    }
    #[test]
    fn cast_errors_match_but_unknown_errors_and_failure_indices_do_not() {
        let dir = fixture("ASSUME 1 + \"a\" = 0");
        assert!(measure(request(dir.path())).unwrap().answers_equal);
        let dir = fixture("ASSUME 1 + <<1>> = 0");
        assert!(!measure(request(dir.path())).unwrap().answers_equal);
        assert!(!Verdict::False { index: 0 }.matches(&Verdict::False { index: 1 }));
        let unknown = Verdict::Error {
            index: 0,
            category: None,
            detail: "same".into(),
        };
        assert!(!unknown.matches(&unknown));
    }
}
