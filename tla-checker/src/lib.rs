//! The model checker behind the `tla` command. A specification and its
//! configuration are [`load`]ed, the configured model is [`bind`]ed, and a
//! [`Checker`] explores it and checks liveness, witnesses and postconditions.
//! The result is an [`Outcome`] for a front end to render.

mod actions;
mod liveness;

use std::{
    fmt,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};
use tlaplus_engine::{Options, explore, explore_parallel, validate_trace};
use tlaplus_ir::{ExprId, Level, Program};
use tlaplus_lower::Lowered;
use tlaplus_modules::{Config, temporal::Temporal};
use tlaplus_observe::Timeline;
use tlaplus_temporal::Instantiated;

pub use tlaplus_engine::{Completion, Violation, ViolationKind};
pub use tlaplus_model::{BoundModel, State, Value};

/// Why checking stopped, by the phase that failed.
#[derive(Debug)]
pub enum Error {
    /// A module failed to parse.
    Syntax(String),
    /// A file could not be read.
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    /// Loading, configuring or lowering the specification failed.
    Load(tlaplus_modules::Error),
    /// The configuration names something the specification lacks, or uses it
    /// at the wrong level.
    Configuration(String),
    /// A feature this checker does not implement.
    Unsupported(&'static str),
    /// Evaluating or exploring the model failed.
    Evaluation(tlaplus_model::Error),
    /// A reported counterexample does not replay on the model.
    InvalidCounterexample,
    /// Instantiating or checking a temporal property failed.
    Liveness(tlaplus_temporal::Error),
    /// Disk-backed exploration failed to store or read a state.
    Storage(tlaplus_disk::Error<tlaplus_model::Error>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(e) | Self::Configuration(e) => f.write_str(e),
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Load(e) => e.fmt(f),
            Self::Unsupported(what) => f.write_str(what),
            Self::Evaluation(e) => e.fmt(f),
            Self::InvalidCounterexample => f.write_str("invalid safety counterexample"),
            Self::Liveness(e) => e.fmt(f),
            Self::Storage(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<tlaplus_modules::Error> for Error {
    fn from(e: tlaplus_modules::Error) -> Self {
        Self::Load(e)
    }
}

impl From<tlaplus_model::Error> for Error {
    fn from(e: tlaplus_model::Error) -> Self {
        Self::Evaluation(e)
    }
}

impl From<tlaplus_temporal::Error> for Error {
    fn from(e: tlaplus_temporal::Error) -> Self {
        Self::Liveness(e)
    }
}

impl From<tlaplus_disk::Error<tlaplus_model::Error>> for Error {
    fn from(e: tlaplus_disk::Error<tlaplus_model::Error>) -> Self {
        Self::Storage(e)
    }
}

fn configuration(message: impl Into<String>) -> Error {
    Error::Configuration(message.into())
}

fn stage<T>(timeline: Option<&Timeline>, name: &'static str, operation: impl FnOnce() -> T) -> T {
    match timeline {
        Some(timeline) => timeline.span(name, operation),
        None => operation(),
    }
}

/// Parses a module without loading its imports.
pub fn check_syntax(text: &str) -> Result<(), Error> {
    tlaplus_parser::parse(text)
        .map(drop)
        .map_err(|e| Error::Syntax(e.to_string()))
}

/// Loads and lowers a module with an empty configuration.
pub fn check_module(path: &Path) -> Result<(), Error> {
    let source = tlaplus_modules::load(path, &[])?;
    tlaplus_lower::lower(&source, &Config::default())?;
    Ok(())
}

/// A specification lowered with its configuration, ready to bind.
pub struct Loaded {
    config: Config,
    temporal: Temporal,
    lowered: Lowered,
    /// How many operators the source module defines. Lowering numbers them
    /// first, in source order, so `0..definitions` are the user definitions
    /// whose constant bodies are cached before exploration.
    definitions: usize,
}

/// Loads `spec` and its configuration: `config` when given, otherwise the
/// sibling `.cfg` file, or else the specification's own `CONFIG` section.
pub fn load(
    spec: &Path,
    config: Option<&Path>,
    libraries: &[PathBuf],
    timeline: Option<&Timeline>,
) -> Result<Loaded, Error> {
    let sibling = spec.with_extension("cfg");
    let embedded = !sibling.exists()
        && std::fs::read_to_string(spec)
            .is_ok_and(|text| !tlaplus_modules::embedded_config(&text).trim().is_empty());
    let config_path = match config {
        Some(path) => path.to_path_buf(),
        None if embedded => spec.to_path_buf(),
        None => sibling,
    };
    let mut text = std::fs::read_to_string(&config_path).map_err(|error| Error::Io {
        path: config_path.clone(),
        error,
    })?;
    if config_path.extension().is_some_and(|ext| ext == "tla") {
        text = tlaplus_modules::embedded_config(&text);
    }
    let mut config = tlaplus_modules::config(&text)?;
    let mut source = stage(timeline, "load", || tlaplus_modules::load(spec, libraries))?;
    tlaplus_modules::override_operators(&mut source, &mut config)?;
    let temporal = tlaplus_modules::temporal::prepare(&mut source, &mut config)?;
    let lowered = stage(timeline, "lower", || tlaplus_lower::lower(&source, &config))?;
    if config.specification.is_some() {
        return Err(Error::Unsupported(
            "SPECIFICATION temporal decomposition is not implemented; use INIT/NEXT",
        ));
    }
    Ok(Loaded {
        config,
        temporal,
        lowered,
        definitions: source.definitions.len(),
    })
}

/// How a model is checked.
#[derive(Clone, Debug)]
pub struct Settings {
    pub seed: i64,
    pub workers: NonZeroUsize,
    pub max_states: Option<NonZeroUsize>,
    pub check_deadlock: bool,
    pub continue_after_violation: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seed: 1,
            workers: NonZeroUsize::MIN,
            max_states: None,
            check_deadlock: true,
            continue_after_violation: false,
        }
    }
}

/// Where exploration keeps its state graph when it does not fit in memory.
#[derive(Clone, Debug)]
pub struct Disk {
    pub path: PathBuf,
    pub cache_bytes: NonZeroUsize,
}

/// The result of binding a configuration to a lowered specification.
pub enum Bound {
    /// An `ASSUME` does not hold; nothing is explored.
    AssumptionViolated,
    /// Like TLC, a model without variables or behavior only checks its assumptions.
    Trivial,
    Ready(Box<Checker>),
}

/// A bound model and the checks configured for it.
pub struct Checker {
    pub model: BoundModel,
    live: Instantiated,
    possible: Vec<(String, ExprId)>,
    postconditions: Vec<(String, ExprId)>,
    settings: Settings,
}

fn resolve(
    program: &Program,
    failures: &std::collections::BTreeMap<String, String>,
    name: &str,
) -> Result<ExprId, Error> {
    if let Some(error) = failures.get(name) {
        return Err(configuration(format!("{name}: {error}")));
    }
    let op = program
        .operators
        .iter()
        .find(|op| op.name == name)
        .ok_or_else(|| configuration(format!("unknown configured operator {name}")))?;
    if !op.parameters.is_empty() {
        return Err(configuration(format!(
            "configured operator {name} requires arguments"
        )));
    }
    Ok(op.body)
}

/// A configured expression may not exceed `max`: a state predicate cannot
/// mention primed variables, an action cannot be temporal.
fn require_level(model: &BoundModel, id: ExprId, max: Level, what: &str) -> Result<(), Error> {
    if model.program.nodes[id].level > max {
        let level = match max {
            Level::Constant => "constant",
            Level::State => "state",
            Level::Action => "action",
            Level::Temporal => "temporal",
        };
        return Err(configuration(format!("{what} must have {level} level")));
    }
    Ok(())
}

/// `SYMMETRY`: a constant set of permutations of model values. Like TLC, the
/// configured permutations generate a group, and every member of that group
/// is applied when states are canonicalized, so `Permutations(A) \cup
/// Permutations(B)` also covers permuting both at once.
fn configure_symmetry(model: &mut BoundModel, id: ExprId) -> Result<(), Error> {
    require_level(model, id, Level::Constant, "SYMMETRY")?;
    let permutations = model.evaluate(id, &[], &[])?;
    let mut generators = Vec::new();
    for permutation in permutations.elements().map_err(tlaplus_model::Error::from)? {
        let entries = permutation
            .entries()
            .map_err(tlaplus_model::Error::from)?
            .to_vec();
        let domain = entries.iter().map(|(key, _)| key).collect::<Vec<_>>();
        if domain.iter().any(|key| !matches!(key, Value::Model(_)))
            || !domain
                .iter()
                .all(|key| entries.iter().any(|(_, value)| value == *key))
        {
            return Err(configuration(
                "SYMMETRY requires permutations of model values",
            ));
        }
        generators.push(entries);
    }
    model.checks.symmetry = symmetry_group(generators);
    Ok(())
}

/// The group the permutations generate, without the identity, each member
/// written over the union of their domains.
fn symmetry_group(generators: Vec<Vec<(Value, Value)>>) -> Vec<Vec<(Value, Value)>> {
    let mut domain: Vec<Value> = Vec::new();
    for (key, _) in generators.iter().flatten() {
        if !domain.contains(key) {
            domain.push(key.clone());
        }
    }
    // A permutation as the image of each domain element, in domain order.
    let image = |permutation: &[(Value, Value)]| -> Vec<Value> {
        domain
            .iter()
            .map(|key| {
                permutation
                    .iter()
                    .find(|(k, _)| k == key)
                    .map_or_else(|| key.clone(), |(_, v)| v.clone())
            })
            .collect()
    };
    let identity: Vec<Value> = domain.clone();
    let generators: Vec<Vec<Value>> = generators.iter().map(|p| image(p)).collect();
    let mut members: Vec<Vec<Value>> = vec![identity.clone()];
    let mut frontier = vec![identity];
    while let Some(current) = frontier.pop() {
        for generator in &generators {
            // Apply `current`, then `generator`.
            let composed: Vec<Value> = current
                .iter()
                .map(|value| {
                    // Every image is a key of its generator, which
                    // `configure_symmetry` checked, so it is in the domain.
                    let index = domain
                        .iter()
                        .position(|d| d == value)
                        .expect("permutation images lie in the domain");
                    generator[index].clone()
                })
                .collect();
            if !members.contains(&composed) {
                members.push(composed.clone());
                frontier.push(composed);
            }
        }
    }
    members
        .into_iter()
        .filter(|member| *member != domain)
        .map(|member| domain.iter().cloned().zip(member).collect())
        .collect()
}

/// A `PROPERTY` conjunct TLC checks on initial states must be a state
/// predicate. `temporal::prepare` already split off the action and temporal
/// conjuncts; one still here is unsupported.
fn classify_properties(
    model: &mut BoundModel,
    properties: Vec<(String, ExprId)>,
) -> Result<(), Error> {
    for (name, id) in properties {
        match model.program.nodes[id].level {
            Level::Constant | Level::State => model.checks.initial_properties.push((name, id)),
            Level::Action => {
                return Err(configuration(format!(
                    "PROPERTY {name} has action level; use [][Action]_vars"
                )));
            }
            Level::Temporal => {
                return Err(configuration(format!(
                    "PROPERTY {name} is not a supported formula"
                )));
            }
        }
    }
    Ok(())
}

/// Binds constants, resolves the configured operators and checks their levels.
pub fn bind(loaded: Loaded, settings: Settings) -> Result<Bound, Error> {
    let Loaded {
        config,
        temporal,
        lowered,
        definitions,
    } = loaded;
    let mut model = BoundModel::new(lowered.program);
    model.seed(settings.seed);
    model.configure_constants(&lowered.bindings, &lowered.overrides)?;
    model.cache_constants(0..definitions)?;
    if model
        .setup_assumptions(&lowered.assumptions, settings.workers)?
        .is_some()
    {
        return Ok(Bound::AssumptionViolated);
    }
    let failures = lowered.failures;
    let resolve = |name: &str| resolve(&model.program, &failures, name);
    let named = |names: &[(String, String)]| {
        names
            .iter()
            .map(|(name, predicate)| Ok((name.clone(), resolve(predicate)?)))
            .collect::<Result<Vec<_>, Error>>()
    };
    let listed = |names: &[String]| {
        names
            .iter()
            .map(|name| Ok((name.clone(), resolve(name)?)))
            .collect::<Result<Vec<_>, Error>>()
    };
    if config.init.is_none() && config.next.is_none() && model.program.variables.is_empty() {
        return Ok(Bound::Trivial);
    }
    let init = resolve(
        config
            .init
            .as_deref()
            .ok_or_else(|| configuration("configuration requires INIT"))?,
    )?;
    require_level(&model, init, Level::State, "INIT")?;
    let next = config
        .next
        .as_deref()
        .ok_or_else(|| configuration("configuration requires NEXT"))?;
    let action = resolve(next)?;
    require_level(&model, action, Level::Action, "NEXT")?;
    let mut invariants = listed(&config.invariants)?;
    invariants.extend(named(&temporal.invariants)?);
    let initial_properties = named(&temporal.initial_properties)?;
    let action_properties = named(&temporal.action_properties)?;
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
    let postconditions = listed(&config.postconditions)?;
    let possible = listed(&config.possible)?;
    let symmetry = config.symmetry.as_deref().map(resolve).transpose()?;
    if let Some(id) = symmetry {
        configure_symmetry(&mut model, id)?;
    }
    for (_, id) in &invariants {
        require_level(&model, *id, Level::State, "INVARIANT")?;
    }
    // Like TLC, a constant invariant is checked once: dropped if TRUE, fatal if not.
    for (name, id) in &invariants {
        if model.program.nodes[*id].level == Level::Constant
            && model.evaluate(*id, &[], &[])? != Value::Bool(true)
        {
            return Err(configuration(format!(
                "The invariant of {name} is equal to FALSE"
            )));
        }
    }
    invariants.retain(|(_, id)| model.program.nodes[*id].level != Level::Constant);
    for id in &state_constraints {
        require_level(&model, *id, Level::State, "CONSTRAINT")?;
    }
    if let Some(id) = view {
        require_level(&model, id, Level::State, "VIEW")?;
    }
    model.checks.init = vec![init];
    model.checks.actions = actions::split(&mut model.program, next, action);
    model.checks.invariants = invariants;
    model.checks.action_properties = action_properties;
    model.checks.state_constraints = state_constraints;
    model.checks.action_constraints = action_constraints;
    model.checks.view = view;
    classify_properties(&mut model, initial_properties)?;
    let live = tlaplus_temporal::instantiate(&mut model, &temporal)?;
    model.checks.check_deadlock = settings.check_deadlock && config.check_deadlock.unwrap_or(true);
    Ok(Bound::Ready(Box::new(Checker {
        model,
        live,
        possible,
        postconditions,
        settings,
    })))
}

/// How much of the state graph exploration covered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Statistics {
    pub initial: usize,
    pub distinct: usize,
    pub depth: usize,
}

/// A temporal property's verdict.
pub enum Liveness {
    Holds,
    /// A counterexample: the states of a lasso, and the index of the state the
    /// last one steps back to, with whether that cycle is a single stuttering state.
    Violated {
        steps: Vec<State>,
        loop_back: (usize, bool),
    },
}

/// What checking found, in the order a front end reports it. Liveness,
/// witnesses and postconditions are only checked after complete exploration,
/// and each stops at its first failure.
pub struct Outcome {
    /// Retained only by run_with_graph; edges are observed transitions, not a tree.
    pub graph: Option<ExplorationGraph>,
    pub statistics: Statistics,
    pub violations: Vec<Violation<State>>,
    pub liveness: Vec<(String, Liveness)>,
    /// A `_POSSIBLE` predicate that no explored state or step satisfies.
    pub unwitnessed: Option<String>,
    pub postcondition_violated: Option<String>,
    pub completion: Completion<State>,
}

/// The explored prefix, including which states were fully expanded.
pub struct ExplorationGraph {
    pub nodes: Vec<tlaplus_engine::Node<State>>,
    pub edges: Vec<tlaplus_engine::Edge>,
}

impl Checker {
    /// Checks in memory while retaining the observed state graph for inspection.
    pub fn run_with_graph(&mut self) -> Result<Outcome, Error> {
        self.run_internal(None, None, true)
    }

