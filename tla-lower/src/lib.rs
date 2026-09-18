mod calls;
mod expressions;
use std::collections::BTreeMap;
use tla_ir::{Builtin as B, Expr as I, ExprId, Level, Literal, Operator, Program};
use tla_modules::{Config, Error};
use tla_syntax::{Expr, Module};

pub struct Lowered {
    pub program: Program,
    pub bindings: Vec<(usize, ExprId)>,
    pub overrides: Vec<(usize, ExprId)>,
    pub assumptions: Vec<ExprId>,
    /// Definitions that could not be lowered, by name. They are errors only for
    /// configured formulas that use them, as unused definitions do not affect TLC.
    pub failures: BTreeMap<String, String>,
}

struct Scope {
    definitions: BTreeMap<String, tla_syntax::Definition>,
    locals: Env,
}

/// What a name in scope denotes: a bound value in a local slot, or an IR node such
/// as a tuple component, an operator argument or a recursive function.
#[derive(Clone, Copy)]
enum Binding {
    Slot(usize),
    Node(ExprId),
}

#[derive(Clone, Default)]
struct Env(BTreeMap<String, Binding>);

impl Env {
    fn get(&self, name: &str) -> Option<Binding> {
        self.0.get(name).copied()
    }
    fn slot(&mut self, name: &str, slot: usize) {
        self.0.insert(name.into(), Binding::Slot(slot));
    }
    fn node(&mut self, name: &str, id: ExprId) {
        self.0.insert(name.into(), Binding::Node(id));
    }
}

struct Lower<'a> {
    module: &'a Module,
    program: Program,
    statuses: Vec<u8>,
    symbols: BTreeMap<String, u32>,
    locals: Env,
    next_local: usize,
    /// The name a `recursive function` binder being lowered refers to itself by.
    function_name: Option<String>,
    /// Inside ENABLED, primed compound expressions (instance variables substituted
    /// by expressions) and the temporary next-state variables that stand for them.
    enabled_scope: Vec<(Expr, usize)>,
    scoped: Vec<Scope>,
    /// LET operators being lowered: scope index, name and operator index.
    active: Vec<(usize, String, usize)>,
    /// Levels of `RECURSIVE` operators assumed for references made before their
    /// bodies are known, and the levels their bodies actually had.
    assumed: BTreeMap<String, Level>,
    observed: BTreeMap<String, Level>,
    failures: BTreeMap<usize, String>,
}

/// Recursive operators are lowered to a level fixpoint: a pass assumes their levels,
/// and passes repeat with the raised levels until every body agrees.
pub fn lower(module: &Module, config: &Config) -> Result<Lowered, Error> {
    let mut assumed = BTreeMap::new();
    loop {
        let (lowered, observed) = lower_once(module, config, &assumed)?;
        let raised: Vec<_> = observed
            .into_iter()
            .filter(|(name, level)| assumed.get(name).is_none_or(|a| a < level))
            .collect();
        if raised.is_empty() {
            return Ok(lowered);
        }
        assumed.extend(raised);
    }
}

fn lower_once(
    module: &Module,
    config: &Config,
    assumed: &BTreeMap<String, Level>,
) -> Result<(Lowered, BTreeMap<String, Level>), Error> {
    let mut lower = Lower {
        module,
        program: Program {
            variables: module.variables.clone(),
            constants: module.constants.clone(),
            ..Program::default()
        },
        statuses: vec![0; module.definitions.len()],
        symbols: BTreeMap::new(),
        locals: Env::default(),
        next_local: 0,
        function_name: None,
        enabled_scope: Vec::new(),
        scoped: Vec::new(),
        active: Vec::new(),
        assumed: assumed.clone(),
        observed: BTreeMap::new(),
        failures: BTreeMap::new(),
    };
    let mut names = std::collections::BTreeSet::new();
    for name in module
        .variables
        .iter()
        .chain(&module.constants)
        .chain(module.definitions.iter().map(|d| &d.name))
    {
        if !names.insert(name) {
            return Err(Error(format!("duplicate symbol {name}")));
        }
    }
    lower.program.operators = module
        .definitions
        .iter()
        .map(|d| Operator {
            name: d.name.clone(),
            parameters: Vec::new(),
            body: 0,
        })
        .collect();
    let mut bindings = Vec::new();
    for (name, value) in &config.constants {
        let slot = module
            .constants
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error(format!("undeclared constant {name}")))?;
        let id = lower.constant(value)?;
        bindings.push((slot, id));
    }
    let mut overrides = Vec::new();
    for (name, replacement) in &config.overrides {
        let slot = module
            .constants
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error(format!("override of nonconstant {name} is unsupported")))?;
        let id = lower.expr(&Expr::Name(replacement.clone()))?;
        if lower.program.nodes[id].level != Level::Constant {
            return Err(Error(format!(
                "constant {name} overridden by nonconstant operator"
            )));
        }
        bindings.push((slot, id));
        overrides.push((slot, id));
    }
    for index in 0..module.definitions.len() {
        let (scoped, active) = (lower.scoped.len(), lower.active.len());
        if let Err(error) = lower.definition(index) {
            // Everything still being lowered depends on the failed definition.
            lower.scoped.truncate(scoped);
            lower.active.truncate(active);
            lower.locals = Env::default();
            for failed in 0..lower.statuses.len() {
                if lower.statuses[failed] == 1 {
                    lower.statuses[failed] = 3;
                    lower.failures.insert(failed, error.0.clone());
                    lower.program.operators[failed].body = lower
                        .program
                        .push(I::Literal(Literal::Bool(false)), Level::Constant);
                }
            }
        }
    }
    let assumptions = module
        .assumptions
        .iter()
        .map(|expr| lower.expr(expr))
        .collect::<Result<Vec<_>, _>>()?;
    if assumptions
        .iter()
        .any(|id| lower.program.nodes[*id].level != Level::Constant)
    {
        return Err(Error("ASSUME must be constant-level".into()));
    }
    lower.program.validate().map_err(|e| Error(e.to_string()))?;
    Ok((
        Lowered {
            program: lower.program,
            bindings,
            overrides,
            assumptions,
            failures: lower
                .failures
                .into_iter()
                .map(|(index, error)| (module.definitions[index].name.clone(), error))
                .collect(),
        },
        lower.observed,
    ))
}

