mod liveness;
mod trace;
use std::{num::NonZeroUsize, path::PathBuf};
use tla_engine::{Completion, Options, explore, explore_parallel};
use tla_ir::Level;
use tla_model::BoundModel;

fn main() {
    // Deeply recursive specifications need more than the default main-thread stack.
    let checker = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| run(std::env::args().skip(1).collect()))
        .expect("start checker thread");
    match checker
        .join()
        .unwrap_or_else(|_| Err("checker panicked".into()))
    {
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
    fn parse(args: Vec<String>) -> Result<Self, String> {
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
                    .ok_or_else(|| format!("missing value for {argument}"))
            };
            match argument.as_str() {
                "-config" | "--config" => result.config = Some(PathBuf::from(value()?)),
                "-workers" | "--workers" => {
                    result.workers = value()?
                        .parse()
                        .map_err(|_| "workers must be a positive integer")?
                }
                "--max-states" => {
                    result.max_states = Some(
                        value()?
                            .parse()
                            .map_err(|_| "max-states must be a positive integer")?,
                    )
                }
                "-deadlock" | "--no-deadlock" => result.check_deadlock = false,
                "-continue" | "--continue" => result.continue_after_violation = true,
                "--unicode" => result.unicode = true,
                "-seed" | "--seed" => {
                    result.seed = value()?.parse().map_err(|_| "seed must be an integer")?
                }
                "--disk" => result.disk = Some(PathBuf::from(value()?)),
                "--disk-cache-bytes" => {
                    result.disk_cache_bytes = value()?
                        .parse()
                        .map_err(|_| "disk-cache-bytes must be a positive integer")?
                }
                "--library" => result.libraries.push(PathBuf::from(value()?)),
                _ if argument.starts_with('-') => {
                    return Err(format!("unsupported flag {argument}"));
                }
                _ if result.spec.as_os_str().is_empty() => result.spec = PathBuf::from(argument),
                _ => return Err("expected exactly one specification".into()),
            }
        }
        if result.spec.as_os_str().is_empty() {
            return Err("missing specification; use --help for usage".into());
        }
        if result.spec.extension().is_none() {
            result.spec.set_extension("tla");
        }
        Ok(result)
    }
}

fn run(args: Vec<String>) -> Result<i32, String> {
    let path = std::env::var_os("REWRITE_TRACE").map(PathBuf::from);
    let timeline = path.as_ref().map(|_| tla_observe::Timeline::new());
    let result = run_model(args, timeline.as_ref());
    if let (Some(path), Some(timeline)) = (path, timeline) {
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        timeline.write(file).map_err(|e| e.to_string())?;
    }
    result
}

fn stage<T>(
    timeline: Option<&tla_observe::Timeline>,
    name: &'static str,
    operation: impl FnOnce() -> T,
) -> T {
    match timeline {
        Some(timeline) => timeline.span(name, operation),
        None => operation(),
    }
}

