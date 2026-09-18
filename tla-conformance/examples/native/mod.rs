use std::{collections::BTreeMap, fs, io::Read, path::Path};
use tla_bridge::{Answers, FaithfulArgs, FaithfulRunMode, Mode, Outcome, tool::Message};
use tla_conformance::Request;

/// Native CLI arguments for a recorded TLC request, and whether DFID was requested.
/// DFID runs are checked by BFS; only mode-independent answers are compared.
pub fn arguments(request: &Request, community: &Path) -> Result<(Vec<String>, bool), String> {
    let parsed = FaithfulArgs::parse(&request.args).map_err(|e| e.0)?;
    if !matches!(parsed.run_mode, FaithfulRunMode::ModelCheck) {
        return Err("simulation is outside the answer contract".into());
    }
    if !request.system_properties.is_empty() {
        return Err("Java system properties are not supported".into());
    }
    if parsed.recover.is_some() {
        return Err("checkpoint recovery is outside the answer contract".into());
    }
    let main = parsed.main_file.ok_or("missing module")?;
    let spec = request.user_dir.join(&main).with_extension("tla");
    let config = request
        .user_dir
        .join(parsed.config_file.as_ref().unwrap_or(&main));
    let config = if config
        .extension()
        .is_some_and(|ext| ext == "cfg" || ext == "tla")
    {
        config
    } else {
        config.with_extension("cfg")
    };
    let mut args = vec![
        "--config".into(),
        config.display().to_string(),
        "--workers".into(),
        parsed.workers.max(1).to_string(),
        "--seed".into(),
        parsed.seed.unwrap_or(1).to_string(),
        // Bounds each child's memory; a capped run is reported, never matched.
        "--max-states".into(),
        std::env::var("REWRITE_MAX_STATES").unwrap_or_else(|_| "2000000".into()),
    ];
    if !parsed.deadlock {
        args.push("--no-deadlock".into());
    }
    if parsed.continuation {
        args.push("--continue".into());
    }
    let mut libraries: Vec<_> = request
        .library_paths
        .iter()
        .map(|path| request.user_dir.join(path))
        .collect();
    // Java's recordings had the CommunityModules available; searched last.
    let _ = request.community_modules;
    libraries.push(community.to_path_buf());
    for library in libraries {
        args.extend(["--library".into(), library.display().to_string()]);
    }
    args.push(spec.display().to_string());
    Ok((args, parsed.dfid.is_some()))
}

pub fn workers(request: &Request) -> i32 {
    FaithfulArgs::parse(&request.args).map_or(1, |parsed| parsed.workers)
}

/// Summarizes native CLI output. Continued violations of more than one kind have no
/// single TLC outcome (TLC reports the last one found), so they stay unknown.
pub fn answers(stdout: &str, exit: Option<i32>, dfid: bool) -> Answers {
    let mut answers = Answers {
        mode: if dfid { Mode::Dfid } else { Mode::Bfs },
        ..Answers::default()
    };
    let mut violations = Vec::new();
    let mut complete = false;
    for line in stdout.lines() {
        // "N distinct states found (I initial), search depth D"
        if line.contains(" distinct states found (") {
            let numbers: Vec<u64> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|n| n.parse().ok())
                .collect();
            if let [distinct, initial, depth] = numbers[..] {
                answers.initial_distinct = Some(initial);
                answers.distinct = Some(distinct);
                answers.depth = Some(depth);
            }
        } else if line == "Deadlock reached." {
            violations.push(Outcome::Deadlock);
        } else if line.ends_with(" is violated.") {
            violations.push(match line.split(' ').next() {
                Some("Invariant") => Outcome::InvariantViolated,
                Some("Initial") => Outcome::InitialPropertyViolated,
                Some("Action") => Outcome::ActionPropertyViolated,
                _ => Outcome::Unknown,
            });
        } else if line.starts_with("Temporal property ") && line.ends_with(" violated") {
            violations.push(Outcome::TemporalPropertyViolated);
        } else if line == "ASSUME violated" {
            violations.push(Outcome::AssumptionViolated);
        } else if line == "Safety checks passed" {
            complete = true;
        }
    }
    answers.outcome = match (&violations[..], exit) {
        (_, None | Some(2)) => Outcome::Unknown,
        ([], Some(0)) if complete => Outcome::Success,
        ([first, rest @ ..], Some(1)) if rest.iter().all(|o| o == first) => *first,
        _ => Outcome::Unknown,
    };
    answers
}

