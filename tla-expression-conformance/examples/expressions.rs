use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tla_bridge::{ModelRequest, with_model};
use tla_eval::{Evaluator, Random};
#[derive(Deserialize)]
struct Case {
    id: String,
    spec_dir: String,
    root_module: String,
    cfg: Option<String>,
    exprs: Vec<String>,
}
#[derive(Default, Deserialize, Serialize)]
struct Report {
    cases: usize,
    expressions: usize,
    matched: usize,
    both_errors: usize,
    lowering_errors: BTreeMap<String, Vec<String>>,
    differences: Vec<String>,
    setup_errors: Vec<String>,
    random_state: Option<u64>,
}
fn copy(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let destination = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy(&entry.path(), &destination)?;
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: expressions FAITHFUL_EVAL_ROOT CASES.jsonl REPORT.json".into());
    }
    let root = PathBuf::from(&args[0]).canonicalize()?;
    if std::env::var_os("REWRITE_EXPRESSION_CHILD").is_none() {
        return parent(&args);
    }
    let selected: usize = std::env::var("REWRITE_EXPRESSION_CHILD")?.parse()?;
    let mut report = Report::default();
    for (index, line) in fs::read_to_string(&args[1])?.lines().enumerate() {
        if index != selected {
            continue;
        }
        let case: Case = serde_json::from_str(line)?;
        if std::env::var("REWRITE_FILTER").is_ok_and(|filter| !case.id.contains(&filter)) {
            continue;
        }
        report.cases += 1;
        report.expressions += case.exprs.len();
        let scratch = tempfile::tempdir()?;
        copy(&root.join(&case.spec_dir), scratch.path())?;
        let mut source = format!(
            "---- MODULE RewriteExpressions ----\nEXTENDS {}\n",
            case.root_module
        );
        for (i, expression) in case.exprs.iter().enumerate() {
            source.push_str(&format!("Expression{i} ==\n{expression}\n"));
        }
        source.push_str("====\n");
        let path = scratch.path().join("RewriteExpressions.tla");
        fs::write(&path, source)?;
        let mut request = ModelRequest::new(path);
        request.config = Some(case.cfg.as_ref().map_or_else(
            || "RewriteExpressions.cfg".into(),
            |cfg| {
                let mut path = PathBuf::from(cfg);
                if path.extension().is_none() {
                    path.set_extension("cfg");
                }
                path
            },
        ));
        if case.cfg.is_none() {
            fs::write(scratch.path().join("RewriteExpressions.cfg"), "")?;
        }
        request.community_modules = true;
        let count = case.exprs.len();
        let random = Arc::new(Mutex::new(
            std::env::var("REWRITE_RANDOM_STATE")
                .ok()
                .and_then(|s| s.parse().ok())
                .and_then(|s| Random::from_seed_state(s).ok())
                .unwrap_or_else(|| Random::new(1)),
        ));
        let evaluator_random = Arc::clone(&random);
        match with_model(request, move |model| {
            Ok((0..count)
                .map(|i| {
                    let state = model.random_state();
                    let result = model
                        .constant_expression(&format!("Expression{i}"))
                        .map_err(|e| e.to_string());
                    result.map(|(program, expression, expected)| {
                        let mut evaluator = Evaluator::new(&program, &[], &[], &[]);
                        #[rustfmt::skip]
                        let _ = evaluator_random.lock().map(|mut random| *random = Random::from_seed_state(state).expect("random state"));
                        evaluator.random = Some(&evaluator_random);
                        (expected, evaluator.evaluate(expression))
                    })
                })
                .collect::<Vec<_>>())
        }) {
            Err(error) => report.setup_errors.push(format!("{}: {error}", case.id)),
            Ok(results) => {
                for (i, result) in results.into_iter().enumerate() {
                    let label = format!("{}[{i}]: {}", case.id, case.exprs[i]);
                    match result {
                        Err(error) => report.lowering_errors.entry(error).or_default().push(label),
                        Ok((expected, actual)) => match (expected, actual) {
                            (Ok(expected), Ok(actual)) if expected == actual => report.matched += 1,
                            (Err(_), Err(_)) => report.both_errors += 1,
                            (expected, actual) => report
                                .differences
                                .push(format!("{label}: expected {expected:?}, actual {actual:?}")),
                        },
                    }
                }
            }
        }
        report.random_state = random.lock().ok().map(|random| random.state());
    }
    fs::write(&args[2], serde_json::to_vec_pretty(&report)?)?;
    eprintln!(
        "{} expressions: {} values match, {} both error, {} differences, {} unsupported, {} setup errors",
        report.expressions,
        report.matched,
        report.both_errors,
        report.differences.len(),
        report.lowering_errors.values().map(Vec::len).sum::<usize>(),
        report.setup_errors.len()
    );
    if !report.differences.is_empty()
        || !report.lowering_errors.is_empty()
        || !report.setup_errors.is_empty()
    {
        std::process::exit(1);
    }
    Ok(())
}
fn parent(args: &[std::ffi::OsString]) -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut report = Report::default();
    let mut random_state = Random::new(1).state();
    let timeout = Duration::from_secs(
        std::env::var("REWRITE_TIMEOUT")
            .unwrap_or_else(|_| "10".into())
            .parse()?,
    );
    for (index, line) in fs::read_to_string(&args[1])?.lines().enumerate() {
        let case: Case = serde_json::from_str(line)?;
        if std::env::var("REWRITE_FILTER").is_ok_and(|filter| !case.id.contains(&filter)) {
            continue;
        }
        let scratch = tempfile::tempdir()?;
        let output = scratch.path().join("result.json");
        let mut child = Command::new(std::env::current_exe()?)
            .args(&args[..2])
            .arg(&output)
            .env("REWRITE_EXPRESSION_CHILD", index.to_string())
            .env("REWRITE_RANDOM_STATE", random_state.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if start.elapsed() >= timeout {
                child.kill()?;
                child.wait()?;
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        match fs::read(&output)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Report>(&bytes).ok())
        {
            Some(result) => {
                random_state = result.random_state.unwrap_or(random_state);
                report.cases += result.cases;
                report.expressions += result.expressions;
                report.matched += result.matched;
                report.both_errors += result.both_errors;
                report.differences.extend(result.differences);
                report.setup_errors.extend(result.setup_errors);
                for (error, cases) in result.lowering_errors {
                    report
                        .lowering_errors
                        .entry(error)
                        .or_default()
                        .extend(cases);
                }
            }
            None => {
                report.cases += 1;
                report.expressions += case.exprs.len();
                report.setup_errors.push(format!(
                    "{}: child status {status:?}, {} expressions unresolved",
                    case.id,
                    case.exprs.len()
                ));
            }
        }
        fs::write(&args[2], serde_json::to_vec_pretty(&report)?)?;
    }
    eprintln!(
        "{} expressions: {} values match, {} both error, {} differences, {} unsupported, {} setup/timeout/abort failures",
        report.expressions,
        report.matched,
        report.both_errors,
        report.differences.len(),
        report.lowering_errors.values().map(Vec::len).sum::<usize>(),
        report.setup_errors.len()
    );
    if !report.differences.is_empty()
        || !report.lowering_errors.is_empty()
        || !report.setup_errors.is_empty()
    {
        std::process::exit(1);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_enumeration_errors_preserve_domain_and_range() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/FunctionEnumeration.tla");
        for name in [
            "Domain",
            "Range",
            "Both",
            "SequenceDomain",
            "SequenceRange",
            "NestedDomain",
            "EmptyDomain",
            "EmptyRange",
            "IntervalEmpty",
            "IntervalRange",
            "Finite",
            "RecordField",
            "RecordOrder",
            "RecordSequence",
            "RecordEmpty",
            "RecordFinite",
            "ProductFirst",
            "ProductSecond",
            "ProductBoth",
            "ProductSequence",
            "ProductEmpty",
            "ProductFinite",
        ] {
            let (program, id, expected) = with_model(ModelRequest::new(&path), |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[])
                .evaluate(id)
                .map_err(|e| e.0);
            let expected = expected.map_err(|e| {
                e.strip_prefix("util.Assert$TLCRuntimeException: ")
                    .unwrap_or(&e)
                    .to_owned()
            });
            assert_eq!(actual, expected, "{name}");
        }
    }

    #[test]
    fn integer_comparisons_match_faithful_operand_errors() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/IntegerComparisons.tla");
        for name in [
            "LtLeft", "LtRight", "LeLeft", "LeRight", "GtLeft", "GtRight", "GeLeft", "GeRight",
            "Valid",
        ] {
            let (program, id, expected) = with_model(ModelRequest::new(&path), |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[])
                .evaluate(id)
                .map_err(|e| e.0);
            let expected = expected.map_err(|e| {
                e.strip_prefix("tlc2.tool.EvalException: ")
                    .unwrap_or(&e)
                    .to_owned()
            });
            assert_eq!(actual, expected, "{name}");
        }
    }

    #[test]
    fn naturals_comparison_keeps_its_module_specific_error_name() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/NaturalComparisons.tla");
        for name in ["GeLeft", "GeRight", "GeAlias", "GeHigher", "Valid"] {
            let (program, id, expected) = with_model(ModelRequest::new(&path), |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[])
                .evaluate(id)
                .map_err(|e| e.0);
            let expected = expected.map_err(|e| {
                e.strip_prefix("tlc2.tool.EvalException: ")
                    .unwrap_or(&e)
                    .to_owned()
            });
            assert_eq!(actual, expected, "{name}");
        }
    }

    #[test]
    fn sequence_bounds_and_type_errors_match_faithful() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/SequenceBounds.tla");
        for name in [
            "TupleReversed",
            "StringReversed",
            "ExtremeReversed",
            "Valid",
            "LowerBound",
            "UpperBound",
            "StringBound",
            "BothBounds",
            "EmptyBounds",
            "SecondType",
            "ThirdType",
            "FirstType",
            "InvalidLen",
            "InvalidHead",
            "InvalidTail",
            "InvalidAppend",
        ] {
            let (program, id, expected) = with_model(ModelRequest::new(&path), |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[])
                .evaluate(id)
                .map_err(|e| e.0);
            let expected = expected.map_err(|e| {
                e.strip_prefix("tlc2.tool.EvalException: ")
                    .unwrap_or(&e)
                    .to_owned()
            });
            assert_eq!(actual, expected, "{name}");
        }
    }

    #[test]
    fn infinite_sets_remain_deferred_until_cardinality_or_rendering_requires_them() {
        for (name, error) in [
            ("NaturalCardinality", Some("InfiniteCardinality(\"Nat\")")),
            ("IntegerCardinality", Some("InfiniteCardinality(\"Int\")")),
            ("StringCardinality", Some("InfiniteCardinality(\"STRING\")")),
            ("PowerValue", Some("InfiniteCardinality(\"Nat\")")),
            ("PrintSecondError", Some("division by zero")),
            ("MixedSet", Some("Incomparable")),
            ("Checks", None),
            ("UnionFunction", None),
            ("UnionFunctionText", None),
            ("TupleCardinality", Some("Type(\"enumerable set\")")),
            ("EmptyTupleCardinality", Some("Type(\"enumerable set\")")),
            ("RecordCardinality", Some("Type(\"enumerable set\")")),
            ("FunctionCardinality", Some("Type(\"enumerable set\")")),
            ("LambdaCardinality", Some("Type(\"enumerable set\")")),
            ("EmptySequenceCardinality", Some("Type(\"enumerable set\")")),
            ("SequenceCardinality", Some("Type(\"enumerable set\")")),
            ("ScalarCardinality", Some("Type(\"enumerable set\")")),
            ("SetCardinalities", None),
        ] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/InfiniteOperators.tla");
            let (program, id, expected) = with_model(ModelRequest::new(path), move |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[]).evaluate(id);
            if let Some(error) = error {
                assert!(expected.is_err(), "{name}: {expected:?}");
                if error == "Type(\"enumerable set\")" {
                    assert!(
                        expected
                            .as_ref()
                            .unwrap_err()
                            .contains("Attempted to compute cardinality of the value\n"),
                        "{name}: {expected:?}"
                    );
                }
                if let Some(set) = error
                    .strip_prefix("InfiniteCardinality(\"")
                    .and_then(|s| s.strip_suffix("\")"))
                {
                    let detail = if name == "PowerValue" {
                        format!("overridden value {set}.")
                    } else {
                        format!("cardinality of the value\n{set}")
                    };
                    assert!(
                        expected.as_ref().unwrap_err().contains(&detail),
                        "{name}: {expected:?}"
                    );
                }
                let actual = actual.unwrap_err().0;
                assert!(actual.starts_with(error), "{name}: {actual}");
            } else {
                assert_eq!(actual.unwrap(), expected.unwrap(), "{name}");
            }
        }
    }

    #[test]
    fn fingerprints_match_faithful_for_scalars_atoms_and_nested_values() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/Fingerprints.tla");
        for index in [0, 1, 130] {
            let mut request = ModelRequest::new(&path);
            request.fingerprint_index = index;
            let (program, id, expected) =
                with_model(request, |model| model.constant_expression("Check")).unwrap();
            assert_eq!(
                Evaluator::new(&program, &[], &[], &[])
                    .evaluate(id)
                    .unwrap(),
                expected.unwrap()
            );
        }
    }
    #[test]
    fn configured_operator_tables_preserve_arguments_and_undefined_calls() {
        for name in ["Check", "Undefined"] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/ConfiguredOperators.tla");
            let (program, id, expected) = with_model(ModelRequest::new(path), move |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[]).evaluate(id);
            if name == "Undefined" {
                assert!(expected.is_err() && actual.is_err());
            } else {
                assert_eq!(actual.unwrap(), expected.unwrap());
            }
        }
    }
    #[test]
    fn assert_error_preserves_literal_syntax_and_reports_unknown_error_text() {
        for name in [
            "Checks",
            "Bindings",
            "AliasNoError",
            "HigherCheck",
            "ForwardCheck",
            "NonLiteral",
            "ComputedMessage",
            "WrappedCheck",
            "UnknownText",
        ] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/AssertErrors.tla");
            let (program, id, expected) = with_model(ModelRequest::new(path), move |model| {
                model.constant_expression(name)
            })
            .unwrap();
            let actual = Evaluator::new(&program, &[], &[], &[]).evaluate(id);
            match name {
                "NonLiteral" | "ComputedMessage" | "WrappedCheck" | "ForwardCheck" => {
                    assert!(expected.is_err(), "{name}: {expected:?}");
                    assert_eq!(
                        actual.unwrap_err().0,
                        "AssertError requires a string literal message"
                    );
                }
                "UnknownText" => {
                    assert_eq!(expected.unwrap(), tla_eval::Value::Bool(false));
                    assert!(
                        actual
                            .unwrap_err()
                            .0
                            .starts_with("AssertError cannot compare this error text:")
                    );
                }
                _ => assert_eq!(actual.unwrap(), expected.unwrap(), "{name}"),
            }
        }
    }
}
