use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Request {
    pub args: Vec<String>,
    pub user_dir: PathBuf,
    #[serde(default)]
    pub library_paths: Vec<PathBuf>,
    #[serde(default)]
    pub community_modules: bool,
    #[serde(default)]
    pub system_properties: Vec<(String, String)>,
    #[serde(default)]
    pub engine: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Execution<T> {
    Completed(T),
    TimedOut,
    Failed {
        exit_code: Option<i32>,
        diagnostic: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub request: Request,
}

pub fn examples(models: &Path, recordings: &Path) -> io::Result<Vec<Case>> {
    let models = fs::canonicalize(models)?;
    let mut cases = Vec::new();
    for entry in fs::read_dir(recordings)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "gz") {
            continue;
        }
        let data: serde_json::Value =
            serde_json::from_reader(flate2::read::GzDecoder::new(fs::File::open(path)?))?;
        let field = |name: &str| {
            data[name]
                .as_str()
                .ok_or_else(|| io::Error::other(format!("missing {name}")))
        };
        let module = models.join(field("module")?);
        let config = models.join(field("cfg")?);
        if !module.starts_with(&models)
            || !config.starts_with(&models)
            || module
                .components()
                .chain(config.components())
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(io::Error::other("model paths must stay within the corpus"));
        }
        let mut directory = module
            .parent()
            .ok_or_else(|| io::Error::other("module path has no parent"))?;
        while !config.starts_with(directory) {
            directory = directory
                .parent()
                .ok_or_else(|| io::Error::other("no common model directory"))?;
        }
        cases.push(Case {
            id: field("cfg")?.into(),
            request: Request {
                args: vec![
                    "-tool".into(),
                    "-deadlock".into(),
                    "-workers".into(),
                    "1".into(),
                    "-seed".into(),
                    "1".into(),
                    "-fp".into(),
                    "0".into(),
                    "-config".into(),
                    config
                        .strip_prefix(directory)
                        .unwrap()
                        .to_string_lossy()
                        .into(),
                    module
                        .strip_prefix(directory)
                        .unwrap()
                        .to_string_lossy()
                        .into(),
                ],
                user_dir: directory.into(),
                library_paths: Vec::new(),
                community_modules: false,
                system_properties: vec![],
                engine: false,
            },
        });
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}