/// Recorded Java answers keyed by case ID: a fuzz corpus directory (`fuzz/N`) or
/// anything `Corpus::read` accepts.
pub fn expected(path: &Path, ids: &[String]) -> Result<BTreeMap<String, Answers>, String> {
    let fuzz: Vec<_> = ids
        .iter()
        .filter_map(|id| id.strip_prefix("fuzz/"))
        .collect();
    if fuzz.len() != ids.len() {
        return Ok(tla_bridge::corpus::Corpus::read(path)
            .map_err(|e| e.to_string())?
            .answers);
    }
    let mut result = BTreeMap::new();
    for seed in fuzz {
        let file = fs::File::open(path.join(format!("seed_{seed}.json.gz")))
            .map_err(|e| format!("fuzz/{seed}: {e}"))?;
        let mut text = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        let record: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let answers = match &record["java"] {
            java if java.is_object() && java["timeout"] != true => {
                let messages = java["messages"]
                    .as_array()
                    .ok_or("recording without messages")?
                    .iter()
                    .map(|m| Message {
                        code: m["code"].as_i64().unwrap_or(0) as i32,
                        severity: 0,
                        text: m["text"].as_str().unwrap_or_default().into(),
                    })
                    .collect::<Vec<_>>();
                let exit = java["exit"].as_i64().map(|e| e as i32);
                let mut answers = tla_bridge::tool::extract_messages(&messages, exit)
                    .map_err(|e| e.to_string())?;
                // A run that ended in an exception or evaluation error has no verdict,
                // even if `-continue` reported violations before it.
                if messages.iter().any(|m| matches!(m.code, 1000 | 2283)) {
                    answers.outcome = tla_bridge::Outcome::Unknown;
                }
                answers
            }
            _ => Answers::default(),
        };
        result.insert(format!("fuzz/{seed}"), answers);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_maps_to_tlc_answers() {
        let run = "12 distinct states found (1 initial), search depth 4\nSafety checks passed\n";
        let success = answers(run, Some(0), false);
        assert_eq!(success.outcome, Outcome::Success);
        assert_eq!(
            (success.initial_distinct, success.distinct, success.depth),
            (Some(1), Some(12), Some(4))
        );
        let violated =
            "2 distinct states found (1 initial), search depth 2\n\nInvariant Inv is violated.\n";
        assert_eq!(
            answers(violated, Some(1), false).outcome,
            Outcome::InvariantViolated
        );
        let mixed = "Invariant A is violated.\nAction property B is violated.\n";
        assert_eq!(answers(mixed, Some(1), false).outcome, Outcome::Unknown);
        assert_eq!(answers(run, Some(2), false).outcome, Outcome::Unknown);
        assert_eq!(answers(run, None, false).outcome, Outcome::Unknown);
        assert_eq!(answers(run, Some(0), true).mode, Mode::Dfid);
        let limited = "2 distinct states found (1 initial), search depth 2\nIncomplete: state limit reached\n";
        assert_eq!(answers(limited, Some(3), false).outcome, Outcome::Unknown);
        let live =
            "2 distinct states found (1 initial), search depth 2\nTemporal property P violated\n";
        assert_eq!(
            answers(live, Some(1), false).outcome,
            Outcome::TemporalPropertyViolated
        );
    }

    #[test]
    fn recorded_arguments_map_to_native_flags() {
        let request = Request {
            args: [
                "-deadlock",
                "-continue",
                "-seed",
                "7",
                "-workers",
                "4",
                "-config",
                "M",
            ]
            .into_iter()
            .chain(["-dfid", "3", "-fp", "2", "Spec"])
            .map(String::from)
            .collect(),
            user_dir: "/models".into(),
            library_paths: vec!["lib".into()],
            community_modules: true,
            system_properties: vec![],
            engine: false,
        };
        let (args, dfid) = arguments(&request, Path::new("/cm")).unwrap();
        assert!(dfid);
        assert_eq!(
            args.join(" "),
            "--config /models/M.cfg --workers 4 --seed 7 --max-states 2000000 --no-deadlock --continue \
             --library /models/lib --library /cm /models/Spec.tla"
        );
        let mut simulate = request.clone();
        simulate.args = vec!["-simulate".into(), "Spec".into()];
        assert!(arguments(&simulate, Path::new("/cm")).is_err());
    }
}