    /// Explores the model, in memory or on `disk`, and runs the configured checks.
    pub fn run(
        &mut self,
        disk: Option<&Disk>,
        timeline: Option<&Timeline>,
    ) -> Result<Outcome, Error> {
        self.run_internal(disk, timeline, false)
    }

    fn run_internal(
        &mut self,
        disk: Option<&Disk>,
        timeline: Option<&Timeline>,
        retain_graph: bool,
    ) -> Result<Outcome, Error> {
        let options = Options {
            check_deadlock: self.model.checks.check_deadlock,
            max_states: self.settings.max_states,
            record_graph: retain_graph
                || !self.live.properties.is_empty()
                || !self.possible.is_empty(),
            continue_after_violation: self.settings.continue_after_violation,
        };
        if let Some(disk) = disk {
            return self.run_on_disk(disk, &options, timeline);
        }
        let workers = self.settings.workers;
        let report = match timeline {
            Some(timeline) => explore_with(&timeline.measured(&self.model), &options, workers),
            None => explore_with(&self.model, &options, workers),
        }?;
        let statistics = Statistics {
            initial: report.statistics.initial_distinct,
            distinct: report.nodes.len(),
            depth: report.statistics.depth,
        };
        for violation in &report.violations {
            self.validate(violation)?;
        }
        if let Completion::Violated(violation) = &report.completion {
            self.validate(violation)?;
        }
        let violated = !report.violations.is_empty();
        let complete = matches!(report.completion, Completion::Complete);
        let mut liveness = Vec::new();
        let mut unwitnessed = None;
        let mut postcondition_violated = None;
        'checks: {
            if complete && !self.live.properties.is_empty() {
                liveness = stage(timeline, "liveness", || {
                    liveness::check(&self.model, &report, &self.live)
                })?;
                if liveness
                    .iter()
                    .any(|(_, verdict)| matches!(verdict, Liveness::Violated { .. }))
                {
                    break 'checks;
                }
            }
            if complete && !self.possible.is_empty() {
                let counts = liveness::possible_counts(&self.model, &report, &self.possible)?;
                self.model.publish_possible(&counts)?;
            }
            if !complete || violated {
                break 'checks;
            }
            if let Some(name) = liveness::unwitnessed(&self.model, &report, &self.possible)? {
                unwitnessed = Some(name);
                break 'checks;
            }
            // TLC evaluates postconditions last; they matter only for a run without violations.
            if !self.postconditions.is_empty() {
                self.model.publish_statistics([
                    ("queue", 0),
                    ("distinct", report.nodes.len()),
                    ("initial", report.statistics.initial_distinct),
                    ("generated", report.statistics.generated),
                    ("diameter", report.statistics.depth),
                ])?;
                for (name, id) in &self.postconditions {
                    let value = self.model.evaluate(*id, &[], &[])?;
                    if value != Value::Bool(true) {
                        postcondition_violated = Some(name.clone());
                        break 'checks;
                    }
                }
            }
        }
        Ok(Outcome {
            graph: retain_graph.then_some(ExplorationGraph {
                nodes: report.nodes,
                edges: report.edges,
            }),
            statistics,
            violations: report.violations,
            liveness,
            unwitnessed,
            postcondition_violated,
            completion: report.completion,
        })
    }

