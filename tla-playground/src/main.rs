//! Isolated native playground worker: request JSON in, structured result file out.
mod render;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{num::NonZeroUsize, path::Path};
use tla_checker::{Bound, Settings};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    language: String,
    source: String,
    #[serde(default)]
    config: String,
    max_states: usize,
}

fn execute(request: Request) -> Result<Value, Value> {
    let error = |e: String| json!({"status":"error", "message":e});
    if request.source.len() + request.config.len() > 256_000 {
        return Err(error("Source exceeds 256 KB".into()));
    }
    #[cfg(target_os = "wasi")]
    let dir = tempfile::tempdir_in("/tmp").map_err(|e| error(e.to_string()))?;
    #[cfg(not(target_os = "wasi"))]
    let dir = tempfile::tempdir().map_err(|e| error(e.to_string()))?;
    let (module, source, config, artifact) = match request.language.as_str() {
        "tsl" => {
            let artifact = tla_dsl::compile(&request.source).map_err(|e| {
                json!({
                    "status":"error", "message":e.message,
                    "location":{"line":e.location.line,"column":e.location.column}
                })
            })?;
            if artifact.config.is_empty() {
                return Err(error(
                    "Select a spec; a library has no state machine to run".into(),
                ));
            }
            (
                artifact.module.clone(),
                artifact.tla.clone(),
                artifact.config.clone(),
                Some(artifact),
            )
        }
        "tla" => {
            let parsed = tla_parser::module(&request.source).map_err(|e| error(e.to_string()))?;
            (parsed.name, request.source, request.config, None)
        }
        _ => return Err(error("Language must be tsl or tla".into())),
    };
    if !module
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || module.is_empty()
    {
        return Err(error("Invalid module name".into()));
    }
    let path = dir.path().join(format!("{module}.tla"));
    std::fs::write(&path, &source).map_err(|e| error(e.to_string()))?;
    if !config.trim().is_empty() {
        std::fs::write(path.with_extension("cfg"), &config).map_err(|e| error(e.to_string()))?;
    }
    let loaded = tla_checker::load(&path, None, &[], None).map_err(|e| error(e.to_string()))?;
    let settings = Settings {
        max_states: NonZeroUsize::new(request.max_states.clamp(1, 2000)),
        ..Settings::default()
    };
    let mut checker = match tla_checker::bind(loaded, settings).map_err(|e| error(e.to_string()))? {
        Bound::AssumptionViolated => {
            return Ok(
                json!({"status":"assumption", "message":"An ASSUME is false; no states explored."}),
            );
        }
        Bound::Trivial => {
            return Ok(
                json!({"status":"trivial", "message":"Assumptions checked; no state machine configured."}),
            );
        }
        Bound::Ready(checker) => checker,
    };
    let outcome = checker.run_with_graph().map_err(|e| error(e.to_string()))?;
    Ok(render::outcome(&checker.model, outcome, artifact.as_ref()))
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: tla-playground REQUEST.json RESULT.json");
        std::process::exit(2);
    }
    let result = std::fs::read(&args[1])
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).map_err(|e| e.to_string()))
        .map(execute);
    let result = match result {
        Ok(Ok(value)) | Ok(Err(value)) => value,
        Err(message) => json!({"status":"error", "message":message}),
    };
    if let Err(error) = std::fs::write(Path::new(&args[2]), serde_json::to_vec(&result).unwrap()) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