impl Lower<'_> {
    fn symbol(&mut self, text: &str) -> u32 {
        let next = self.symbols.len() as u32;
        *self.symbols.entry(text.into()).or_insert(next)
    }
    fn literal(&mut self, literal: Literal) -> ExprId {
        self.program.push(I::Literal(literal), Level::Constant)
    }
    fn constant(&mut self, expression: &Expr) -> Result<ExprId, Error> {
        match expression {
            Expr::Name(name) if name != "TRUE" && name != "FALSE" => {
                let ordinal = self.symbol(name);
                Ok(self.literal(Literal::Model {
                    ordinal,
                    text: name.clone(),
                }))
            }
            Expr::Set(values) | Expr::Tuple(values) => {
                let args = values
                    .iter()
                    .map(|v| self.constant(v))
                    .collect::<Result<_, _>>()?;
                Ok(self.apply(
                    if matches!(expression, Expr::Set(_)) {
                        B::Set
                    } else {
                        B::Tuple
                    },
                    args,
                ))
            }
            _ => self.expr(expression),
        }
    }
    fn definition(&mut self, index: usize) -> Result<(), Error> {
        match self.statuses[index] {
            2 => return Ok(()),
            3 => return Err(Error(self.failures[&index].clone())),
            1 if self.module.definitions[index].recursive => return Ok(()),
            1 => {
                return Err(Error(format!(
                    "operator {} is used recursively without a RECURSIVE declaration",
                    self.module.definitions[index].name
                )));
            }
            _ => {}
        }
        self.statuses[index] = 1;
        let definition = self.module.definitions[index].clone();
        if self.native(index).is_none() && definition.arities.iter().any(|arity| *arity > 0) {
            // Operators with operator parameters are expanded at each call.
            self.program.operators[index].body = self.literal(Literal::Bool(false));
            self.statuses[index] = 2;
            return Ok(());
        }
        let saved = std::mem::take(&mut self.locals);
        let parameters = definition
            .parameters
            .iter()
            .map(|name| {
                let slot = self.fresh();
                self.locals.slot(name, slot);
                slot
            })
            .collect::<Vec<_>>();
        self.program.operators[index].parameters = parameters.clone();
        let body = match self.native(index) {
            Some(op) => {
                let args = parameters
                    .iter()
                    .map(|slot| self.program.push(I::Local(*slot), Level::Constant))
                    .collect();
                Ok(self.apply(op, args))
            }
            None => {
                if calls::is_function(&definition) {
                    self.function_name = Some(definition.name.clone());
                }
                self.expr(&definition.body)
            }
        };
        self.locals = saved;
        let body = body?;
        if definition.recursive {
            self.observe(&definition.name, body);
        }
        self.program.operators[index] = Operator {
            name: definition.name,
            parameters,
            body,
        };
        self.statuses[index] = 2;
        Ok(())
    }
    fn fresh(&mut self) -> usize {
        self.next_local += 1;
        self.next_local - 1
    }
    /// The builtin TLC uses instead of a standard module's definition.
    fn native(&self, index: usize) -> Option<B> {
        let definition = &self.module.definitions[index];
        definition
            .standard
            .then(|| calls::builtin(base(&definition.name)))
            .flatten()
    }
    fn observe(&mut self, name: &str, body: ExprId) {
        let level = self.program.nodes[body].level;
        let entry = self.observed.entry(name.into()).or_default();
        *entry = (*entry).max(level);
    }
    /// The level of an operator's body, or its assumed level while it is being lowered.
    fn body_level(&self, name: &str, body: Option<ExprId>) -> Level {
        match body {
            Some(body) => self.program.nodes[body].level,
            None => self.assumed.get(name).copied().unwrap_or_default(),
        }
    }
    fn apply(&mut self, op: B, args: Vec<ExprId>) -> ExprId {
        let mut level = args
            .iter()
            .map(|id| self.program.nodes[*id].level)
            .max()
            .unwrap_or_default();
        if op == B::Unchanged {
            level = Level::Action;
        }
        self.program.push(I::Apply { op, args }, level)
    }
}

/// An operator's own name, without instance prefixes or a `Module@` local renaming.
fn base(name: &str) -> &str {
    let name = name.rsplit('!').next().unwrap_or(name);
    match name.split_once('@') {
        Some((module, rest))
            if !module.is_empty() && module.chars().all(|c| c.is_alphanumeric() || c == '_') =>
        {
            rest
        }
        _ => name,
    }
}

#[cfg(test)]
mod tests;
