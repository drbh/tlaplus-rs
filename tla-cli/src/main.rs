//! The `tla` command: parses arguments, drives `tla-checker`, and renders
//! outcomes and counterexamples the way TLC reports them.
mod trace;
use std::{fmt, num::NonZeroUsize, path::PathBuf};
use tla_checker::{Bound, BoundModel, Completion, Disk, Liveness, Outcome, Settings};

/// Why the command could not finish.
#[derive(Debug)]
enum Error {
    Usage(String),
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    Check(tla_checker::Error),
    /// The timing trace could not be written.
    Trace(String),
    Panicked,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Trace(message) => f.write_str(message),
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Check(e) => e.fmt(f),
            Self::Panicked => f.write_str("checker panicked"),
        }
    }
}

impl From<tla_checker::Error> for Error {
    fn from(e: tla_checker::Error) -> Self {
        Self::Check(e)
    }
}

fn usage(message: impl Into<String>) -> Error {
    Error::Usage(message.into())
}

fn main() {
    // Deeply recursive specifications need more than the default main-thread stack.
    let checker = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| run(std::env::args().skip(1).collect()))
        .expect("start checker thread");
    match checker.join().unwrap_or_else(|_| Err(Error::Panicked)) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    }
}

struct Arguments {
    spec: PathBuf,
    config: Option<PathBuf>,
    workers: NonZeroUsize,
    max_states: Option<NonZeroUsize>,
    check_deadlock: bool,
    continue_after_violation: bool,
    unicode: bool,
    seed: i64,
    libraries: Vec<PathBuf>,
    disk: Option<PathBuf>,
    disk_cache_bytes: NonZeroUsize,
}

impl Arguments {
    fn parse(args: Vec<String>) -> Result<Self, Error> {
        let mut result = Self {
            spec: PathBuf::new(),
            config: None,
            workers: NonZeroUsize::new(1).unwrap(),
            max_states: None,
            check_deadlock: true,
            continue_after_violation: false,
            unicode: false,
            seed: 1,
            libraries: Vec::new(),
            disk: None,
            disk_cache_bytes: NonZeroUsize::new(64 * 1024 * 1024).unwrap(),
        };
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| usage(format!("missing value for {argument}")))
            };
            match argument.as_str() {
                "-config" | "--config" => result.config = Some(PathBuf::from(value()?)),
                "-workers" | "--workers" => {
                    result.workers = value()?
                        .parse()
                        .map_err(|_| usage("workers must be a positive integer"))?
                }
                "--max-states" => {
                    result.max_states = Some(
                        value()?
                            .parse()
                            .map_err(|_| usage("max-states must be a positive integer"))?,
                    )
                }
                "-deadlock" | "--no-deadlock" => result.check_deadlock = false,
                "-continue" | "--continue" => result.continue_after_violation = true,
                "--unicode" => result.unicode = true,
                "-seed" | "--seed" => {
                    result.seed = value()?
                        .parse()
                        .map_err(|_| usage("seed must be an integer"))?
                }
                "--disk" => result.disk = Some(PathBuf::from(value()?)),
                "--disk-cache-bytes" => {
                    result.disk_cache_bytes = value()?
                        .parse()
                        .map_err(|_| usage("disk-cache-bytes must be a positive integer"))?
                }
                "--library" => result.libraries.push(PathBuf::from(value()?)),
                _ if argument.starts_with('-') => {
                    return Err(usage(format!("unsupported flag {argument}")));
                }
                _ if result.spec.as_os_str().is_empty() => result.spec = PathBuf::from(argument),
                _ => return Err(usage("expected exactly one specification")),
            }
        }
        if result.spec.as_os_str().is_empty() {
            return Err(usage("missing specification; use --help for usage"));
        }
        if result.spec.extension().is_none() {
            result.spec.set_extension("tla");
        }
        Ok(result)
    }
}

fn run(args: Vec<String>) -> Result<i32, Error> {
    let path = std::env::var_os("REWRITE_TRACE").map(PathBuf::from);
    let timeline = path.as_ref().map(|_| tla_observe::Timeline::new());
    let result = run_model(args, timeline.as_ref());
    if let (Some(path), Some(timeline)) = (path, timeline) {
        let file = std::fs::File::create(&path).map_err(|error| Error::Io { path, error })?;
        timeline
            .write(file)
            .map_err(|e| Error::Trace(e.to_string()))?;
    }
    result
}

