use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};
use tlaplus_bridge::{FaithfulArgs, FaithfulRunMode, ModelRequest, model::Stage, with_loaded_model};
use tlaplus_conformance::{Case, Execution, Request, execute_suite};

#[derive(Debug, Deserialize, Serialize)]
enum Observation {
    Loaded,
    Rejected { code: Option<i32>, detail: String },
    Unresolved(String),
}

#[derive(Deserialize)]
struct Recording {
    module: String,
    cfg: String,
    exit_status: i32,
    messages: Vec<tlaplus_bridge::tool::Message>,
    run: Run,
}

#[derive(Deserialize)]
struct Run {
    run_status: String,
}

#[derive(Clone, Serialize)]
struct Expected {
    code: i32,
    detail: Option<String>,
}

fn expected(recording: &Recording) -> Option<Expected> {
    let errors: Vec<_> = recording
        .messages
        .iter()
        .filter(|m| m.severity == 1)
        .collect();
    if recording.run.run_status != "completed" || errors.len() != 1 {
        return None;
    }
    match (recording.exit_status, errors[0].code) {
        (150, tlaplus_bridge::recording::TLC_PARSING_FAILED) => Some(Expected {
            code: tlaplus_bridge::recording::TLC_PARSING_FAILED,
            detail: None,
        }),
        (255, 1000) => {
            let (_, nested) = errors[0]
                .text
                .split_once("The exception was a tlc2.tool.ConfigFileException\n: ")?;
            let messages = tlaplus_bridge::tool::parse_stream(nested).ok()?;
            let [message] = messages.as_slice() else {
                return None;
            };
            if message.code != 5006 || message.severity != 0 {
                return None;
            }
            Some(Expected {
                code: 1000,
                detail: Some(format!(
                    "tlc2.tool.ConfigFileException: {} [{}]",
                    message.text, message.text
                )),
            })
        }
        _ => None,
    }
}

fn matches(expected: Option<Expected>, actual: &Execution<Observation>) -> bool {
    matches!((expected, actual),
        (Some(expected), Execution::Completed(Observation::Rejected { code: Some(actual), detail }))
        if expected.code == *actual && expected.detail.as_ref().is_none_or(|text|
            text == detail || expected.code == 2230 && detail.strip_prefix(text)
                .is_some_and(|tail| tail.starts_with(" [") && tail.ends_with(']'))))
}

#[derive(Deserialize)]
struct FuzzRecording {
    seed: u64,
    module: String,
    cfg: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
    java: FuzzRun,
}
#[derive(Deserialize)]
struct FuzzRun {
    exit: Option<i32>,
    timeout: bool,
    messages: Option<Vec<FuzzMessage>>,
}
#[derive(Deserialize)]
struct FuzzMessage {
    code: i32,
    text: String,
}
fn expected_fuzz(run: &FuzzRun) -> Option<Expected> {
    if run.timeout {
        return None;
    }
    let code = match run.exit? {
        150 => 3002,
        151 => 2230,
        _ => return None,
    };
    let messages = run.messages.as_ref()?;
    if messages.iter().any(|m| {
        !matches!(
            m.code,
            2262 | 2187 | 2220 | 2219 | 2185 | 2186 | 2144 | 2149 | 3002 | 2230
        )
    }) {
        return None;
    }
    let mut errors = messages.iter().filter(|m| matches!(m.code, 3002 | 2230));
    let error = errors.next()?;
    if error.code != code || errors.next().is_some() {
        return None;
    }
    if code == 2230
        && error
            .text
            .strip_prefix("The invariant of ")
            .and_then(|text| text.split_once(" is equal to "))
            .is_none_or(|(name, value)| name.is_empty() || value.is_empty())
    {
        return None;
    }
    Some(Expected {
        code,
        detail: (code == 2230).then(|| error.text.clone()),
    })
}

