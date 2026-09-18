use serde::Serialize;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tla_bridge::{Mode, Outcome, RunEnv, observe_faithful};

#[derive(Serialize)]
struct Case {
    round: usize,
    seed: u64,
    workers: usize,
    matched: bool,
    expected: tla_bridge::Answers,
    faithful: tla_bridge::ObservedRun,
    exit: Option<i32>,
    output: String,
    diagnostic: String,
}

fn run(
    binary: &Path,
    spec: &Path,
    workers: usize,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let mut child = Command::new(binary)
        .arg("--workers")
        .arg(workers.to_string())
        .arg(spec)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err("native checker timeout".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
}
fn random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [binary, output] = args.as_slice() else {
        return Err("usage: native_fuzz NATIVE_CLI REPORT.json".into());
    };
    let binary = Path::new(binary).canonicalize()?;
    let mut results = Vec::new();
    for round in 0..2 {
        let mut seed = 0x839f71c9843u64 + round as u64;
        for _ in 0..64 {
            let case_seed = seed;
            let n = 2 + random(&mut seed) % 8;
            let m = 2 + random(&mut seed) % 5;
            let mode = random(&mut seed) % 6;
            let bound = if mode == 1 { n / 2 } else { n - 1 };
            let directory = tempfile::tempdir()?;
            let spec = directory.path().join("Fuzz.tla");
            fs::write(
                &spec,
                format!(
                    "---- MODULE Fuzz ----\nEXTENDS Naturals\nVARIABLES x, y\nInit == x = 0 /\\ y \\in 0..{}\nAdvance == x' = (x+1)%{n} /\\ y' = y\nNext == Advance \\/ (x' = x /\\ y' = (y+1)%{m})\nSafe == x <= {bound}\nKeep == x < {}\nStep == y' <= y\nvars == <<x,y>>\nNoChange == [][UNCHANGED x]_vars\nSpec == Init /\\ [][Next]_vars /\\ WF_vars(Advance)\nLive == <> (x = {})\n====\n",
                    m - 1,
                    n - 1,
                    n - 1
                ),
            )?;
            let cfg = if mode == 5 {
                "SPECIFICATION Spec\nPROPERTY Live\n".to_owned()
            } else {
                format!(
                    "INIT Init\nNEXT Next\nINVARIANT Safe\n{}",
                    match mode {
                        2 => "CONSTRAINT Keep\n",
                        3 => "ACTION_CONSTRAINT Step\n",
                        4 => "PROPERTY NoChange\n",
                        _ => "",
                    }
                )
            };
            fs::write(directory.path().join("Fuzz.cfg"), cfg)?;
            let expected = observe_faithful(
                vec![
                    "-workers".into(),
                    "1".into(),
                    "-seed".into(),
                    "1".into(),
                    "-noGenerateSpecTE".into(),
                    "-metadir".into(),
                    directory.path().join("states").to_string_lossy().into(),
                    "Fuzz".into(),
                ],
                RunEnv {
                    user_dir: directory.path().into(),
                    max_heap_bytes: Some(64 << 20),
                    max_direct_memory_bytes: Some(16 << 20),
                    ..RunEnv::default()
                },
            )?;
            for workers in [1, 4] {
                let actual = run(&binary, &spec, workers)?;
                let output = String::from_utf8_lossy(&actual.stdout).into_owned();
                let diagnostic = String::from_utf8_lossy(&actual.stderr).into_owned();
                let matched = expected.summary.mode == Mode::Bfs
                    && match expected.summary.outcome {
                        Outcome::Success => {
                            actual.status.code() == Some(0)
                                && expected.summary.distinct.is_some_and(|n| {
                                    output.contains(&format!("\n{n} distinct states found"))
                                        || output.starts_with(&format!("{n} distinct states found"))
                                })
                                && expected
                                    .summary
                                    .depth
                                    .is_some_and(|n| output.contains(&format!("depth={n}\n")))
                        }
                        Outcome::InvariantViolated => {
                            actual.status.code() == Some(1)
                                && output.contains("Invariant(\"Safe\")")
                        }
                        Outcome::ActionPropertyViolated => {
                            actual.status.code() == Some(1)
                                && output.contains("ActionProperty(\"NoChange\")")
                        }
                        Outcome::Deadlock => {
                            actual.status.code() == Some(1) && output.contains("Deadlock")
                        }
                        Outcome::TemporalPropertyViolated => {
                            actual.status.code() == Some(1)
                                && output.contains("Temporal property Live violated")
                        }
                        _ => false,
                    };
                results.push(Case {
                    round,
                    seed: case_seed,
                    workers,
                    matched,
                    expected: expected.summary.clone(),
                    faithful: expected.clone(),
                    exit: actual.status.code(),
                    output,
                    diagnostic,
                });
            }
        }
        eprintln!(
            "round {round}: {} matching checks",
            results
                .iter()
                .filter(|r| r.round == round && r.matched)
                .count()
        );
        fs::write(output, serde_json::to_vec_pretty(&results)?)?;
    }
    if results.iter().any(|r| !r.matched) {
        std::process::exit(1);
    }
    Ok(())
}