fn run_model(args: Vec<String>, timeline: Option<&tla_observe::Timeline>) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Usage: tla [--config FILE] [--workers N] [--no-deadlock] [--continue] [--seed N] [--unicode] [--max-states N] [--library DIR] [--disk FILE --disk-cache-bytes N] SPEC.tla\nChecks finite models with INIT/NEXT or SPECIFICATION configuration. Unsupported language and CLI features produce errors."
        );
        return Ok(0);
    }
    if args.first().is_some_and(|a| a == "--check-syntax") {
        if args.len() != 2 {
            return Err("usage: tla --check-syntax FILE".into());
        }
        let text = std::fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
        tla_parser::parse(&text).map_err(|e| e.to_string())?;
        return Ok(0);
    }
    if args.first().is_some_and(|arg| arg == "--check") {
        if args.len() != 2 {
            return Err("usage: tla --check FILE".into());
        }
        let source = tla_modules::load(&PathBuf::from(&args[1]), &[]).map_err(|e| e.to_string())?;
        tla_lower::lower(&source, &tla_modules::Config::default()).map_err(|e| e.to_string())?;
        return Ok(0);
    }
    let args = Arguments::parse(args)?;
    // Without --config: the sibling .cfg, or else the spec's own CONFIG section.
    let sibling = args.spec.with_extension("cfg");
    let embedded = !sibling.exists()
        && std::fs::read_to_string(&args.spec)
            .is_ok_and(|text| !tla_modules::embedded_config(&text).trim().is_empty());
    let config_path = match args.config {
        Some(path) => path,
        None if embedded => args.spec.clone(),
        None => sibling,
    };
    let mut text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    if config_path.extension().is_some_and(|ext| ext == "tla") {
        text = tla_modules::embedded_config(&text);
    }
    let mut config = tla_modules::config(&text).map_err(|e| e.to_string())?;
    let mut source = stage(timeline, "load", || {
        tla_modules::load(&args.spec, &args.libraries)
    })
    .map_err(|e| e.to_string())?;
    tla_modules::override_operators(&mut source, &mut config).map_err(|e| e.to_string())?;
    let temporal = tla_temporal::prepare(&mut source, &mut config).map_err(|e| e.to_string())?;
    let lowered = stage(timeline, "lower", || tla_lower::lower(&source, &config))
        .map_err(|e| e.to_string())?;
    if config.specification.is_some() {
        return Err(
            "SPECIFICATION temporal decomposition is not implemented; use INIT/NEXT".into(),
        );
    }
    let mut model = BoundModel::new(lowered.program);
    model.seed(args.seed);
    model
        .configure_constants(&lowered.bindings, &lowered.overrides)
        .map_err(|e| e.to_string())?;
    model
        .cache_constants(0..source.definitions.len())
        .map_err(|e| e.to_string())?;
    if model
        .setup_assumptions(&lowered.assumptions, args.workers)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        println!("ASSUME violated");
        return Ok(1);
    }
    let failures = lowered.failures;
    let resolve = |name: &str| -> Result<usize, String> {
        if let Some(error) = failures.get(name) {
            return Err(format!("{name}: {error}"));
        }
        let op = model
            .program
            .operators
            .iter()
            .find(|op| op.name == name)
            .ok_or_else(|| format!("unknown configured operator {name}"))?;
        if !op.parameters.is_empty() {
            return Err(format!("configured operator {name} requires arguments"));
        }
        Ok(op.body)
    };
    if config.init.is_none() && config.next.is_none() && model.program.variables.is_empty() {
        // Like TLC, a model without variables or behavior only checks its assumptions.
        statistics(0, 0, 0);
        println!("Safety checks passed");
        return Ok(0);
    }
    let initial = config
        .init
        .as_deref()
        .ok_or("configuration requires INIT")?;
    let next = config
        .next
        .as_deref()
        .ok_or("configuration requires NEXT")?;
    let init = resolve(initial)?;
    if model.program.nodes[init].level > Level::State {
        return Err("INIT must have state level".into());
    }
    let action = resolve(next)?;
    if model.program.nodes[action].level > Level::Action {
        return Err("NEXT must have action level".into());
    }
    let invariants = config
        .invariants
        .iter()
        .map(|name| Ok((name.clone(), resolve(name)?)))
        .chain(
            temporal
                .invariants
                .iter()
                .map(|(name, predicate)| Ok((name.clone(), resolve(predicate)?))),
        )
        .collect::<Result<Vec<_>, String>>()?;
    let properties = temporal
        .initial_properties
        .iter()
        .map(|(name, predicate)| Ok((name.clone(), resolve(predicate)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let action_properties = temporal
        .action_properties
        .iter()
        .map(|(name, predicate)| Ok((name.clone(), resolve(predicate)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let state_constraints = config
        .constraints
        .iter()
        .map(|name| resolve(name))
        .collect::<Result<Vec<_>, _>>()?;
    let action_constraints = config
        .action_constraints
        .iter()
        .map(|name| resolve(name))
        .collect::<Result<Vec<_>, _>>()?;
    let view = config.view.as_deref().map(resolve).transpose()?;
    let postconditions = config
        .postconditions
        .iter()
        .map(|name| Ok((name.clone(), resolve(name)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let possible = config
        .possible
        .iter()
        .map(|name| Ok((name.clone(), resolve(name)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let symmetry = config.symmetry.as_deref().map(resolve).transpose()?;
    if let Some(id) = symmetry {
        if model.program.nodes[id].level != Level::Constant {
            return Err("SYMMETRY must have constant level".into());
        }
        let permutations = model.evaluate(id, &[], &[]).map_err(|e| e.to_string())?;
        for permutation in permutations.elements().map_err(|e| e.to_string())? {
            let entries = permutation.entries().map_err(|e| e.to_string())?.to_vec();
            let domain = entries
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            let range = entries
                .iter()
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>();
            if domain
                .iter()
                .any(|key| !matches!(key, tla_model::Value::Model(_)))
                || !domain.iter().all(|key| range.contains(key))
            {
                return Err("SYMMETRY requires permutations of model values".into());
            }
            model.symmetry.push(entries);
        }
    }
    let mut invariants = invariants;
    for (_, id) in &invariants {
        if model.program.nodes[*id].level > Level::State {
            return Err("INVARIANT must have state level".into());
        }
    }
    // Like TLC, a constant invariant is checked once: dropped if TRUE, fatal if not.
    for (name, id) in &invariants {
        if model.program.nodes[*id].level == Level::Constant
            && model.evaluate(*id, &[], &[]).map_err(|e| e.to_string())?
                != tla_model::Value::Bool(true)
        {
            return Err(format!("The invariant of {name} is equal to FALSE"));
        }
    }
    invariants.retain(|(_, id)| model.program.nodes[*id].level != Level::Constant);
    for id in &state_constraints {
        if model.program.nodes[*id].level > Level::State {
            return Err("CONSTRAINT must have state level".into());
        }
    }
    if view.is_some_and(|id| model.program.nodes[id].level > Level::State) {
        return Err("VIEW must have state level".into());
    }
    model.init = vec![init];
    model.actions = trace::actions(&mut model.program, next, action);
    let style = trace::Style::new(args.unicode);
    model.invariants = invariants;
    model.action_properties = action_properties;
    model.state_constraints = state_constraints;
    model.action_constraints = action_constraints;
    model.view = view;
    for (name, id) in properties {
        match model.program.nodes[id].level {
            Level::Constant | Level::State => model.initial_properties.push((name, id)),
            Level::Action => {
                return Err(format!(
                    "PROPERTY {name} has action level; use [][Action]_vars"
                ));
            }
            Level::Temporal => {
                return Err(format!("PROPERTY {name} is not a supported formula"));
            }
        }
    }
    let live = tla_temporal::instantiate(&mut model, &temporal)?;
    model.check_deadlock = args.check_deadlock && config.check_deadlock.unwrap_or(true);
    let options = Options {
        check_deadlock: model.check_deadlock,
        max_states: args.max_states,
        record_graph: !live.properties.is_empty() || !possible.is_empty(),
        continue_after_violation: args.continue_after_violation,
    };
    if let Some(path) = args.disk {
        if args.workers.get() != 1 {
            return Err("disk exploration currently requires one worker".into());
        }
        if !live.properties.is_empty() {
            return Err("disk liveness graph recording is not implemented".into());
        }
        if args.continue_after_violation {
            return Err("disk exploration does not support --continue".into());
        }
        let store =
            tla_store::Store::open(&path, args.disk_cache_bytes).map_err(|e| e.to_string())?;
        let report = if let Some(timeline) = timeline {
            tla_disk::explore(
                &timeline.measured(&model),
                &tla_disk::StateCodec,
                &store,
                &options,
            )
        } else {
            tla_disk::explore(&model, &tla_disk::StateCodec, &store, &options)
        }
        .map_err(|e| e.to_string())?;
        statistics(
            report.statistics.initial_distinct,
            report.distinct as usize,
            report.statistics.depth,
        );
        return finish(&model, report.completion, false, style);
    }
    let report = if let Some(timeline) = timeline {
        let measured = timeline.measured(&model);
        if args.workers.get() == 1 {
            explore(&measured, &options)
        } else {
            explore_parallel(&measured, &options, args.workers)
        }
    } else if args.workers.get() == 1 {
        explore(&model, &options)
    } else {
        explore_parallel(&model, &options, args.workers)
    }
    .map_err(|e| e.to_string())?;
    statistics(
        report.statistics.initial_distinct,
        report.nodes.len(),
        report.statistics.depth,
    );
    for violation in &report.violations {
        trace::violation(&model, violation, style)?;
    }
    let violated = !report.violations.is_empty();
    if matches!(report.completion, Completion::Complete)
        && !live.properties.is_empty()
        && !stage(timeline, "liveness", || {
            liveness::check(&model, &report, &live, style)
        })?
    {
        return Ok(1);
    }
    if matches!(report.completion, Completion::Complete) && !possible.is_empty() {
        let counts = liveness::possible_counts(&model, &report, &possible)?;
        model.publish_possible(&counts).map_err(|e| e.to_string())?;
    }
    if matches!(report.completion, Completion::Complete)
        && !violated
        && let Some(name) = liveness::unwitnessed(&model, &report, &possible)?
    {
        println!("Possible {name} unwitnessed");
        return Ok(1);
    }
    // TLC evaluates postconditions last; they matter only for a run without violations.
    if matches!(report.completion, Completion::Complete) && !violated && !postconditions.is_empty()
    {
        model
            .publish_statistics([
                ("queue", 0),
                ("distinct", report.nodes.len()),
                ("initial", report.statistics.initial_distinct),
                ("generated", report.statistics.generated),
                ("diameter", report.statistics.depth),
            ])
            .map_err(|e| e.to_string())?;
        for (name, id) in &postconditions {
            let value = model.evaluate(*id, &[], &[]).map_err(|e| e.to_string())?;
            if value != tla_model::Value::Bool(true) {
                println!("Postcondition {name} violated");
                return Ok(1);
            }
        }
    }
    finish(&model, report.completion, violated, style)
}

fn statistics(initial: usize, distinct: usize, depth: usize) {
    println!("{distinct} distinct states found ({initial} initial), search depth {depth}");
}

fn finish(
    model: &BoundModel,
    completion: Completion<tla_model::State>,
    violated: bool,
    style: trace::Style,
) -> Result<i32, String> {
    match completion {
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
            trace::violation(model, &violation, style)?;
            Ok(1)
        }
    }
}

#[cfg(test)]
mod tests;