fn fuzz_recording(
    case: &Case,
    recordings: &Path,
) -> Result<FuzzRecording, Box<dyn std::error::Error>> {
    let seed: u64 = case
        .id
        .strip_prefix("fuzz/")
        .ok_or("missing fuzz seed")?
        .parse()?;
    let path = recordings.join(format!("seed_{seed}.json.gz"));
    let record: FuzzRecording =
        serde_json::from_reader(flate2::read::GzDecoder::new(fs::File::open(path)?))?;
    let model = request(case.request.clone())?;
    if seed != record.seed
        || fs::read_to_string(&model.spec)? != record.module
        || fs::read_to_string(model.config.as_ref().ok_or("missing config")?)? != record.cfg
    {
        return Err(format!("requested inputs differ from recording {}", case.id).into());
    }
    for (name, text) in &record.files {
        let path = Path::new(name);
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || fs::read_to_string(
                model
                    .spec
                    .parent()
                    .ok_or("missing model directory")?
                    .join(path),
            )? != *text
        {
            return Err(
                format!("companion input differs from recording {}: {name}", case.id).into(),
            );
        }
    }
    Ok(record)
}

fn request(request: Request) -> Result<ModelRequest, String> {
    let parsed = FaithfulArgs::parse(&request.args).map_err(|e| e.0)?;
    if parsed.dfid.is_some()
        || !matches!(parsed.run_mode, FaithfulRunMode::ModelCheck)
        || !request.system_properties.is_empty()
    {
        return Err("unsupported setup options".into());
    }
    let directory = fs::canonicalize(request.user_dir).map_err(|e| e.to_string())?;
    let main = parsed.main_file.ok_or("missing module")?;
    let mut model = ModelRequest::new(directory.join(&main).with_extension("tla"));
    let config = directory.join(parsed.config_file.as_ref().unwrap_or(&main));
    model.config = Some(if config.extension().is_some_and(|ext| ext == "tla") {
        config
    } else {
        config.with_extension("cfg")
    });
    model.seed = parsed.seed.unwrap_or(1);
    model.fingerprint_index = usize::try_from(parsed.fp.unwrap_or(0)).map_err(|e| e.to_string())?;
    model.community_modules = request.community_modules;
    model.library_paths = request
        .library_paths
        .iter()
        .map(|p| directory.join(p))
        .collect();
    Ok(model)
}