pub fn execute_suite<T: serde::de::DeserializeOwned + Send>(
    executable: &Path,
    cases: &[Case],
    timeout: Duration,
    mut progress: impl FnMut(&str, &Execution<T>),
) -> io::Result<BTreeMap<String, Execution<T>>> {
    let workers: usize = std::env::var("REWRITE_CASE_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .map_err(|_| io::Error::other("REWRITE_CASE_WORKERS must be positive"))?;
    execute_batch(executable, cases, timeout, workers, &mut progress)
}

fn execute_batch<T: serde::de::DeserializeOwned + Send>(
    executable: &Path,
    cases: &[Case],
    timeout: Duration,
    workers: usize,
    progress: &mut impl FnMut(&str, &Execution<T>),
) -> io::Result<BTreeMap<String, Execution<T>>> {
    let ids: std::collections::BTreeSet<_> = cases.iter().map(|case| &case.id).collect();
    if ids.len() != cases.len() || cases.is_empty() {
        return Err(io::Error::other(
            "suite must contain unique, nonempty cases",
        ));
    }
    if workers == 0 {
        return Err(io::Error::other("REWRITE_CASE_WORKERS must be positive"));
    }
    let snapshot = tempfile::tempdir()?;
    let executable_snapshot = snapshot.path().join("runner");
    fs::copy(executable, &executable_snapshot)?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results = BTreeMap::new();
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        for _ in 0..workers.min(cases.len()) {
            let (sender, next, executable) = (sender.clone(), &next, &executable_snapshot);
            scope.spawn(move || {
                while let Some(case) =
                    cases.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    let result =
                        execute(executable, &case.request, timeout).unwrap_or_else(|error| {
                            Execution::Failed {
                                exit_code: None,
                                diagnostic: error.to_string(),
                            }
                        });
                    if sender.send((case.id.clone(), result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (id, result) in receiver {
            progress(&id, &result);
            results.insert(id, result);
        }
    });
    Ok(results)
}

fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other(format!(
                "symlink in model input: {}",
                entry.path().display()
            )));
        }
        let dest = target.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

pub fn execute<T: serde::de::DeserializeOwned>(
    executable: &Path,
    request: &Request,
    timeout: Duration,
) -> io::Result<Execution<T>> {
    if timeout.is_zero() {
        return Err(io::Error::other("timeout must be positive"));
    }
    let executable = fs::canonicalize(executable)?;
    let scratch = tempfile::tempdir()?;
    let mut request = request.clone();
    let model = scratch.path().join("model");
    copy_tree(&request.user_dir, &model)?;
    request.user_dir = model;
    let input = scratch.path().join("request.json");
    let output = scratch.path().join("result.json");
    let errors = scratch.path().join("stderr.txt");
    fs::write(&input, serde_json::to_vec(&request)?)?;
    let mut child = Command::new(executable)
        .arg(&input)
        .arg(&output)
        .current_dir(&request.user_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&errors)?)
        .spawn()?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(
                    Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
                );
            }
            result => {
                let _ = child.kill();
                child.wait()?;
                return result.map(|_| Execution::TimedOut);
            }
        }
    };
    if status.success() {
        match fs::File::open(&output).and_then(|file| {
            serde_json::from_reader(io::BufReader::new(file)).map_err(io::Error::from)
        }) {
            Ok(value) => return Ok(Execution::Completed(value)),
            Err(error) => {
                return Ok(Execution::Failed {
                    exit_code: status.code(),
                    diagnostic: format!("invalid result: {error}"),
                });
            }
        }
    }
    let mut diagnostic = String::new();
    fs::File::open(errors)?
        .take(65536)
        .read_to_string(&mut diagnostic)?;
    Ok(Execution::Failed {
        exit_code: status.code(),
        diagnostic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn script(directory: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("worker.sh");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn parallel_cases_are_isolated_and_use_one_executable_snapshot() {
        let inputs = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let worker = script(bin.path(), "cat \"$1\" > \"$2\"");
        let cases: Vec<_> = (0..8)
            .map(|i| Case {
                id: i.to_string(),
                request: Request {
                    args: vec![i.to_string()],
                    user_dir: inputs.path().into(),
                    library_paths: vec![],
                    community_modules: false,
                    system_properties: vec![],
                    engine: false,
                },
            })
            .collect();
        let mut observed = Vec::new();
        let results = execute_batch::<Request>(
            &worker,
            &cases,
            Duration::from_secs(2),
            2,
            &mut |id, result| {
                let Execution::Completed(request) = result else {
                    panic!("{result:?}")
                };
                assert_eq!(request.args, [id]);
                observed.push(id.to_owned());
                script(bin.path(), "exit 7");
            },
        )
        .unwrap();
        observed.sort();
        assert_eq!(observed, results.keys().cloned().collect::<Vec<_>>());
        assert_eq!(results.len(), cases.len());
        assert!(
            execute_batch::<()>(&worker, &cases, Duration::from_secs(1), 0, &mut |_, _| {})
                .is_err()
        );
        assert!(
            execute_batch::<()>(
                &worker,
                &cases[..0],
                Duration::from_secs(1),
                1,
                &mut |_, _| {}
            )
            .is_err()
        );
        let duplicates = vec![cases[0].clone(); 2];
        assert!(
            execute_batch::<()>(
                &worker,
                &duplicates,
                Duration::from_secs(1),
                1,
                &mut |_, _| {}
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn bounds_runs_reaps_workers_and_preserves_inputs() {
        let inputs = tempfile::tempdir().unwrap();
        fs::write(inputs.path().join("model.tla"), "unchanged").unwrap();
        let bin = tempfile::tempdir().unwrap();
        let request = Request {
            args: vec![],
            user_dir: inputs.path().into(),
            library_paths: Vec::new(),
            community_modules: false,
            system_properties: vec![],
            engine: false,
        };
        let worker = script(
            bin.path(),
            "printf changed > model.tla\nprintf '{\"answer\":42}' > \"$2\"",
        );
        let output =
            execute::<serde_json::Value>(&worker, &request, Duration::from_secs(2)).unwrap();
        assert!(matches!(output, Execution::Completed(v) if v["answer"] == 42));
        assert_eq!(
            fs::read_to_string(inputs.path().join("model.tla")).unwrap(),
            "unchanged"
        );
        script(bin.path(), "exec sleep 10");
        let start = Instant::now();
        assert!(matches!(
            execute::<()>(&worker, &request, Duration::from_millis(30)).unwrap(),
            Execution::TimedOut
        ));
        assert!(start.elapsed() < Duration::from_secs(2));
        script(bin.path(), "exit 7");
        assert!(matches!(
            execute::<()>(&worker, &request, Duration::from_secs(2)).unwrap(),
            Execution::Failed {
                exit_code: Some(7),
                ..
            }
        ));
        script(bin.path(), "printf invalid > \"$2\"");
        assert!(matches!(
            execute::<()>(&worker, &request, Duration::from_secs(2)).unwrap(),
            Execution::Failed { .. }
        ));
    }
}
