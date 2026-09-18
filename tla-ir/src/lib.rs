//! The rewrite's intermediate representation: a flat arena of expression
//! nodes with slot-based bindings and one level per node.

mod analysis;
mod builtins;
mod transform;

pub use builtins::{Builtin, Quantifier};
use std::fmt;

pub type ExprId = usize;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum Level {
    #[default]
    Constant,
    State,
    Action,
    Temporal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Literal {
    Bool(bool),
    Int(i32),
    String { ordinal: u32, text: String },
    Model { ordinal: u32, text: String },
    Set(Vec<Literal>),
    Tuple(Vec<Literal>),
    Function(Vec<(Literal, Literal)>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expr {
    AssertError {
        message: Option<String>,
        body: ExprId,
    },
    BuiltinOperator(Builtin),
    /// An ENABLED binding retains distinct current and primed interpretations.
    Scoped {
        current: ExprId,
        next: ExprId,
    },
    Lambda {
        parameters: Vec<usize>,
        body: ExprId,
    },
    Invoke {
        function: ExprId,
        args: Vec<ExprId>,
        literal_message: bool,
    },
    At,
    Literal(Literal),
    Variable {
        slot: usize,
        primed: bool,
    },
    Local(usize),
    Constant(usize),
    Apply {
        op: Builtin,
        args: Vec<ExprId>,
    },
    Call {
        operator: usize,
        args: Vec<ExprId>,
    },
    If {
        condition: ExprId,
        then_expr: ExprId,
        else_expr: ExprId,
    },
    Quantify {
        kind: Quantifier,
        slot: usize,
        /// `None` is `\E x : p` / `\A x : p` over an unenumerable universe.
        domain: Option<ExprId>,
        body: ExprId,
    },
    Function {
        slot: usize,
        /// `None` is `f[x] == e` without a domain; `DOMAIN f` must be given
        /// separately and applications cannot be membership-checked.
        domain: Option<ExprId>,
        body: ExprId,
    },
    RecursiveFunction {
        slot: usize,
        domain: Option<ExprId>,
        body: ExprId,
    },
    Choose {
        slot: usize,
        /// `None` is `CHOOSE x : p`, which TLC cannot evaluate.
        domain: Option<ExprId>,
        body: ExprId,
    },
    Except {
        function: ExprId,
        key: ExprId,
        value: ExprId,
    },
    Temporal {
        op: String,
        args: Vec<ExprId>,
        bindings: Vec<(usize, ExprId)>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub expr: Expr,
    pub level: Level,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operator {
    pub name: String,
    pub parameters: Vec<usize>,
    pub body: ExprId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Program {
    pub sources: std::collections::BTreeMap<ExprId, String>,
    pub methods: std::collections::BTreeMap<ExprId, String>,
    /// The polynomial for state fingerprints, when the configuration picks one;
    /// otherwise [`DEFAULT_FINGERPRINT_POLYNOMIAL`].
    pub fingerprint_polynomial: Option<u64>,
    pub function_labels: std::collections::BTreeMap<ExprId, (String, String)>,
    pub strings: std::collections::BTreeMap<String, u32>,
    pub nodes: Vec<Node>,
    pub operators: Vec<Operator>,
    pub variables: Vec<String>,
    /// Temporary next-state slots for ENABLED, excluded from stored states.
    pub enabled_variables: usize,
    pub constants: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

impl Program {
    pub fn at_source(&mut self, id: ExprId, source: String) -> ExprId {
        self.sources.entry(id).or_insert(source);
        id
    }
    pub fn source(&self, id: ExprId) -> Option<&str> {
        self.sources.get(&id).map(String::as_str)
    }
    pub fn derived(&mut self, from: ExprId, expr: Expr, level: Level) -> ExprId {
        let id = self.push(expr, level);
        self.inherit(from, id)
    }
    pub fn inherit(&mut self, from: ExprId, id: ExprId) -> ExprId {
        if let Some(source) = self.sources.get(&from).cloned() {
            self.at_source(id, source);
        }
        if let Some(method) = self.methods.get(&from).cloned() {
            self.methods.entry(id).or_insert(method);
        }
        id
    }
    pub fn checked_apply(
        &mut self,
        op: Builtin,
        args: Vec<ExprId>,
        literal_message: bool,
        level: Level,
    ) -> Result<ExprId, Error> {
        let expr = if op == Builtin::TlcCache {
            if args.len() != 2 {
                return Err(Error("TLCCache expects two operands".into()));
            }
            let site = -i32::try_from(self.nodes.len()).map_err(|e| Error(e.to_string()))? - 1;
            let metadata = self.push(
                Expr::Literal(Literal::Tuple(vec![
                    Literal::Int(site),
                    Literal::Function(Vec::new()),
                ])),
                Level::Constant,
            );
            let mut args = args;
            args.push(metadata);
            Expr::Apply {
                op: Builtin::Memo,
                args,
            }
        } else if op == Builtin::AssertError {
            let [expected, body] = args.as_slice() else {
                return Err(Error("AssertError expects two operands".into()));
            };
            let message = match &self.nodes[*expected].expr {
                Expr::Literal(Literal::String { text, .. }) if literal_message => {
                    Some(text.clone())
                }
                _ => None,
            };
            Expr::AssertError {
                message,
                body: *body,
            }
        } else {
            Expr::Apply { op, args }
        };
        Ok(self.push(expr, level))
    }
    pub fn temporal(
        &mut self,
        op: String,
        args: Vec<ExprId>,
        bindings: Vec<(usize, ExprId)>,
        level: Level,
    ) -> ExprId {
        self.push(Expr::Temporal { op, args, bindings }, level)
    }
    pub fn temporal_expr(op: String, args: Vec<ExprId>, bindings: Vec<(usize, ExprId)>) -> Expr {
        Expr::Temporal { op, args, bindings }
    }

    pub fn push(&mut self, expr: Expr, level: Level) -> ExprId {
        if let Expr::Literal(literal) = &expr {
            let mut pending = vec![literal];
            while let Some(value) = pending.pop() {
                match value {
                    Literal::String { ordinal, text } | Literal::Model { ordinal, text } => {
                        self.strings.insert(text.clone(), *ordinal);
                    }
                    Literal::Set(values) | Literal::Tuple(values) => pending.extend(values),
                    Literal::Function(entries) => {
                        for (key, value) in entries {
                            pending.extend([key, value]);
                        }
                    }
                    _ => {}
                }
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node { expr, level });
        id
    }
    pub fn validate(&self) -> Result<(), Error> {
        for node in &self.nodes {
            let mut references: Vec<&ExprId> = Vec::new();
            match &node.expr {
                Expr::Variable { slot, .. }
                    if *slot >= self.variables.len() + self.enabled_variables =>
                {
                    return Err(Error("invalid variable slot".into()));
                }
                Expr::Constant(slot) if *slot >= self.constants.len() => {
                    return Err(Error("invalid constant slot".into()));
                }
                Expr::Scoped { current, next } => references.extend([current, next]),
                Expr::Apply { args, .. } => references.extend(args),
                Expr::Lambda { body, .. } | Expr::AssertError { body, .. } => references.push(body),
                Expr::Invoke { function, args, .. } => {
                    references.push(function);
                    references.extend(args);
                }
                Expr::Call { operator, args } => {
                    let op = self
                        .operators
                        .get(*operator)
                        .ok_or_else(|| Error("invalid operator slot".into()))?;
                    if args.len() != op.parameters.len() {
                        return Err(Error("operator arity mismatch".into()));
                    }
                    references.extend(args);
                }
                Expr::If {
                    condition,
                    then_expr,
                    else_expr,
                } => references.extend([condition, then_expr, else_expr]),
                Expr::Quantify { domain, body, .. }
                | Expr::Function { domain, body, .. }
                | Expr::RecursiveFunction { domain, body, .. }
                | Expr::Choose { domain, body, .. } => {
                    references.extend(domain);
                    references.push(body);
                }
                Expr::Except {
                    function,
                    key,
                    value,
                } => references.extend([function, key, value]),
                Expr::Temporal { args, bindings, .. } => {
                    references.extend(args);
                    references.extend(bindings.iter().map(|(_, id)| id));
                }
                _ => {}
            }
            if references.into_iter().any(|id| *id >= self.nodes.len()) {
                return Err(Error("invalid expression reference".into()));
            }
        }
        if self.operators.iter().any(|op| op.body >= self.nodes.len()) {
            return Err(Error("invalid operator body".into()));
        }
        Ok(())
    }
}

impl Program {
    /// `slot` holds a tuple; read its `index`-th member after checking `arity`.
    pub fn tuple_local(&mut self, slot: usize, index: usize, arity: usize) -> ExprId {
        let local = self.push(Expr::Local(slot), Level::Constant);
        let arity = self.push(Expr::Literal(Literal::Int(arity as i32)), Level::Constant);
        let tuple = self.push(
            Expr::Apply {
                op: Builtin::Destructure,
                args: vec![local, arity],
            },
            Level::Constant,
        );
        let index = self.push(
            Expr::Literal(Literal::Int(index as i32 + 1)),
            Level::Constant,
        );
        self.push(
            Expr::Apply {
                op: Builtin::Apply,
                args: vec![tuple, index],
            },
            Level::Constant,
        )
    }

    /// The domain of a `arity`-tuple binder over `domain`, as the set of
    /// tuples that destructuring accepts.
    pub fn tuple_domain(
        &mut self,
        slot: usize,
        domain: ExprId,
        arity: usize,
        exact: bool,
    ) -> ExprId {
        let arity = self.push(Expr::Literal(Literal::Int(arity as i32)), Level::Constant);
        let local = self.push(Expr::Local(slot), Level::Constant);
        let check = self.push(
            Expr::Apply {
                op: if exact {
                    Builtin::DestructureExact
                } else {
                    Builtin::Destructure
                },
                args: vec![local, arity],
            },
            Level::Constant,
        );
        self.push(
            Expr::Quantify {
                kind: Quantifier::SetMap,
                slot,
                domain: Some(domain),
                body: check,
            },
            Level::Constant,
        )
    }
}

/// TLC's first fingerprint polynomial, used when the configuration picks none.
pub const DEFAULT_FINGERPRINT_POLYNOMIAL: u64 = 0x9114_98ae_0e66_bad6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformations_retain_diagnostic_origins() {
        let mut p = Program::default();
        let local = p.push(Expr::Local(0), Level::Constant);
        let root = p.push(
            Expr::Apply {
                op: Builtin::Not,
                args: vec![local],
            },
            Level::Constant,
        );
        p.at_source(root, "source location".into());
        p.methods.insert(root, "method descriptor".into());
        let value = p.push(Expr::Literal(Literal::Bool(true)), Level::Constant);
        let copy = p.copy_expr(root);
        let replaced = p.replace_local(root, 0, value);
        let primed = p.prime(replaced).unwrap();
        let raised = p.at_least(root, Level::Action);
        for id in [copy, replaced, primed, raised] {
            assert_eq!(p.source(id), Some("source location"));
            assert_eq!(p.methods[&id], "method descriptor");
        }
        let function = p.push(Expr::BuiltinOperator(Builtin::Not), Level::Constant);
        p.inherit(root, function);
        let invoked = p
            .invoke(function, vec![value], false, Level::Constant)
            .unwrap();
        assert_eq!(p.methods[&invoked], "method descriptor");
    }
}
