use crate::syntax::{Expr, Failure, parse};
use std::{collections::BTreeMap, path::Path};
use tla_bridge::checker::with_value_session;

fn evaluate(args: &[Expr]) -> Result<String, Failure> {
    let value = || args[1].eval();
    let other = || args[2].eval();
    Ok(match args[0].atom() {
        "eq" => value()?.semantic_eq(&other()?)?.to_string(),
        "cmp" => (value()?.compare(&other()?)? as i8).to_string(),
        "mem" => other()?.contains(&value()?)?.to_string(),
        "size" => value()?.size()?.to_string(),
        "fin" => value()?.finite()?.to_string(),
        "empty" => value()?.empty()?.to_string(),
        "subseteq" => {
            let (value, other) = (value()?, other()?);
            let mut subset = true;
            for v in value.enumerate(100_000)? {
                if !other.contains(&v)? {
                    subset = false;
                    break;
                }
            }
            subset.to_string().to_uppercase()
        }
        other => return Err(Failure::Unsupported(other.into())),
    })
}
#[test]
fn recorded_semantic_value_operations() {
    let mut checked = 0;
    let mut omitted = BTreeMap::new();
    let mut differences = Vec::new();
    for corpus in 1..=6 {
        let (count, skipped, failures) = with_value_session(move || {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tlaplus-rs-explore/crates/tla-tlc-value/tests/oracle");
            let script =
                std::fs::read_to_string(root.join(format!("values-{corpus}.script"))).unwrap();
            let expected =
                std::fs::read_to_string(root.join(format!("values-{corpus}.expected"))).unwrap();
            let mut count = 0;
            let mut skipped = BTreeMap::<String, usize>::new();
            let mut failures = Vec::new();
            assert_eq!(script.lines().count(), expected.lines().count());
            for (line, (source, expected)) in script.lines().zip(expected.lines()).enumerate() {
                let args = parse(source);
                for arg in &args[1..] {
                    if arg.eval().is_err() {
                        break;
                    }
                }
                if matches!(args[0].atom(), "elems" | "nelems") {
                    count += 1;
                    let actual = args[1].eval().and_then(|v| {
                        crate::display::observed(&v, args[0].atom() == "nelems")
                            .map_err(Failure::Value)
                    });
                    match actual {
                        Ok(actual)
                            if !expected.starts_with("ERR ") && !expected.starts_with("NULL") =>
                        {
                            match crate::display::enumeration(expected) {
                                Ok(expected) if actual == expected => (),
                                expected => failures.push(format!(
                                    "{corpus}:{} {source}: expected {expected:?}, got {actual:?}",
                                    line + 1
                                )),
                            }
                        }
                        Err(_) if expected.starts_with("ERR ") || expected.starts_with("NULL") => {}
                        actual => failures.push(format!(
                            "{corpus}:{} {source}: expected {expected}, got {actual:?}",
                            line + 1
                        )),
                    }
                    continue;
                }
                match evaluate(&args) {
                    Err(Failure::Unsupported(op)) => *skipped.entry(op).or_default() += 1,
                    Err(Failure::Value(error)) => {
                        count += 1;
                        if !expected.starts_with("ERR ") && !expected.starts_with("NULL") {
                            failures.push(format!(
                                "{corpus}:{} {source}: expected {expected}, got {error:?}",
                                line + 1
                            ));
                        }
                    }
                    Ok(result) => {
                        count += 1;
                        let expected = if args[0].atom() == "cmp" {
                            expected
                                .parse::<i32>()
                                .map(|n| n.signum().to_string())
                                .unwrap_or_else(|_| expected.into())
                        } else {
                            expected.into()
                        };
                        let expected = expected.split(" {W ").next().unwrap();
                        if result != expected {
                            failures.push(format!(
                                "{corpus}:{} {source}: expected {expected}, got {result}",
                                line + 1
                            ));
                        }
                    }
                }
            }
            (count, skipped, failures)
        });
        checked += count;
        for (op, count) in skipped {
            *omitted.entry(op).or_insert(0) += count;
        }
        differences.extend(failures);
    }
    eprintln!("checked {checked}, omitted {omitted:?}");
    assert!(checked > 500);
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
