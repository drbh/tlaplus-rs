mod native;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, atomic::AtomicUsize, atomic::Ordering},
    time::{Duration, Instant},
};
use tla_bridge::{Answers, Comparison, Outcome};
use tla_conformance::Case;

/// `Match` covers the oracle contract: outcomes, plus counts and depth for successes.
/// `BothFailed` pairs an unclassified native error with a Java rejection or error; it
/// is not evidence that the two failures agree.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Match,
    DfidOutcome,
    Different,
    NativeError,
    BothFailed,
    Inconclusive,
    StateLimit,
    MemoryLimit,
    Timeout,
    Unsupported,
}

#[derive(Serialize)]
struct Result {
    verdict: Verdict,
    expected: Answers,
    actual: Option<Answers>,
    detail: String,
    seconds: f64,
}

/// Resident size in kilobytes, as `ps` reports it.
fn resident_kb(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Runs the checker, killing it on timeout or when it exceeds the memory budget
/// (`REWRITE_MAX_RSS_MB`, default 6000), since the platform enforces no limit.
fn run(
    cli: &Path,
    args: &[String],
    timeout: Duration,
) -> std::result::Result<(Option<i32>, String, String), Verdict> {
    let budget_kb = std::env::var("REWRITE_MAX_RSS_MB")
        .ok()
        .and_then(|mb| mb.parse::<u64>().ok())
        .unwrap_or(6000)
        * 1024;
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| Verdict::Unsupported)?;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(Verdict::Unsupported);
    };
    let readers = (
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            text
        }),
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        }),
    );
    let started = Instant::now();
    let mut checked = started;
    let status = loop {
        let over = if checked.elapsed() >= Duration::from_secs(1) {
            checked = Instant::now();
            resident_kb(child.id()).is_some_and(|kb| kb > budget_kb)
        } else {
            false
        };
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if !over && started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if over {
                    Verdict::MemoryLimit
                } else {
                    Verdict::Timeout
                });
            }
        }
    };
    Ok((
        status.code(),
        readers.0.join().unwrap_or_default(),
        readers.1.join().unwrap_or_default(),
    ))
}

fn classify(expected: &Answers, actual: &Answers, stderr: &str) -> Verdict {
    if actual.outcome == Outcome::Unknown && !stderr.is_empty() {
        return match expected.outcome {
            Outcome::Rejected | Outcome::Unknown => Verdict::BothFailed,
            _ => Verdict::NativeError,
        };
    }
    match tla_bridge::compare(expected, actual) {
        Comparison::Match => Verdict::Match,
        Comparison::Inconclusive
            if actual.mode == tla_bridge::Mode::Dfid
                && expected.outcome == actual.outcome
                && actual.outcome != Outcome::Unknown =>
        {
            Verdict::DfidOutcome
        }
        Comparison::Inconclusive => Verdict::Inconclusive,
        Comparison::Different(_) => Verdict::Different,
    }
}

fn check(
    case: &Case,
    cli: &Path,
    community: &Path,
    expected: Answers,
    timeout: Duration,
) -> Result {
    let started = Instant::now();
    let finish = |verdict, actual, detail: String| Result {
        verdict,
        expected: expected.clone(),
        actual,
        detail: detail.chars().take(2000).collect(),
        seconds: started.elapsed().as_secs_f64(),
    };
    let (args, dfid) = match native::arguments(&case.request, community) {
        Ok(result) => result,
        Err(reason) => return finish(Verdict::Unsupported, None, reason),
    };
    let (exit, stdout, stderr) = match run(cli, &args, timeout) {
        Ok(output) => output,
        Err(verdict) => return finish(verdict, None, String::new()),
    };
    let mut actual = native::answers(&stdout, exit, dfid);
    // TLC's multi-worker search is not level by level, so its depth is no BFS diameter.
    let mut expected = expected.clone();
    if native::workers(&case.request) > 1 {
        (expected.depth, actual.depth) = (Some(0), Some(0));
    }
    let verdict = if stdout.contains("Incomplete: state limit reached") {
        Verdict::StateLimit
    } else {
        classify(&expected, &actual, &stderr)
    };
    finish(verdict, Some(actual), stderr)
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [cli, plan, recordings, report, seconds] = args.as_slice() else {
        return Err(
            "usage: native_corpus NATIVE_CLI PLAN.json RECORDINGS REPORT.json SECONDS".into(),
        );
    };
    let mut cases: Vec<Case> = serde_json::from_slice(&fs::read(plan)?)?;
    if let Ok(filter) = std::env::var("REWRITE_FILTER") {
        cases.retain(|case| case.id.contains(&filter));
    }
    let ids: Vec<_> = cases.iter().map(|case| case.id.clone()).collect();
    let expected = native::expected(recordings, &ids)?;
    let timeout = Duration::from_secs_f64(seconds.to_string_lossy().parse()?);
    let community = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tlaplus-rs-explore/crates/tla-tlc-eval/community_modules");
    let workers: usize = std::env::var("REWRITE_CASE_WORKERS")
        .unwrap_or_else(|_| "1".into())
        .parse()?;
    let next = AtomicUsize::new(0);
    let results = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            scope.spawn(|| {
                while let Some(case) = cases.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let answers = expected.get(&case.id).cloned().unwrap_or_default();
                    let result = check(case, cli, &community, answers, timeout);
                    eprintln!("{:?}: {}", result.verdict, case.id);
                    results.lock().unwrap().insert(case.id.clone(), result);
                }
            });
        }
    });
    let results = results.into_inner()?;
    let mut totals = BTreeMap::<Verdict, usize>::new();
    for result in results.values() {
        *totals.entry(result.verdict).or_default() += 1;
    }
    fs::write(
        report,
        serde_json::to_vec_pretty(&serde_json::json!({ "totals": totals, "cases": results }))?,
    )?;
    eprintln!("{totals:?} of {} cases", results.len());
    Ok(())
}