fn run_model(args: Vec<String>, timeline: Option<&tla_observe::Timeline>) -> Result<i32, Error> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Usage: tla [--config FILE] [--workers N] [--no-deadlock] [--continue] [--seed N] [--unicode] [--max-states N] [--library DIR] [--disk FILE --disk-cache-bytes N] SPEC.tla\nChecks finite models with INIT/NEXT or SPECIFICATION configuration. Unsupported language and CLI features produce errors."
        );
        return Ok(0);
    }
    if args.first().is_some_and(|a| a == "--check-syntax") {
        if args.len() != 2 {
            return Err(usage("usage: tla --check-syntax FILE"));
        }
        let path = PathBuf::from(&args[1]);
        let text = std::fs::read_to_string(&path).map_err(|error| Error::Io { path, error })?;
        tla_checker::check_syntax(&text)?;
        return Ok(0);
    }
    if args.first().is_some_and(|arg| arg == "--check") {
        if args.len() != 2 {
            return Err(usage("usage: tla --check FILE"));
        }
        tla_checker::check_module(&PathBuf::from(&args[1]))?;
        return Ok(0);
    }
    let args = Arguments::parse(args)?;
    let loaded = tla_checker::load(
        &args.spec,
        args.config.as_deref(),
        &args.libraries,
        timeline,
    )?;
    let settings = Settings {
        seed: args.seed,
        workers: args.workers,
        max_states: args.max_states,
        check_deadlock: args.check_deadlock,
        continue_after_violation: args.continue_after_violation,
    };
    let mut checker = match tla_checker::bind(loaded, settings)? {
        Bound::AssumptionViolated => {
            println!("ASSUME violated");
            return Ok(1);
        }
        Bound::Trivial => {
            statistics(0, 0, 0);
            println!("Safety checks passed");
            return Ok(0);
        }
        Bound::Ready(checker) => checker,
    };
    let disk = args.disk.map(|path| Disk {
        path,
        cache_bytes: args.disk_cache_bytes,
    });
    let outcome = checker.run(disk.as_ref(), timeline)?;
    report(&checker.model, outcome, trace::Style::new(args.unicode))
}

/// Prints an outcome the way TLC reports it and returns the exit code.
fn report(model: &BoundModel, outcome: Outcome, style: trace::Style) -> Result<i32, Error> {
    statistics(
        outcome.statistics.initial,
        outcome.statistics.distinct,
        outcome.statistics.depth,
    );
    for violation in &outcome.violations {
        trace::violation(model, violation, style);
    }
    for (name, verdict) in &outcome.liveness {
        match verdict {
            Liveness::Holds => println!("Temporal property {name} passed"),
            Liveness::Violated { steps, loop_back } => {
                println!("\nTemporal property {name} violated");
                let steps = steps
                    .iter()
                    .enumerate()
                    .map(|(i, state)| (state, (i > 0).then(|| "Next step".to_owned())))
                    .collect::<Vec<_>>();
                trace::states(&model.program.variables, &steps, style, Some(*loop_back));
                return Ok(1);
            }
        }
    }
    if let Some(name) = outcome.unwitnessed {
        println!("Possible {name} unwitnessed");
        return Ok(1);
    }
    if let Some(name) = outcome.postcondition_violated {
        println!("Postcondition {name} violated");
        return Ok(1);
    }
    let violated = !outcome.violations.is_empty();
    match outcome.completion {
        Completion::Complete if violated => {
            println!("Exploration complete with violations");
            Ok(1)
        }
        Completion::Complete => {
            println!("Safety checks passed");
            Ok(0)
        }
        Completion::StateLimit => {
            println!("Incomplete: state limit reached");
            Ok(3)
        }
        Completion::Violated(violation) => {
            trace::violation(model, &violation, style);
            Ok(1)
        }
    }
}

fn statistics(initial: usize, distinct: usize, depth: usize) {
    println!("{distinct} distinct states found ({initial} initial), search depth {depth}");
}

#[cfg(test)]
mod tests;
