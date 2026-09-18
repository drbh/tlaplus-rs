mod generate;
pub use tla_constants::materialize as materialize_constants;
use tla_engine::{Transition, TransitionSystem};
use tla_eval::{BooleanContext, Evaluator};
pub use tla_eval::{Error, Value, literal};
use tla_ir::{ExprId, Program};

pub type State = tla_engine::Leveled<Vec<Value>>;
#[derive(Clone, Debug)]
pub struct BoundModel {
    pub caches: tla_eval::Caches,
    pub registers: tla_eval::Registers,
    pub randomized: bool,
    pub random: tla_eval::RandomSessions,
    pub started: std::time::Instant,
    pub strings: tla_eval::Strings,
    pub program: Program,
    pub constants: Vec<Value>,
    pub init: Vec<ExprId>,
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
impl BoundModel {
    pub(crate) fn boolean(
        &self,
        id: ExprId,
        value: Value,
        context: BooleanContext,
    ) -> Result<bool, Error> {
        Ok(tla_eval::boolean(&value, context, self.program.source(id))?)
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
            self.strings.clone(),
            self.caches.active.clone(),
        );
        evaluator.registers = Some(&self.registers);
        evaluator.worker = rayon::current_thread_index();
        evaluator.level = level;
        evaluator.started = self.started;
        evaluator.intern_new_strings = evaluator.worker.is_none();
        evaluator
    }
    pub fn setup_assumptions(
        &mut self,
        assumptions: &[ExprId],
        workers: std::num::NonZeroUsize,
    ) -> Result<Option<usize>, Error> {
        self.registers.begin_setup(workers).map_err(Error)?;
        self.random.begin_setup();
        self.caches.begin_setup().map_err(Error)?;
        for (index, expression) in assumptions.iter().enumerate() {
            if !self.evaluate(*expression, &[], &[])?.boolean()? {
                return Ok(Some(index));
            }
        }
        self.registers.capture_setup().map_err(Error)?;
        self.caches.capture_setup().map_err(Error)?;
        self.random.capture_setup()?;
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
            caches: tla_eval::Caches::default(),
            registers: tla_eval::Registers::default(),
            randomized: program.uses_random(0..program.nodes.len()),
            random: tla_eval::RandomSessions::new(1, tla_eval::Random::new(1)),
            started: std::time::Instant::now(),
            strings: std::sync::Arc::new(std::sync::Mutex::new(program.strings.clone())),
            program,
            constants: vec![],
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
    pub fn bind_constants(&mut self, bindings: &[(usize, ExprId)]) -> Result<(), Error> {
        self.configure_constants(bindings, &[])
    }
    pub fn configure_constants(
        &mut self,
        bindings: &[(usize, ExprId)],
        overrides: &[(usize, ExprId)],
    ) -> Result<(), Error> {
        let random = self.random.current()?;
        let values = tla_constants::configure(
            &mut self.program,
            bindings,
            overrides,
            |program, values, expression| {
                let mut evaluator = Evaluator::with_shared_storage(
                    program,
                    values,
                    &[],
                    &[],
                    self.strings.clone(),
                    self.caches.active.clone(),
                );
                evaluator.random = Some(&random);
                evaluator.started = self.started;
                evaluator.level = 0;
                evaluator.evaluate(expression)
            },
        )?;
        self.set_constants(values)?;
        self.random = self.random.checkpoint()?;
        self.caches = self.caches.checkpoint().map_err(Error)?;
        Ok(())
    }
    pub fn cache_constants(&mut self, operators: std::ops::Range<usize>) -> Result<(), Error> {
        use tla_ir::{Expr, Level};
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
            let runtime = [tla_ir::Builtin::TlcGet, tla_ir::Builtin::TlcGetOrDefault];
            if !operator.parameters.is_empty()
                || node.level != Level::Constant
                || self.program.uses_builtins([body], &runtime)
            {
                continue;
            }
            if let Ok(value) = self.evaluate(body, &[], &[]) {
                let body = self
                    .program
                    .push(Expr::Literal(literal(&value)), Level::Constant);
                self.program.operators[index].body = body;
            }
        }
        self.random = self.random.checkpoint()?;
        self.caches = self.caches.checkpoint().map_err(Error)?;
        Ok(())
    }
    pub fn set_constants(&mut self, constants: Vec<Value>) -> Result<(), Error> {
        let mut strings = self
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
            let key =
                tla_runtime::intern_string(&self.strings, name.into(), true).map_err(Error)?;
            fields.push((key, int(count)));
            published.insert(name.to_owned(), int(count));
        }
        published.insert("stats".into(), Value::function(fields)?);
        self.registers.publish(published).map_err(Error)
    }
    /// Publishes `_POSSIBLE` counts as TLC's `_Possible` module reads them:
    /// `TLCGet("all:named")["s:_possible"]`, one record per worker.
    pub fn publish_possible(&self, counts: &[(String, usize)]) -> Result<(), Error> {
        let string = |text: &str| {
            tla_runtime::intern_string(&self.strings, text.into(), true).map_err(Error)
        };
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
        self.registers.publish(published).map_err(Error)
    }
    /// Seeds random operators as TLC's `-seed` does before any evaluation.
    pub fn seed(&mut self, seed: i64) {
        self.random = tla_eval::RandomSessions::new(seed, tla_eval::Random::new(seed));
    }
    pub fn set_random_seed(&mut self, seed: i64, state: u64) -> Result<(), Error> {
        self.random =
            tla_eval::RandomSessions::new(seed, tla_eval::Random::from_seed_state(state)?);
        Ok(())
    }
    fn begin_successors(&self, state: &State) -> Result<(), Error> {
        if !self.randomized {
            return Ok(());
        }
        let polynomial = self
            .program
            .fingerprint_polynomial
            .unwrap_or(0x9114_98ae_0e66_bad6);
        let fingerprint = self.key(state)?.iter().fold(polynomial, |fp, value| {
            value.extend_fingerprint(fp, polynomial)
        });
        Ok(self.random.begin(Some(fingerprint))?)
    }
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
            self.init
                .iter()
                .copied()
                .chain(
                    self.actions
                        .iter()
                        .chain(&self.invariants)
                        .chain(&self.initial_properties)
                        .chain(&self.action_properties)
                        .map(|(_, id)| *id),
                )
                .chain(self.state_constraints.iter().copied())
                .chain(self.action_constraints.iter().copied())
                .chain(self.view),
            &[tla_ir::Builtin::TlcSet],
        )
    }
}
impl TransitionSystem for BoundModel {
    type State = State;
    type Key = Vec<Value>;
    type Error = Error;
    fn prepare_parallel(&self, workers: std::num::NonZeroUsize) -> Result<(), Error> {
        self.registers.configure(workers);
        Ok(())
    }
    fn parallel_callbacks(&self) -> bool {
        self.uses_registers()
    }
    fn prefetch_successors(&self) -> bool {
        !self.uses_registers()
            && !self.program.uses_random(
                self.invariants
                    .iter()
                    .chain(&self.initial_properties)
                    .chain(&self.action_properties)
                    .map(|(_, id)| *id)
                    .chain(self.state_constraints.iter().copied())
                    .chain(self.action_constraints.iter().copied())
                    .chain(self.view),
            )
    }
    fn initial_states(&self) -> Result<Vec<State>, Error> {
        self.registers.reset().map_err(Error)?;
        self.caches.reset().map_err(Error)?;
        self.random.begin(None)?;
        if self.init.is_empty() && self.program.variables.is_empty() {
            return Ok(Vec::new());
        }
        states(self, &self.init, &Vec::new().into(), true)
    }
    fn successors(&self, state: &State) -> Result<Vec<Transition<State>>, Error> {
        self.begin_successors(state)?;
        let mut transitions = Vec::new();
        for (action, (_, predicate)) in self.actions.iter().enumerate() {
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
        self.registers.reset().map_err(Error)?;
        self.caches.reset().map_err(Error)?;
        self.random.begin(None)?;
        if self.init.is_empty() && self.program.variables.is_empty() {
            return Ok(std::ops::ControlFlow::Continue(()));
        }
        generate::visit(self, &self.init, &Vec::new().into(), true, output)
    }
    fn visit_successors(
        &self,
        state: &State,
        output: &mut impl FnMut(Transition<State>) -> Result<std::ops::ControlFlow<()>, Error>,
    ) -> Result<std::ops::ControlFlow<()>, Error> {
        self.begin_successors(state)?;
        for (action, (_, predicate)) in self.actions.iter().enumerate() {
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
        for permutation in &self.symmetry {
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
        match self.view {
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
        self.allowed(&self.state_constraints, state, &[])
    }
    fn step_allowed(&self, state: &State, next: &State) -> Result<bool, Error> {
        self.allowed(&self.action_constraints, state, next)
    }
    fn invariant(&self, state: &State) -> Result<Option<String>, Error> {
        self.failure(&self.invariants, state, &[])
    }
    fn initial_property(&self, state: &State) -> Result<Option<String>, Error> {
        self.failure(&self.initial_properties, state, &[])
    }
    fn action_property(&self, state: &State, next: &State) -> Result<Option<String>, Error> {
        self.failure(&self.action_properties, state, next)
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