    fn run_on_disk(
        &mut self,
        disk: &Disk,
        options: &Options,
        timeline: Option<&Timeline>,
    ) -> Result<Outcome, Error> {
        if self.settings.workers.get() != 1 {
            return Err(Error::Unsupported(
                "disk exploration currently requires one worker",
            ));
        }
        if !self.live.properties.is_empty() {
            return Err(Error::Unsupported(
                "disk liveness graph recording is not implemented",
            ));
        }
        if self.settings.continue_after_violation {
            return Err(Error::Unsupported(
                "disk exploration does not support --continue",
            ));
        }
        let store = tlaplus_disk::Store::open(&disk.path, disk.cache_bytes)
            .map_err(tlaplus_disk::Error::<tlaplus_model::Error>::from)?;
        let report = if let Some(timeline) = timeline {
            tlaplus_disk::explore(
                &timeline.measured(&self.model),
                &tlaplus_disk::StateCodec,
                &store,
                options,
            )
        } else {
            tlaplus_disk::explore(&self.model, &tlaplus_disk::StateCodec, &store, options)
        }?;
        if let Completion::Violated(violation) = &report.completion {
            self.validate(violation)?;
        }
        Ok(Outcome {
            graph: None,
            statistics: Statistics {
                initial: report.statistics.initial_distinct,
                distinct: report.distinct as usize,
                depth: report.statistics.depth,
            },
            violations: Vec::new(),
            liveness: Vec::new(),
            unwitnessed: None,
            postcondition_violated: None,
            completion: report.completion,
        })
    }

    fn validate(&self, violation: &Violation<State>) -> Result<(), Error> {
        if !validate_trace(&self.model, violation)? {
            return Err(Error::InvalidCounterexample);
        }
        Ok(())
    }
}

/// Serial or parallel exploration, by worker count.
fn explore_with<T>(
    system: &T,
    options: &Options,
    workers: NonZeroUsize,
) -> Result<tlaplus_engine::Report<T::State>, T::Error>
where
    T: tlaplus_engine::TransitionSystem + Sync,
    T::State: Send + Sync,
    T::Error: Send,
    T::Key: Send + Sync,
{
    if workers.get() == 1 {
        explore(system, options)
    } else {
        explore_parallel(system, options, workers)
    }
}