fn observe(input: Request) -> Observation {
    let model = match request(input) {
        Ok(model) => model,
        Err(error) => return Observation::Unresolved(error),
    };
    match with_loaded_model(model, |_| Ok(())) {
        Ok(()) => Observation::Loaded,
        Err(error) if error.stage == Stage::Load => Observation::Rejected {
            code: error.code,
            detail: error.message,
        },
        Err(error) => Observation::Unresolved(error.to_string()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let fuzz = args.first().is_some_and(|arg| arg == "--fuzz");
    let args = &args[usize::from(fuzz)..];
    if let [input, output] = args {
        let request: Request = serde_json::from_slice(&fs::read(input)?)?;
        fs::write(output, serde_json::to_vec_pretty(&observe(request))?)?;
        return Ok(());
    }
    let [plan, recordings, output, seconds] = args else {
        return Err("usage: load_rejections [--fuzz] PLAN RECORDINGS REPORT SECONDS".into());
    };
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(plan)?)?;
    let mut recordings_by_id = BTreeMap::new();
    for case in &cases {
        if fuzz {
            let recording = fuzz_recording(case, Path::new(recordings))?;
            recordings_by_id.insert(
                case.id.clone(),
                (expected_fuzz(&recording.java), recording.java.exit),
            );
            continue;
        }
        let path = Path::new(recordings).join(format!("{}.json.gz", case.id.replace('/', "__")));
        let recording: Recording =
            serde_json::from_reader(flate2::read::GzDecoder::new(fs::File::open(path)?))?;
        if recording.cfg != case.id {
            return Err(format!("recording identity differs for {}", case.id).into());
        }
        if let Ok(model) = request(case.request.clone())
            && (!model.spec.ends_with(&recording.module)
                || !model
                    .config
                    .as_ref()
                    .is_some_and(|p| p.ends_with(&recording.cfg)))
        {
            return Err(format!("requested model differs from recording {}", case.id).into());
        }
        recordings_by_id.insert(
            case.id.clone(),
            (expected(&recording), Some(recording.exit_status)),
        );
    }
    let results = execute_suite::<Observation>(
        &std::env::current_exe()?,
        &cases,
        Duration::from_secs(seconds.to_str().ok_or("invalid timeout")?.parse()?),
        |id, result| {
            let equal = matches(recordings_by_id[id].0.clone(), result);
            eprintln!("{}: {id}", if equal { "match" } else { "unresolved" });
        },
    )?;
    let mut count = 0;
    let mut report = BTreeMap::new();
    for (id, actual) in results {
        let (expected, exit_status) = &recordings_by_id[&id];
        let matched = matches(expected.clone(), &actual);
        count += usize::from(matched);
        report.insert(
            id,
            serde_json::json!({
                "expected_code": expected.as_ref().map(|e| e.code),
                "expected": expected,
                "recorded_exit_status": exit_status,
                "actual": actual,
                "load_error_code_matches": matched,
                "load_error_matches": matched,
            }),
        );
    }
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    eprintln!(
        "{count}/{} cases match recorded Java load errors through the shared frontend",
        cases.len()
    );
    if count != cases.len() {
        return Err("unresolved load rejection comparisons".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzz_setup_rejections_require_completed_matching_diagnostics() {
        let mut run: FuzzRun = serde_json::from_value(serde_json::json!({
            "exit": 151, "timeout": false,
            "messages": [{"code": 2230, "text": "The invariant of Inv is equal to FALSE"}]
        }))
        .unwrap();
        let rejected = |detail: &str| {
            Execution::Completed(Observation::Rejected {
                code: Some(2230),
                detail: detail.into(),
            })
        };
        let detail = "The invariant of Inv is equal to FALSE";
        assert!(matches(expected_fuzz(&run), &rejected(detail)));
        assert!(matches(
            expected_fuzz(&run),
            &rejected(&format!("{detail} [Starting...]"))
        ));
        assert!(!matches(
            expected_fuzz(&run),
            &rejected("The invariant of Other is equal to FALSE")
        ));
        assert!(!matches(
            expected_fuzz(&run),
            &rejected(&format!("{detail} trailing error"))
        ));
        run.timeout = true;
        assert!(expected_fuzz(&run).is_none());
        run.timeout = false;
        run.exit = Some(150);
        assert!(expected_fuzz(&run).is_none());
        run.exit = Some(151);
        run.messages.as_mut().unwrap().push(FuzzMessage {
            code: 1000,
            text: "unexpected".into(),
        });
        assert!(expected_fuzz(&run).is_none());
        run.messages.as_mut().unwrap().pop();
        run.messages.as_mut().unwrap()[0].text.clear();
        assert!(expected_fuzz(&run).is_none());
    }

    #[test]
    fn fuzz_recordings_require_identical_model_config_and_companions() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for (name, text) in [
            ("Spec.tla", "module"),
            ("Spec.cfg", "config"),
            ("Support.tla", "support"),
        ] {
            fs::write(root.join(name), text).unwrap();
        }
        let record = serde_json::json!({
            "seed": 7, "module": "module", "cfg": "config", "files": {"Support.tla": "support"},
            "java": {"exit": 150, "timeout": false, "messages": [{"code": 3002, "text": "parse error"}]}
        });
        let mut output = flate2::write::GzEncoder::new(
            fs::File::create(root.join("seed_7.json.gz")).unwrap(),
            flate2::Compression::default(),
        );
        output
            .write_all(&serde_json::to_vec(&record).unwrap())
            .unwrap();
        output.finish().unwrap();
        let case: Case = serde_json::from_value(serde_json::json!({
            "id": "fuzz/7", "request": {"user_dir": root, "args": ["Spec"]}
        }))
        .unwrap();
        assert!(fuzz_recording(&case, root).is_ok());
        for (name, text) in [
            ("Spec.tla", "module"),
            ("Spec.cfg", "config"),
            ("Support.tla", "support"),
        ] {
            fs::write(root.join(name), "changed").unwrap();
            assert!(fuzz_recording(&case, root).is_err());
            fs::write(root.join(name), text).unwrap();
        }
    }

    #[test]
    fn configuration_rejections_require_matching_exception_details() {
        let mut recording = Recording {
            module: "Spec.tla".into(), cfg: "Spec.cfg".into(), exit_status: 255,
            messages: vec![tlaplus_bridge::tool::Message {
                code: 1000, severity: 1,
                text: "The exception was a tlc2.tool.ConfigFileException\n: @!@!@STARTMSG 5006:0 @!@!@\nconfiguration error\n@!@!@ENDMSG 5006 @!@!@".into(),
            }],
            run: Run { run_status: "completed".into() },
        };
        let detail = expected(&recording).unwrap().detail.unwrap();
        let actual = Execution::Completed(Observation::Rejected {
            code: Some(1000),
            detail,
        });
        assert!(matches(expected(&recording), &actual));
        assert!(!matches(
            expected(&recording),
            &Execution::Completed(Observation::Rejected {
                code: Some(1000),
                detail: "another error".into(),
            })
        ));
        recording.messages[0].text = recording.messages[0].text.replace("5006", "5005");
        assert!(!matches(expected(&recording), &actual));
        recording.messages[0].text = "unrelated unexpected exception".into();
        assert!(!matches(expected(&recording), &actual));
    }

    #[test]
    fn loading_uses_only_requested_library_directories() {
        let temp = tempfile::tempdir().unwrap();
        let model = temp.path().join("model");
        let library = temp.path().join("library");
        fs::create_dir(&model).unwrap();
        fs::create_dir(&library).unwrap();
        fs::write(model.join("Spec.tla"), "---- MODULE Spec ----\nEXTENDS Support\nVARIABLE x\nInit == x = Zero\nNext == UNCHANGED x\n====").unwrap();
        fs::write(model.join("Spec.cfg"), "INIT Init\nNEXT Next\n").unwrap();
        fs::write(
            library.join("Support.tla"),
            "---- MODULE Support ----\nZero == 0\n====",
        )
        .unwrap();
        let mut input: Request = serde_json::from_value(serde_json::json!({
            "user_dir": model, "args": ["Spec"]
        }))
        .unwrap();
        assert!(request(input.clone()).unwrap().library_paths.is_empty());
        assert!(matches!(
            observe(input.clone()),
            Observation::Rejected {
                code: Some(3002),
                ..
            }
        ));
        input.library_paths.push(library);
        assert!(matches!(observe(input), Observation::Loaded));
    }

    #[test]
    fn load_gate_requires_explicit_recorded_and_observed_rejections() {
        let mut recording = Recording {
            module: "example.tla".into(),
            cfg: "example.cfg".into(),
            exit_status: 150,
            messages: vec![tlaplus_bridge::tool::Message {
                code: 3002,
                severity: 1,
                text: "Parsing or semantic analysis failed.".into(),
            }],
            run: Run {
                run_status: "completed".into(),
            },
        };
        let rejected = Execution::Completed(Observation::Rejected {
            code: Some(3002),
            detail: "missing library".into(),
        });
        assert!(matches(expected(&recording), &rejected));
        for actual in [
            Execution::Completed(Observation::Loaded),
            Execution::TimedOut,
            Execution::Completed(Observation::Rejected {
                code: Some(1),
                detail: "other".into(),
            }),
            Execution::Completed(Observation::Unresolved("unsupported".into())),
        ] {
            assert!(!matches(expected(&recording), &actual));
        }
        recording.run.run_status = "timeout".into();
        assert!(!matches(expected(&recording), &rejected));
        recording.run.run_status = "completed".into();
        recording.exit_status = 0;
        assert!(!matches(expected(&recording), &rejected));
        recording.exit_status = 150;
        recording.messages.clear();
        assert!(!matches(expected(&recording), &rejected));
    }
}
