//! A configured model: a lowered program bound to constant values, the
//! [`Checks`] a configuration asks for and the [`Runtime`] state of a run.
//! [`BoundModel`] implements the engine's transition system by generating
//! initial states and successors.
mod constants;
mod generate;
pub use constants::materialize as materialize_constants;
use tlaplus_engine::{Transition, TransitionSystem};
use tlaplus_eval::{BooleanContext, Evaluator};
pub use tlaplus_eval::{Error, Value, literal};
use tlaplus_ir::{ExprId, Program};

pub type State = tlaplus_engine::Leveled<Vec<Value>>;

/// A lowered program with its constants bound, ready to generate states.
/// `program` and `constants` are fixed after binding; `checks` says what to
/// explore and verify; `runtime` is the mutable state of one run.
#[derive(Clone, Debug)]
pub struct BoundModel {
    pub program: Program,
    pub constants: Vec<Value>,
    pub runtime: Runtime,
    pub checks: Checks,
}

/// Mutable state the evaluator keeps across a run: TLC registers and caches,
/// random streams, the string table and the clock.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub caches: tlaplus_eval::Caches,
    pub registers: tlaplus_eval::Registers,
    pub randomized: bool,
    pub random: tlaplus_eval::RandomSessions,
    pub started: std::time::Instant,
    pub strings: tlaplus_eval::Strings,
}

/// What a configuration asks the checker to explore and verify, as
/// expressions of the model's program.
#[derive(Clone, Debug)]
pub struct Checks {
    pub init: Vec<ExprId>,
    /// The next-state relation split into named actions.
    pub actions: Vec<(String, ExprId)>,
    pub invariants: Vec<(String, ExprId)>,
    pub initial_properties: Vec<(String, ExprId)>,
    pub action_properties: Vec<(String, ExprId)>,
    pub temporal_properties: Vec<(String, ExprId)>,
    pub temporal_assumptions: Vec<(String, ExprId)>,
    pub state_constraints: Vec<ExprId>,
    pub action_constraints: Vec<ExprId>,
    pub view: Option<ExprId>,
    pub symmetry: Vec<Vec<(Value, Value)>>,
    pub check_deadlock: bool,
}

