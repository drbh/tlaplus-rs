use std::{collections::BTreeSet, sync::Arc};
use tla_bridge::ModelRequest;
use tla_bridge::values::ValueKey;
use tla_conformance::Request;
use tla_disk::{Codec, StateCodec};
use tla_engine::Options;
use tla_model::BoundModel;
use tla_values::Value;

pub type SharedValues = tla_conformance::snapshots::SharedValues<ValueKey>;
pub type Samples = tla_conformance::snapshots::Samples<ValueKey>;
pub use tla_conformance::snapshots::intern_values;

pub fn write_json(
    path: &std::path::Path,
    value: &impl serde::Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    let mut output = std::io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(&mut output, value)?;
    output.flush()?;
    Ok(())
}

pub fn replay(model: &BoundModel, samples: &Samples) -> Result<bool, String> {
    use tla_engine::TransitionSystem;
    for (bytes, expected) in samples {
        let state = StateCodec.decode(bytes).map_err(|e| e.to_string())?;
        let actual = model
            .successors(&state)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|edge| Arc::new(edge.state.iter().map(key).collect()))
            .collect::<BTreeSet<SharedValues>>();
        if &actual != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn trace_length<S>(report: &tla_engine::Report<S>) -> Option<usize> {
    match &report.completion {
        tla_engine::Completion::Violated(violation) => Some(violation.trace.states.len()),
        _ => None,
    }
}
/// Continued violations are compared as a set of kinds with minimal trace lengths.
pub fn outcome<S>(report: &tla_engine::Report<S>) -> String {
    let mut outcome = match &report.completion {
        tla_engine::Completion::Complete => "success".into(),
        tla_engine::Completion::StateLimit => "state limit".into(),
        tla_engine::Completion::Violated(violation) => format!("{:?}", violation.kind),
    };
    let continued: BTreeSet<_> = report
        .violations
        .iter()
        .map(|v| format!("{:?}@{}", v.kind, v.trace.states.len()))
        .collect();
    if !continued.is_empty() {
        outcome = format!("{outcome} continued {continued:?}");
    }
    outcome
}

pub fn answers(outcome: &str, initial: usize, states: usize, depth: usize) -> tla_bridge::Answers {
    use tla_bridge::Outcome;
    let outcome = match outcome {
        "success" => Outcome::Success,
        "Deadlock" => Outcome::Deadlock,
        v if v.starts_with("InitialProperty(") => Outcome::InitialPropertyViolated,
        v if v.starts_with("Invariant(") => Outcome::InvariantViolated,
        v if v.starts_with("ActionProperty(") => Outcome::ActionPropertyViolated,
        _ => Outcome::Unknown,
    };
    tla_bridge::Answers {
        mode: tla_bridge::Mode::Bfs,
        outcome,
        initial_distinct: Some(initial as u64),
        distinct: Some(states as u64),
        depth: Some(depth as u64),
    }
}

pub fn compare_recorded(
    actual: tla_bridge::corpus::Corpus,
    recordings: &std::path::Path,
    output: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if actual.answers.is_empty() {
        return Err("no pipeline evidence".into());
    }
    let mut expected = tla_bridge::corpus::Corpus::read(recordings)?;
    expected
        .answers
        .retain(|id, _| actual.answers.contains_key(id));
    let report = tla_bridge::corpus::compare_corpora(&expected, &actual);
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    if !report.is_match() {
        return Err("recorded safety answers differ or are inconclusive".into());
    }
    Ok(())
}

pub fn configuration(request: &Request) -> Result<(ModelRequest, Options), String> {
    let parsed = tla_bridge::FaithfulArgs::parse(&request.args).map_err(|e| e.0)?;
    let dfid_as_bfs = std::env::var_os("REWRITE_DFID_AS_BFS").is_some();
    if (parsed.dfid.is_some() && !dfid_as_bfs)
        || !matches!(parsed.run_mode, tla_bridge::FaithfulRunMode::ModelCheck)
        || !request.system_properties.is_empty()
    {
        return Err("unsupported run mode/options".into());
    }
    let main = parsed.main_file.ok_or("missing module")?;
    let mut request_model = ModelRequest::new(request.user_dir.join(&main).with_extension("tla"));
    let config = request
        .user_dir
        .join(parsed.config_file.as_ref().unwrap_or(&main));
    request_model.config = Some(if config.extension().is_some_and(|ext| ext == "tla") {
        config
    } else {
        config.with_extension("cfg")
    });
    request_model.seed = parsed.seed.unwrap_or(1);
    request_model.fingerprint_index =
        usize::try_from(parsed.fp.unwrap_or(0)).map_err(|e| e.to_string())?;
    request_model.community_modules = request.community_modules;
    request_model.library_paths = request
        .library_paths
        .iter()
        .map(|path| request.user_dir.join(path))
        .collect();
    let options = Options {
        check_deadlock: parsed.deadlock,
        max_states: Some(
            std::env::var("REWRITE_MAX_STATES")
                .unwrap_or_else(|_| "100000".into())
                .parse()
                .map_err(|_| "REWRITE_MAX_STATES must be positive")?,
        ),
        record_graph: true,
        continue_after_violation: parsed.continuation,
    };
    Ok((request_model, options))
}

pub fn rejection(message: &str, faithful: bool) -> (Option<String>, String) {
    if message.starts_with("Attempted to apply the operator overridden by the Java method\n")
        && message.contains("\nbut it produced the following error:\nCannot cast ")
    {
        return (Some("method_argument_cast".into()), message.into());
    }
    if message.starts_with("Attempted to apply the operator ~ to a non-boolean\n")
        || message.starts_with("A non-boolean expression ")
        || ["initial states", "next states", "ENABLED"]
            .iter()
            .any(|phase| {
                message.starts_with(&format!("In computing {phase}, a non-boolean expression"))
            })
        || message.starts_with("TLC expected a boolean value, but did not find one.")
    {
        return (Some("boolean_context".into()), message.into());
    }
    if message.starts_with("In applying the function\n")
        && message.ends_with("\nwhich is not in its domain.\n")
    {
        return (Some("function_domain".into()), message.into());
    }
    if message.starts_with("Attempted to check equality of integer ")
        && message.contains(" with non-integer:\n")
    {
        return (Some("integer_equality".into()), message.into());
    }
    if message.starts_with("Attempted to check if the value:\n")
        && message.contains("\nis an element of the string ")
    {
        return (Some("string_membership".into()), message.into());
    }
    if let Some(value) = message
        .strip_prefix("The second argument of SortSeq should be a operator, but instead it is:\n")
    {
        return (Some("sort_comparator".into()), value.into());
    }
    let runtime_message = if faithful {
        message
            .rsplit("but it produced the following error:\n")
            .next()
            .unwrap_or(message)
    } else {
        message
    };
    if runtime_message.starts_with("Attempted to enumerate S \\cup T when S:\n") {
        return (Some("union_enumeration".into()), runtime_message.into());
    }
    if let Some(name) = runtime_message
        .strip_prefix("TLCGet(")
        .and_then(|rest| rest.strip_suffix(") was undefined."))
    {
        return (Some("undefined_register".into()), name.into());
    }
    let cardinality = if faithful {
        message
            .rsplit("but it produced the following error:\n")
            .next()
            .unwrap_or(message)
            .strip_prefix("Attempted to compute the number of elements in the overridden value ")
            .and_then(|name| name.strip_suffix('.'))
            .or_else(|| message.strip_prefix("Attempted to compute cardinality of the value\n"))
    } else {
        message
            .strip_prefix("InfiniteCardinality(\"")
            .and_then(|name| name.strip_suffix("\")"))
    };
    if let Some(name @ ("Nat" | "Int" | "STRING")) = cardinality {
        return (Some("infinite_cardinality".into()), name.into());
    }
    if let Some(names) = message.strip_prefix("unassigned variables: ") {
        let names: Vec<_> = names.split(", ").collect();
        if names
            .iter()
            .all(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        {
            return (Some("incomplete_state".into()), names.join(", "));
        }
    }
    let prefix = if faithful {
        "The first argument of Assert evaluated to FALSE; the second argument was:\n"
    } else {
        "assertion failed: "
    };
    if let Some(detail) = message.strip_prefix(prefix) {
        return (Some("assertion".into()), detail.trim().into());
    }
    let modulus_prefix = if faithful {
        "The second argument of % should be a positive number, but instead it is:\n"
    } else {
        "non-positive modulus: "
    };
    if let Some(value) = message
        .strip_prefix(modulus_prefix)
        .and_then(|n| n.parse::<i32>().ok())
        && value <= 0
    {
        return (Some("non_positive_modulus".into()), value.to_string());
    }
    let domain = if faithful {
        message
            .strip_prefix("TLC encountered the non-enumerable quantifier bound\n")
            .and_then(|rest| rest.split_once('\n').map(|(domain, _)| domain))
            .or_else(|| {
                message
                    .strip_prefix("Attempted to enumerate { x \\in S : p(x) } when S:\n")
                    .and_then(|rest| rest.strip_suffix("\nis not enumerable"))
            })
    } else {
        message.strip_prefix("non-enumerable domain: ")
    };
    if let Some(domain) = domain {
        return (Some("non_enumerable_domain".into()), domain.into());
    }
    let message_without_location = message.split_once("\nline ").map_or(message, |(m, _)| m);
    for position in ["first", "second"] {
        if let Some(rest) =
            message_without_location.strip_prefix(&format!("The {position} argument of "))
            && let Some((op, _)) = rest.split_once(" should be an integer, but instead it is:\n")
            && matches!(op, "<" | "<=" | ">" | ">=")
        {
            return (
                Some("integer_comparison".into()),
                message_without_location.into(),
            );
        }
    }
    for (category, prefix) in [
        (
            "record_enumeration",
            "Attempted to enumerate a set of the form [l1 : v1, ..., ln : vn],\n",
        ),
        (
            "product_enumeration",
            "Attempted to enumerate a set of the form s1 \\X s2 ... \\X sn,\n",
        ),
        (
            "function_enumeration",
            "Attempted to enumerate a set of the form [D -> R],but the ",
        ),
        ("infinite_membership", "Attempted to check if the value:\n"),
        ("tuple_index", "Attempted to access index "),
        ("record_field", "Attempted to select nonexistent field "),
        (
            "subsequence_bound",
            "The second argument of SubSeq must be in the domain of its first argument:\n",
        ),
        (
            "subsequence_bound",
            "The third argument of SubSeq must be in the domain of its first argument:\n",
        ),
        (
            "sequence_type",
            "The argument of Len should be a sequence, but instead it is:\n",
        ),
        (
            "sequence_type",
            "The argument of Head should be a sequence, but instead it is:\n",
        ),
        (
            "sequence_type",
            "The argument of Tail should be a sequence, but instead it is:\n",
        ),
        (
            "sequence_type",
            "Evaluating an expression of the form Append(s, v) when s is not a sequence:\n",
        ),
    ] {
        if message_without_location.starts_with(prefix) {
            return (Some(category.into()), message_without_location.into());
        }
    }
    for (category, expected, actual) in [
        (
            "applied_interval",
            "A non-function (a set of the form i..j) was applied as a function.",
            "A non-function (a set of the form i..j) was applied as a function.",
        ),
        (
            "division_by_zero",
            "The second argument of \\div is 0.",
            "division by zero",
        ),
        (
            "empty_head",
            "Attempted to apply Head to the empty sequence.",
            "Head of empty sequence",
        ),
        (
            "empty_tail",
            "Attempted to apply Tail to the empty sequence.",
            "Tail of empty sequence",
        ),
        (
            "case_without_branch",
            "Attempted to evaluate a CASE with no conditions true.",
            "CASE has no true condition",
        ),
        (
            "choose_without_witness",
            "Attempted to compute the value of an expression of form\nCHOOSE x \\in S: P, but no element of S satisfied P.",
            "CHOOSE has no satisfying element",
        ),
    ] {
        if message_without_location == if faithful { expected } else { actual } {
            return (Some(category.into()), actual.into());
        }
    }
    (None, message.into())
}

pub fn key(value: &Value) -> ValueKey {
    match value {
        Value::Bool(value) => ValueKey::Bool(*value),
        Value::Int(value) => ValueKey::Int(*value),
        Value::String(atom) => ValueKey::String(atom.text.to_string()),
        Value::Model(atom) => ValueKey::Model(atom.text.to_string()),
        Value::Set(values) => {
            let mut values: Vec<_> = values.iter().map(key).collect();
            values.sort();
            ValueKey::Set(values)
        }
        Value::Function(entries) => {
            let mut entries: Vec<_> = entries.iter().map(|(k, v)| (key(k), key(v))).collect();
            entries.sort();
            ValueKey::Function(entries)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_library_paths_are_explicit() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("Spec.tla"), "---- MODULE Spec ----\nEXTENDS FiniteSetsExt\nVARIABLE x\nInit == x = 0\nNext == UNCHANGED x\n====").unwrap();
        std::fs::write(temp.path().join("Spec.cfg"), "INIT Init\nNEXT Next\n").unwrap();
        let mut input: Request =
            serde_json::from_value(serde_json::json!({"user_dir": temp.path(), "args": ["Spec"]}))
                .unwrap();
        let (model, _) = configuration(&input).unwrap();
        assert!(model.library_paths.is_empty());
        let error = tla_bridge::with_loaded_model(model, |_| Ok(())).unwrap_err();
        assert_eq!(error.code, Some(tla_bridge::recording::TLC_PARSING_FAILED));
        input.library_paths.push(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tlaplus-rs-explore/crates/tla-tlc-eval/community_modules"),
        );
        let (model, _) = configuration(&input).unwrap();
        tla_bridge::with_loaded_model(model, |_| Ok(())).unwrap();
    }

    #[test]
    fn recorded_projection_preserves_complete_counts_and_unknown_outcomes() {
        use tla_bridge::{Comparison, compare};
        assert_ne!(
            compare(&answers("success", 1, 3, 3), &answers("success", 1, 4, 3)),
            Comparison::Match
        );
        assert_eq!(
            compare(
                &answers("Invariant(\"Inv\")", 1, 3, 3),
                &answers("Invariant(\"Inv\")", 1, 4, 3)
            ),
            Comparison::Match
        );
        assert_eq!(
            compare(
                &answers("state limit", 1, 3, 3),
                &answers("state limit", 1, 3, 3)
            ),
            Comparison::Inconclusive
        );
    }
}
