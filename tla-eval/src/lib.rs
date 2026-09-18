mod deferred;
mod functional;
mod sets;
pub use functional::{bind, materialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
pub use tla_library::{BooleanContext, Error, boolean, literal};
pub use tla_random::{Random, Sessions as RandomSessions};
pub use tla_runtime::{Cache, Caches, Registers};
pub type Strings = Arc<Mutex<BTreeMap<String, u32>>>;
pub type Enabled<'a> = dyn Fn(ExprId, &[Option<Value>]) -> Result<bool, Error> + 'a;
use tla_ir::{Builtin, Expr, ExprId, Program, Quantifier};
pub use tla_values::{SetExpr, Value};

#[derive(Clone)]
pub struct Evaluator<'a> {
    pub cache: Arc<Cache>,
    pub worker: Option<usize>,
    pub registers: Option<&'a Registers>,
    pub random: Option<&'a Mutex<tla_random::Random>>,
    pub started: std::time::Instant,
    pub level: usize,
    pub intern_new_strings: bool,
    pub enabled: Option<&'a Enabled<'a>>,
    pub strings: Strings,
    at: Option<Value>,
    pub program: &'a Program,
    pub constants: &'a [Value],
    pub state: &'a [Option<Value>],
    pub next: &'a [Option<Value>],
    pub recursion_limit: usize,
}
impl<'a> Evaluator<'a> {
    pub fn new(
        program: &'a Program,
        constants: &'a [Value],
        state: &'a [Option<Value>],
        next: &'a [Option<Value>],
    ) -> Self {
        Self::with_shared_storage(
            program,
            constants,
            state,
            next,
            Arc::new(Mutex::new(program.strings.clone())),
            Arc::default(),
        )
    }
    pub fn with_shared_storage(
        program: &'a Program,
        constants: &'a [Value],
        state: &'a [Option<Value>],
        next: &'a [Option<Value>],
        strings: Strings,
        cache: Arc<Cache>,
    ) -> Self {
        Self {
            cache,
            worker: None,
            registers: None,
            random: None,
            started: std::time::Instant::now(),
            level: 1,
            intern_new_strings: true,
            enabled: None,
            program,
            constants,
            state,
            next,
            recursion_limit: 4096,
            strings,
            at: None,
        }
    }
    pub fn with_strings(mut self, strings: Strings) -> Self {
        self.strings = strings;
        self
    }
    pub fn evaluate_runtime(&self, id: ExprId) -> Result<tla_value_expr::Expr, Error> {
        deferred::runtime(self, id, &mut Vec::new(), 0)
    }
    pub fn evaluate(&self, id: ExprId) -> Result<Value, Error> {
        self.with_locals(id, &[])
    }
    pub fn with_locals(&self, id: ExprId, locals: &[Option<Value>]) -> Result<Value, Error> {
        self.eval(id, &mut locals.to_vec(), 0)
    }
    fn eval(
        &self,
        id: ExprId,
        locals: &mut Vec<Option<Value>>,
        depth: usize,
    ) -> Result<Value, Error> {
        stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
            self.eval_inner(id, locals, depth)
                .map_err(|error| Error(tla_library::contextual_error(self.program, id, error.0)))
        })
    }
    fn if_guard(&self, id: ExprId, value: Value) -> Result<bool, Error> {
        Ok(boolean(
            &value,
            BooleanContext::If,
            self.program.source(id),
        )?)
    }
    fn eval_inner(
        &self,
        id: ExprId,
        locals: &mut Vec<Option<Value>>,
        depth: usize,
    ) -> Result<Value, Error> {
        if depth >= self.recursion_limit {
            return Err(Error("recursion limit exceeded".into()));
        }
        let expression = &self
            .program
            .nodes
            .get(id)
            .ok_or_else(|| Error("invalid expression".into()))?
            .expr;
        let evaluate = |id, locals: &mut Vec<Option<Value>>| self.eval(id, locals, depth + 1);
        if let Expr::Apply { op, args } = expression
            && let Some(value) = deferred::query(self, *op, args, locals, depth)?
        {
            return Ok(value);
        }
        match expression {
            Expr::AssertError { message, body } => {
                functional::assert_error(self, message.as_deref(), *body, locals, depth)
            }
            Expr::Scoped { current, .. } => evaluate(*current, locals),
            Expr::Lambda { .. } | Expr::BuiltinOperator(_) => Err(Error(
                "operator cannot be materialized as a state value".into(),
            )),
            Expr::Invoke { function, args, .. } => {
                let values = args
                    .iter()
                    .map(|arg| evaluate(*arg, locals))
                    .collect::<Result<_, _>>()?;
                functional::invoke(self, *function, values, locals, depth + 1)
            }
            Expr::Apply {
                op: Builtin::SelectSeq | Builtin::SortSeq,
                args,
            } => functional::sequence_operator(self, expression, args, locals, depth + 1),
            Expr::At => self
                .at
                .clone()
                .ok_or_else(|| Error("@ outside EXCEPT".into())),
            Expr::Literal(literal) => materialize(literal),
            Expr::Variable { slot, primed } => (if *primed { self.next } else { self.state })
                .get(*slot)
                .and_then(Clone::clone)
                .ok_or_else(|| {
                    Error(format!(
                        "unbound variable {slot}{}",
                        if *primed { "'" } else { "" }
                    ))
                }),
            Expr::Constant(slot) => self
                .constants
                .get(*slot)
                .cloned()
                .ok_or_else(|| Error(format!("unbound constant {slot}"))),
            Expr::Local(slot) => locals
                .get(*slot)
                .and_then(Clone::clone)
                .ok_or_else(|| Error(format!("unbound local {slot}"))),
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => evaluate(
                if self.if_guard(id, evaluate(*condition, locals)?)? {
                    *then_expr
                } else {
                    *else_expr
                },
                locals,
            ),
            Expr::Call { operator, args } => {
                let op = self
                    .program
                    .operators
                    .get(*operator)
                    .ok_or_else(|| Error("unknown operator".into()))?;
                if op.parameters.len() != args.len() {
                    return Err(Error("operator arity mismatch".into()));
                }
                let values = args
                    .iter()
                    .map(|id| evaluate(*id, locals))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut nested = locals.clone();
                for (&slot, value) in op.parameters.iter().zip(values) {
                    bind(&mut nested, slot, value);
                }
                evaluate(op.body, &mut nested)
            }
            Expr::Apply {
                op: Builtin::And,
                args,
            } => {
                for arg in args {
                    if !evaluate(*arg, locals)?.boolean()? {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            Expr::Apply {
                op: Builtin::Or,
                args,
            } => {
                for arg in args {
                    if evaluate(*arg, locals)?.boolean()? {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            Expr::Apply {
                op: Builtin::Implies,
                args,
            } if args.len() == 2 => Ok(Value::Bool(
                !evaluate(args[0], locals)?.boolean()? || evaluate(args[1], locals)?.boolean()?,
            )),
            Expr::Apply {
                op: Builtin::Unchanged,
                args,
            } => {
                for arg in args {
                    if !self.unchanged(*arg, locals, depth + 1)? {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            Expr::Apply {
                op: Builtin::Except,
                args,
            } => {
                let (&base, updates) = args
                    .split_first()
                    .ok_or_else(|| Error("EXCEPT without base".into()))?;
                let mut function = evaluate(base, locals)?;
                if !matches!(function, Value::Function(_)) {
                    return Err(Error("EXCEPT requires a function".into()));
                }
                for update in updates {
                    let Expr::Apply {
                        op: Builtin::Tuple,
                        args,
                    } = &self.program.nodes[*update].expr
                    else {
                        return Err(Error("invalid EXCEPT update".into()));
                    };
                    if args.len() != 2 {
                        return Err(Error("invalid EXCEPT update arity".into()));
                    }
                    let path = evaluate(args[0], locals)?.sequence()?;
                    let mut previous = function.clone();
                    let mut found = true;
                    for key in &path {
                        match previous.apply(key) {
                            Ok(value) => previous = value,
                            Err(_) => {
                                found = false;
                                break;
                            }
                        }
                    }
                    if found {
                        let evaluator = Evaluator {
                            at: Some(previous),
                            ..self.clone()
                        };
                        let replacement = evaluator.eval(args[1], locals, depth + 1)?;
                        function = function.except(&path, replacement)?;
                    }
                }
                Ok(function)
            }
            Expr::Apply {
                op: Builtin::Case,
                args,
            } => {
                for pair in args {
                    let Expr::Apply {
                        op: Builtin::Tuple,
                        args,
                    } = &self.program.nodes[*pair].expr
                    else {
                        return Err(Error("invalid CASE branch".into()));
                    };
                    if args.len() != 2 {
                        return Err(Error("invalid CASE branch arity".into()));
                    }
                    if evaluate(args[0], locals)?.boolean()? {
                        return evaluate(args[1], locals);
                    }
                }
                Err(Error("CASE has no true condition".into()))
            }
            Expr::Apply { op, args } => {
                let values = args
                    .iter()
                    .map(|id| evaluate(*id, locals))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(Value::String(first)) = values.first() {
                    match op {
                        Builtin::Concat if values.len() == 2 => {
                            let Value::String(second) = &values[1] else {
                                return Err(Error("string concatenation expects strings".into()));
                            };
                            return self.string(format!("{}{}", first.text, second.text));
                        }
                        Builtin::Tail if values.len() == 1 => {
                            let units: Vec<_> = first.text.encode_utf16().collect();
                            if units.is_empty() {
                                return Err(Error("Tail of empty sequence".into()));
                            }
                            return self.string(
                                String::from_utf16(&units[1..])
                                    .map_err(|e| Error(e.to_string()))?,
                            );
                        }
                        Builtin::Subsequence if values.len() == 3 => {
                            let units: Vec<_> = first.text.encode_utf16().collect();
                            let range = tla_library::subsequence_range(
                                &values[0],
                                &values[1],
                                &values[2],
                                units.len(),
                            )?;
                            let units = &units[range];
                            return self.string(
                                String::from_utf16(units).map_err(|e| Error(e.to_string()))?,
                            );
                        }
                        Builtin::Len if values.len() == 1 => {
                            return i32::try_from(first.text.encode_utf16().count())
                                .map(Value::Int)
                                .map_err(|e| Error(e.to_string()));
                        }
                        _ => {}
                    }
                }
                tla_library::validate_method_args(
                    self.program.methods.get(&id).map(String::as_str),
                    &values,
                )?;
                tla_library::apply_builtin(*op, values)
            }
            Expr::Quantify {
                kind,
                slot,
                domain,
                body,
            } => {
                let domain = functional::enumerate(
                    self,
                    domain
                        .ok_or_else(|| Error("unbounded quantifier cannot be enumerated".into()))?,
                    locals,
                    depth,
                )?;
                let mut output = Vec::new();
                for value in &domain {
                    deferred::validate_tuple_binding(
                        self,
                        *body,
                        *slot,
                        value,
                        matches!(kind, Quantifier::Exists | Quantifier::ForAll),
                    )?;
                    let mut nested = locals.clone();
                    bind(&mut nested, *slot, value.clone());
                    let result = evaluate(*body, &mut nested)?;
                    match kind {
                        Quantifier::Exists if result.boolean()? => return Ok(Value::Bool(true)),
                        Quantifier::ForAll if !result.boolean()? => return Ok(Value::Bool(false)),
                        Quantifier::SetFilter if result.boolean()? => output.push(value.clone()),
                        Quantifier::SetMap => output.push(result),
                        _ => {}
                    }
                }
                match kind {
                    Quantifier::Exists => Ok(Value::Bool(false)),
                    Quantifier::ForAll => Ok(Value::Bool(true)),
                    _ => Ok(Value::set(output)?),
                }
            }
            Expr::Function { slot, domain, body } => {
                let domain = functional::enumerate(
                    self,
                    domain.ok_or_else(|| Error("function value requires a domain".into()))?,
                    locals,
                    depth,
                )?;
                let mut entries = Vec::new();
                for key in &domain {
                    deferred::validate_tuple_binding(self, *body, *slot, key, true)?;
                    let mut nested = locals.clone();
                    bind(&mut nested, *slot, key.clone());
                    entries.push((key.clone(), evaluate(*body, &mut nested)?));
                }
                Ok(Value::function(entries)?)
            }
            Expr::RecursiveFunction { .. } => {
                Err(Error("recursive function requires application".into()))
            }
            Expr::Choose { slot, domain, body } => {
                let domain = functional::enumerate(
                    self,
                    domain.ok_or_else(|| Error("unbounded CHOOSE cannot be evaluated".into()))?,
                    locals,
                    depth,
                )?;
                for value in &domain {
                    deferred::validate_tuple_binding(self, *body, *slot, value, true)?;
                    let mut nested = locals.clone();
                    bind(&mut nested, *slot, value.clone());
                    if evaluate(*body, &mut nested)?.boolean()? {
                        return Ok(value.clone());
                    }
                }
                Err(Error("CHOOSE has no satisfying element".into()))
            }
            Expr::Except {
                function,
                key,
                value,
            } => {
                let function = evaluate(*function, locals)?;
                let key = evaluate(*key, locals)?;
                let Value::Function(entries) = function else {
                    return Err(Error("EXCEPT requires a function".into()));
                };
                let replacement = evaluate(*value, locals)?;
                Ok(Value::function(
                    entries
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                if *k == key {
                                    replacement.clone()
                                } else {
                                    v.clone()
                                },
                            )
                        })
                        .collect(),
                )?)
            }
            Expr::Temporal { op, .. } => Err(Error(format!(
                "temporal expression {op} requires behavior evaluation"
            ))),
        }
    }
    fn unchanged(
        &self,
        id: ExprId,
        locals: &mut Vec<Option<Value>>,
        depth: usize,
    ) -> Result<bool, Error> {
        match &self.program.nodes[id].expr {
            Expr::Variable {
                slot,
                primed: false,
            } => {
                let bound = |state: &'a [Option<Value>], which: &str| {
                    let value = state.get(*slot).and_then(Option::as_ref);
                    value.ok_or_else(|| Error(format!("unbound UNCHANGED {which}variable")))
                };
                Ok(bound(self.state, "")?.semantic_eq(bound(self.next, "next ")?)?)
            }
            // An instance variable within ENABLED: its expression and its stand-in.
            Expr::Scoped { current, next } => Ok(self
                .eval(*current, locals, depth)?
                .semantic_eq(&self.eval(*next, locals, depth)?)?),
            Expr::Apply {
                op: Builtin::Tuple,
                args,
            } => {
                for arg in args {
                    if !self.unchanged(*arg, locals, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => {
                let current = self.eval(id, locals, depth)?;
                let next = Evaluator {
                    state: self.next,
                    ..self.clone()
                }
                .eval(id, locals, depth)?;
                Ok(current.semantic_eq(&next)?)
            }
        }
    }
}

/// Interns `text` in the program's string table, assigning the next ordinal
/// when `allow_new` permits it.
pub fn intern_string(
    strings: &Mutex<BTreeMap<String, u32>>,
    text: String,
    allow_new: bool,
) -> Result<Value, String> {
    let mut strings = strings.lock().map_err(|_| "string intern table poisoned")?;
    if !allow_new && !strings.contains_key(&text) {
        return Err("parallel evaluation cannot assign a new string ordinal".into());
    }
    let next = strings
        .values()
        .copied()
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("string ordinal overflow")?;
    let ordinal = *strings.entry(text.clone()).or_insert(next);
    Ok(Value::string(text, ordinal))
}