impl Default for Checks {
    fn default() -> Self {
        Self {
            init: vec![],
            actions: vec![],
            invariants: vec![],
            initial_properties: vec![],
            action_properties: vec![],
            temporal_properties: vec![],
            temporal_assumptions: vec![],
            state_constraints: vec![],
            action_constraints: vec![],
            view: None,
            symmetry: vec![],
            check_deadlock: true,
        }
    }
}
impl BoundModel {
    pub(crate) fn boolean(
        &self,
        id: ExprId,
        value: Value,
        context: BooleanContext,
    ) -> Result<bool, Error> {
        Ok(tlaplus_eval::boolean(&value, context, self.program.source(id))?)
    }
    pub(crate) fn evaluator<'a>(
        &'a self,
        current: &'a [Option<Value>],
        next: &'a [Option<Value>],
        level: usize,
    ) -> Evaluator<'a> {
        let mut evaluator = Evaluator::with_shared_storage(
            &self.program,
            &self.constants,
            current,
            next,
            self.runtime.strings.clone(),
            self.runtime.caches.active.clone(),
        );
        evaluator.registers = Some(&self.runtime.registers);
        evaluator.worker = rayon::current_thread_index();
        evaluator.level = level;
        evaluator.started = self.runtime.started;
        // Worker threads share the string table; only the main thread may
        // assign new ordinals, so workers see a consistent numbering.
        evaluator.intern_new_strings = evaluator.worker.is_none();
        evaluator
    }
    pub fn setup_assumptions(
        &mut self,
        assumptions: &[ExprId],
        workers: std::num::NonZeroUsize,
    ) -> Result<Option<usize>, Error> {
        self.runtime
            .registers
            .begin_setup(workers)
            .map_err(Error::from)?;
        self.runtime.random.begin_setup();
        self.runtime.caches.begin_setup().map_err(Error::from)?;
        for (index, expression) in assumptions.iter().enumerate() {
            if !self.evaluate(*expression, &[], &[])?.boolean()? {
                return Ok(Some(index));
            }
        }
        self.runtime
            .registers
            .capture_setup()
            .map_err(Error::from)?;
        self.runtime.caches.capture_setup().map_err(Error::from)?;
        self.runtime.random.capture_setup()?;
        Ok(None)
    }
    pub(crate) fn complete_values(&self, values: Vec<Option<Value>>) -> Result<Vec<Value>, Error> {
        if values.len() != self.program.variables.len() {
            return Err(Error("state arity mismatch".into()));
        }
        let missing: Vec<_> = self
            .program
            .variables
            .iter()
            .zip(&values)
            .filter_map(|(name, value)| value.is_none().then_some(name.as_str()))
            .collect();
        if !missing.is_empty() {
            return Err(Error(format!(
                "unassigned variables: {}",
                missing.join(", ")
            )));
        }
        Ok(values.into_iter().flatten().collect())
    }
    pub fn new(program: Program) -> Self {
        Self {
            runtime: Runtime {
                caches: tlaplus_eval::Caches::default(),
                registers: tlaplus_eval::Registers::default(),
                randomized: program.uses_random(0..program.nodes.len()),
                random: tlaplus_eval::RandomSessions::new(1, tlaplus_eval::Random::new(1)),
                started: std::time::Instant::now(),
                strings: std::sync::Arc::new(std::sync::Mutex::new(program.strings.clone())),
            },
            program,
            constants: vec![],
            checks: Checks::default(),
        }
    }
    pub fn configure_constants(
        &mut self,
        bindings: &[(usize, ExprId)],
        overrides: &[(usize, ExprId)],
    ) -> Result<(), Error> {
        let random = self.runtime.random.current()?;
        let values = constants::configure(
            &mut self.program,
            bindings,
            overrides,
            |program, values, expression| {
                let mut evaluator = Evaluator::with_shared_storage(
                    program,
                    values,
                    &[],
                    &[],
                    self.runtime.strings.clone(),
                    self.runtime.caches.active.clone(),
                );
                evaluator.random = Some(&random);
                evaluator.started = self.runtime.started;
                evaluator.level = 0;
                evaluator.evaluate(expression)
            },
        )?;
        self.set_constants(values)?;
        self.runtime.random = self.runtime.random.checkpoint()?;
        self.runtime.caches = self.runtime.caches.checkpoint().map_err(Error::from)?;
        Ok(())
    }
    pub fn cache_constants(&mut self, operators: std::ops::Range<usize>) -> Result<(), Error> {
        use tlaplus_ir::{Expr, Level};
        for index in operators {
            let operator = self
                .program
                .operators
                .get(index)
                .ok_or_else(|| Error("invalid constant operator".into()))?;
            let body = operator.body;
            let node = self
                .program
                .nodes
                .get(body)
                .ok_or_else(|| Error("invalid constant body".into()))?;
            // TLC cannot evaluate checker registers or the level before checking starts.
            let runtime = [tlaplus_ir::Builtin::TlcGet, tlaplus_ir::Builtin::TlcGetOrDefault];
            if !operator.parameters.is_empty()
                || node.level != Level::Constant
                || self.program.uses_builtins([body], &runtime)
            {
                continue;
            }
            // A constant body that fails now is left as it is; the error
            // surfaces where the definition is used, as TLC reports it.
            if let Ok(value) = self.evaluate(body, &[], &[]) {
                let body = self
                    .program
                    .push(Expr::Literal(literal(&value)), Level::Constant);
                self.program.operators[index].body = body;
            }
        }
        self.runtime.random = self.runtime.random.checkpoint()?;
        self.runtime.caches = self.runtime.caches.checkpoint().map_err(Error::from)?;
        Ok(())
    }
    pub fn set_constants(&mut self, constants: Vec<Value>) -> Result<(), Error> {
        let mut strings = self
            .runtime
            .strings
            .lock()
            .map_err(|_| Error("string intern table poisoned".into()))?;
        let mut pending: Vec<_> = constants.iter().collect();
        while let Some(value) = pending.pop() {
            match value {
                Value::String(atom) | Value::Model(atom) => {
                    strings.insert(atom.text.to_string(), atom.ordinal);
                }
                Value::Set(values) => pending.extend(values.iter()),
                Value::Function(entries) => {
                    for (key, value) in entries.iter() {
                        pending.extend([key, value]);
                    }
                }
                _ => {}
            }
        }
        self.constants = constants;
        Ok(())
    }
    /// Makes a finished run's statistics available to `TLCGet`, as TLC does for
    /// `POSTCONDITION`: `[queue, distinct, initial, generated, diameter, duration, worker]`.
    pub fn publish_statistics(&self, counts: [(&str, usize); 5]) -> Result<(), Error> {
        let int = |n: usize| Value::Int(i32::try_from(n).unwrap_or(i32::MAX));
        let mut fields = Vec::new();
        let mut published = std::collections::BTreeMap::new();
        for (name, count) in counts.into_iter().chain([("duration", 0), ("worker", 0)]) {
            let key = tlaplus_eval::intern_string(&self.runtime.strings, name.into(), true)?;
            fields.push((key, int(count)));
            published.insert(name.to_owned(), int(count));
        }
        published.insert("stats".into(), Value::function(fields)?);
        self.runtime
            .registers
            .publish(published)
            .map_err(Error::from)
    }
    /// Publishes `_POSSIBLE` counts as TLC's `_Possible` module reads them:
    /// `TLCGet("all:named")["s:_possible"]`, one record per worker.
    pub fn publish_possible(&self, counts: &[(String, usize)]) -> Result<(), Error> {
        let string = |text: &str| tlaplus_eval::intern_string(&self.runtime.strings, text.into(), true);
        let fields = counts
            .iter()
            .map(|(name, n)| {
                Ok((
                    string(name)?,
                    Value::Int(i32::try_from(*n).unwrap_or(i32::MAX)),
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let workers = Value::tuple(vec![Value::function(fields)?]);
        let registers = Value::function(vec![(string("s:_possible")?, workers)])?;
        let published = std::collections::BTreeMap::from([("all:named".to_owned(), registers)]);
        self.runtime
            .registers
            .publish(published)
            .map_err(Error::from)
    }
    /// Seeds random operators as TLC's `-seed` does before any evaluation.
    pub fn seed(&mut self, seed: i64) {
        self.runtime.random = tlaplus_eval::RandomSessions::new(seed, tlaplus_eval::Random::new(seed));
    }
    pub fn set_random_seed(&mut self, seed: i64, state: u64) -> Result<(), Error> {
        self.runtime.random =
            tlaplus_eval::RandomSessions::new(seed, tlaplus_eval::Random::from_seed_state(state)?);
        Ok(())
    }
    fn begin_successors(&self, state: &State) -> Result<(), Error> {
        if !self.runtime.randomized {
            return Ok(());
        }
        let polynomial = self
            .program
            .fingerprint_polynomial
            .unwrap_or(tlaplus_ir::DEFAULT_FINGERPRINT_POLYNOMIAL);
        let fingerprint = self.key(state)?.iter().fold(polynomial, |fp, value| {
            value.extend_fingerprint(fp, polynomial)
        });
        Ok(self.runtime.random.begin(Some(fingerprint))?)
    }
    /// Evaluates `id` in `state` (and `next` for action-level expressions).
    /// Constant expressions pass an empty `state`, which is depth 0; any real
    /// state is treated as depth 1, enough for `TLCGet("level")` style queries.
    pub fn evaluate(&self, id: ExprId, state: &[Value], next: &[Value]) -> Result<Value, Error> {
        self.evaluate_at(id, state, next, usize::from(!state.is_empty()))
    }
    pub fn evaluate_at(
        &self,
        id: ExprId,
        state: &[Value],
        next: &[Value],
        depth: usize,
    ) -> Result<Value, Error> {
        let state: Vec<_> = state.iter().cloned().map(Some).collect();
        let next: Vec<_> = next.iter().cloned().map(Some).collect();
        generate::evaluate(self, id, &state, &next, &[], depth)
    }
    fn failure(
        &self,
        properties: &[(String, ExprId)],
        state: &State,
        next: &[Value],
    ) -> Result<Option<String>, Error> {
        for (name, id) in properties {
            let value = self.evaluate_at(*id, state, next, state.depth)?;
            if !self.boolean(*id, value, BooleanContext::Predicate)? {
                return Ok(Some(name.clone()));
            }
        }
        Ok(None)
    }
    fn allowed(&self, predicates: &[ExprId], state: &State, next: &[Value]) -> Result<bool, Error> {
        for id in predicates {
            let value = self.evaluate_at(*id, state, next, state.depth)?;
            if !self.boolean(*id, value, BooleanContext::Predicate)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn uses_registers(&self) -> bool {
        self.program.uses_builtins(
            self.checks
                .init
                .iter()
                .copied()
                .chain(
                    self.checks
                        .actions
                        .iter()
                        .chain(&self.checks.invariants)
                        .chain(&self.checks.initial_properties)
                        .chain(&self.checks.action_properties)
                        .map(|(_, id)| *id),
                )
                .chain(self.checks.state_constraints.iter().copied())
                .chain(self.checks.action_constraints.iter().copied())
                .chain(self.checks.view),
            &[tlaplus_ir::Builtin::TlcSet],
        )
    }
}
impl BoundModel {
    /// Resets run state for the initial predicate. `false` means the model has
    /// no behavior to generate: no variables and no `INIT`.
    fn begin_initial(&self) -> Result<bool, Error> {
        self.runtime.registers.reset()?;
        self.runtime.caches.reset()?;
        self.runtime.random.begin(None)?;
        Ok(!(self.checks.init.is_empty() && self.program.variables.is_empty()))
    }
}

impl TransitionSystem for BoundModel {
    type State = State;
    type Key = Vec<Value>;
    type Error = Error;
    fn prepare_parallel(&self, workers: std::num::NonZeroUsize) -> Result<(), Error> {
        self.runtime.registers.configure(workers);
        Ok(())
    }
    fn parallel_callbacks(&self) -> bool {
        self.uses_registers()
    }
    fn prefetch_successors(&self) -> bool {
        !self.uses_registers()
            && !self.program.uses_random(
                self.checks
                    .invariants
                    .iter()
                    .chain(&self.checks.initial_properties)
                    .chain(&self.checks.action_properties)
                    .map(|(_, id)| *id)
                    .chain(self.checks.state_constraints.iter().copied())
                    .chain(self.checks.action_constraints.iter().copied())
                    .chain(self.checks.view),
            )
    }
    fn initial_states(&self) -> Result<Vec<State>, Error> {
        if !self.begin_initial()? {
            return Ok(Vec::new());
        }
        states(self, &self.checks.init, &Vec::new().into(), true)
    }
    fn successors(&self, state: &State) -> Result<Vec<Transition<State>>, Error> {
        self.begin_successors(state)?;
        let mut transitions = Vec::new();
        for (action, (_, predicate)) in self.checks.actions.iter().enumerate() {
            transitions.extend(
                states(self, &[*predicate], state, false)?
                    .into_iter()
                    .map(|state| Transition { action, state }),
            );
        }
        Ok(transitions)
    }
    fn visit_initial(
        &self,
        output: &mut impl FnMut(State) -> Result<std::ops::ControlFlow<()>, Error>,
    ) -> Result<std::ops::ControlFlow<()>, Error> {
        if !self.begin_initial()? {
            return Ok(std::ops::ControlFlow::Continue(()));
        }
        generate::visit(self, &self.checks.init, &Vec::new().into(), true, output)
    }
    fn visit_successors(
        &self,
        state: &State,
        output: &mut impl FnMut(Transition<State>) -> Result<std::ops::ControlFlow<()>, Error>,
    ) -> Result<std::ops::ControlFlow<()>, Error> {
        self.begin_successors(state)?;
        for (action, (_, predicate)) in self.checks.actions.iter().enumerate() {
            if generate::visit(self, &[*predicate], state, false, &mut |state| {
                output(Transition { action, state })
            })?
            .is_break()
            {
                return Ok(std::ops::ControlFlow::Break(()));
            }
        }
        Ok(std::ops::ControlFlow::Continue(()))
    }
    fn key(&self, state: &State) -> Result<Vec<Value>, Error> {
        let mut canonical = state.values.clone();
        for permutation in &self.checks.symmetry {
            let candidate = state
                .iter()
                .map(|v| permute(v, permutation))
                .collect::<Result<Vec<_>, _>>()?;
            let order = candidate
                .iter()
                .zip(&canonical)
                .find_map(|(a, b)| match a.compare(b) {
                    Ok(std::cmp::Ordering::Equal) => None,
                    result => Some(result),
                })
                .transpose()?;
            if order == Some(std::cmp::Ordering::Less) {
                canonical = candidate;
            }
        }
        match self.checks.view {
            Some(view) => Ok(vec![self.evaluate_at(
                view,
                &canonical,
                &[],
                state.depth,
            )?]),
            None => Ok(canonical),
        }
    }
    fn same_state(&self, a: &State, b: &State) -> Result<bool, Error> {
        Ok(a == b && a.depth == b.depth)
    }
    fn state_allowed(&self, state: &State) -> Result<bool, Error> {
        self.allowed(&self.checks.state_constraints, state, &[])
    }
    fn step_allowed(&self, state: &State, next: &State) -> Result<bool, Error> {
        self.allowed(&self.checks.action_constraints, state, next)
    }
    fn invariant(&self, state: &State) -> Result<Option<String>, Error> {
        self.failure(&self.checks.invariants, state, &[])
    }
    fn initial_property(&self, state: &State) -> Result<Option<String>, Error> {
        self.failure(&self.checks.initial_properties, state, &[])
    }
    fn action_property(&self, state: &State, next: &State) -> Result<Option<String>, Error> {
        self.failure(&self.checks.action_properties, state, next)
    }
}
fn permute(value: &Value, permutation: &[(Value, Value)]) -> Result<Value, Error> {
    if let Some((_, replacement)) = permutation.iter().find(|(key, _)| key == value) {
        return Ok(replacement.clone());
    }
    Ok(match value {
        Value::Set(values) => Value::set(
            values
                .iter()
                .map(|v| permute(v, permutation))
                .collect::<Result<_, _>>()?,
        )?,
        Value::Function(entries) => Value::function(
            entries
                .iter()
                .map(|(k, v)| Ok((permute(k, permutation)?, permute(v, permutation)?)))
                .collect::<Result<_, Error>>()?,
        )?,
        _ => value.clone(),
    })
}

fn states(
    model: &BoundModel,
    predicates: &[ExprId],
    state: &State,
    initial: bool,
) -> Result<Vec<State>, Error> {
    let mut states = Vec::new();
    let _ = generate::visit(model, predicates, state, initial, &mut |state| {
        states.push(state);
        Ok(std::ops::ControlFlow::Continue(()))
    })?;
    Ok(states)
}
