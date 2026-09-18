//! The TLA+ surface syntax: tokens, the expression and module trees, and a
//! hand-written parser for the subset the checker understands. Everything
//! downstream consumes [`Module`] and [`Expr`]; nothing here knows about
//! values or evaluation.
mod lexer;
mod parser;

pub use lexer::{Token, lex};
pub use parser::{expression, module};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for Diagnostic {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Integer(i64),
    String(String),
    Name(String),
    Call(String, Vec<Expr>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Set(Vec<Expr>),
    Tuple(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Let(Vec<Definition>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A binder. Several names destructure a tuple (`<<a, b>> \in S`, or the
    /// arguments of `[x \in S, y \in T |-> e]` over `S \X T`); no domain is an
    /// unbounded `\E x : p`, `\A x : p` or `CHOOSE x : p`.
    Bound {
        kind: String,
        names: Vec<String>,
        tuple: bool,
        domain: Option<Box<Expr>>,
        body: Box<Expr>,
    },
    Index(Box<Expr>, Box<Expr>),
    Stutter(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub name: String,
    pub parameters: Vec<String>,
    /// The arity of each parameter; nonzero for operator parameters such as `F(_)`.
    pub arities: Vec<usize>,
    pub body: Expr,
    /// Declared by `RECURSIVE`, so references may precede the body.
    pub recursive: bool,
    /// Defined by a standard or community module, whose operators TLC may implement
    /// natively instead of evaluating the definition.
    pub standard: bool,
}

impl Default for Definition {
    fn default() -> Self {
        Self {
            name: String::new(),
            parameters: Vec::new(),
            arities: Vec::new(),
            body: Expr::Name("TRUE".into()),
            recursive: false,
            standard: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub extends: Vec<String>,
    pub variables: Vec<String>,
    pub constants: Vec<String>,
    pub definitions: Vec<Definition>,
    pub assumptions: Vec<Expr>,
    pub instances: Vec<Instance>,
    pub local_names: Vec<String>,
    /// Modules nested in this one, which it may extend or instantiate.
    pub modules: Vec<Module>,
    /// Constants and variables with the number of this module's definitions
    /// that precede their declaration, which is where they come into scope.
    pub declared_at: Vec<(usize, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub local: bool,
    pub name: Option<String>,
    /// Parameters of `P(x) == INSTANCE M`, prepended to every instantiated operator.
    pub parameters: Vec<String>,
    pub module: String,
    pub substitutions: Vec<(String, Expr)>,
}

pub fn substitute(
    expression: &Expr,
    replacements: &std::collections::BTreeMap<String, Expr>,
) -> Result<Expr, Diagnostic> {
    let apply = |expr| substitute(expr, replacements);
    Ok(match expression {
        Expr::Name(name) => replacements
            .get(name)
            .cloned()
            .unwrap_or_else(|| expression.clone()),
        Expr::Call(name, args) => {
            let args = args.iter().map(apply).collect::<Result<Vec<_>, _>>()?;
            match replacements.get(name) {
                Some(Expr::Name(name)) => Expr::Call(name.clone(), args),
                // A partially applied operator, such as one from a parameterized instance.
                Some(Expr::Call(name, prefix)) => {
                    Expr::Call(name.clone(), prefix.iter().cloned().chain(args).collect())
                }
                Some(value) if args.is_empty() => value.clone(),
                Some(_) => {
                    return Err(Diagnostic {
                        offset: 0,
                        message: "operator substitution needs an operator".into(),
                    });
                }
                None => Expr::Call(name.clone(), args),
            }
        }
        Expr::Unary(op, child) => Expr::Unary(op.clone(), Box::new(apply(child)?)),
        Expr::Binary(op, a, b) => {
            Expr::Binary(op.clone(), Box::new(apply(a)?), Box::new(apply(b)?))
        }
        Expr::Index(a, b) => Expr::Index(Box::new(apply(a)?), Box::new(apply(b)?)),
        Expr::Stutter(a, b) => Expr::Stutter(Box::new(apply(a)?), Box::new(apply(b)?)),
        Expr::If(a, b, c) => Expr::If(
            Box::new(apply(a)?),
            Box::new(apply(b)?),
            Box::new(apply(c)?),
        ),
        Expr::Set(values) => Expr::Set(values.iter().map(apply).collect::<Result<_, _>>()?),
        Expr::Tuple(values) => Expr::Tuple(values.iter().map(apply).collect::<Result<_, _>>()?),
        Expr::Record(fields) => Expr::Record(
            fields
                .iter()
                .map(|(name, value)| Ok((name.clone(), apply(value)?)))
                .collect::<Result<_, Diagnostic>>()?,
        ),
        Expr::Bound {
            kind,
            names,
            tuple,
            domain,
            body,
        } => {
            let mut nested = replacements.clone();
            let mut renames = std::collections::BTreeMap::new();
            let mut renamed = Vec::new();
            for name in names {
                nested.remove(name);
                let fresh = fresh(name, expression, replacements);
                renames.insert(name.clone(), Expr::Name(fresh.clone()));
                renamed.push(fresh);
            }
            let body = substitute(body, &renames)?;
            Expr::Bound {
                kind: kind.clone(),
                names: renamed,
                tuple: *tuple,
                domain: domain.as_deref().map(apply).transpose()?.map(Box::new),
                body: Box::new(substitute(&body, &nested)?),
            }
        }
        Expr::Let(definitions, body) => {
            let mut nested = replacements.clone();
            for definition in definitions {
                nested.remove(&definition.name);
            }
            let renames = definitions
                .iter()
                .map(|definition| {
                    (
                        definition.name.clone(),
                        Expr::Name(fresh(&definition.name, expression, replacements)),
                    )
                })
                .collect();
            let body = substitute(body, &renames)?;
            let definitions = definitions
                .iter()
                .map(|definition| {
                    let mut definition = substitute_definition(definition, &renames)?;
                    let Expr::Name(name) = &renames[&definition.name] else {
                        unreachable!()
                    };
                    definition.name = name.clone();
                    substitute_definition(&definition, &nested)
                })
                .collect::<Result<_, Diagnostic>>()?;
            Expr::Let(definitions, Box::new(substitute(&body, &nested)?))
        }
        Expr::Integer(_) | Expr::String(_) => expression.clone(),
    })
}

pub fn substitute_definition(
    definition: &Definition,
    replacements: &std::collections::BTreeMap<String, Expr>,
) -> Result<Definition, Diagnostic> {
    let mut scope = replacements.clone();
    let mut body = definition.body.clone();
    let mut parameters = Vec::new();
    for parameter in &definition.parameters {
        scope.remove(parameter);
        let renamed = fresh(parameter, &body, replacements);
        body = substitute(
            &body,
            &std::collections::BTreeMap::from([(parameter.clone(), Expr::Name(renamed.clone()))]),
        )?;
        parameters.push(renamed);
    }
    Ok(Definition {
        name: definition.name.clone(),
        parameters,
        arities: definition.arities.clone(),
        body: substitute(&body, &scope)?,
        recursive: definition.recursive,
        standard: definition.standard,
    })
}

/// SANY's rule that no bound name, parameter or LET definition redeclares a name
/// already in scope. Definitions come into scope in order, after their bodies.
pub fn check_scopes(
    definitions: &[Definition],
    assumptions: &[Expr],
    declared_at: &[(usize, String)],
    mut scope: std::collections::BTreeSet<String>,
) -> Result<(), Diagnostic> {
    let mut bound = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        scope.extend(
            declared_at
                .iter()
                .filter(|(at, _)| *at <= index)
                .map(|(_, name)| name.clone()),
        );
        if definition.recursive {
            scope.insert(definition.name.clone());
        }
        declare_all(&definition.parameters, &scope, &mut bound)?;
        scopes(&definition.body, &scope, &mut bound)?;
        bound.clear();
        scope.insert(definition.name.clone());
    }
    scope.extend(declared_at.iter().map(|(_, name)| name.clone()));
    assumptions
        .iter()
        .try_for_each(|a| scopes(a, &scope, &mut bound))
}

fn declare_all(
    names: &[String],
    scope: &std::collections::BTreeSet<String>,
    bound: &mut Vec<String>,
) -> Result<(), Diagnostic> {
    for name in names {
        if scope.contains(name) || bound.contains(name) {
            return Err(Diagnostic {
                offset: 0,
                message: format!("Multiply-defined symbol '{name}'"),
            });
        }
        bound.push(name.clone());
    }
    Ok(())
}

fn scopes(
    expression: &Expr,
    scope: &std::collections::BTreeSet<String>,
    bound: &mut Vec<String>,
) -> Result<(), Diagnostic> {
    let depth = bound.len();
    match expression {
        Expr::Bound {
            names,
            domain,
            body,
            ..
        } => {
            if let Some(domain) = domain {
                scopes(domain, scope, bound)?;
            }
            declare_all(names, scope, bound)?;
            scopes(body, scope, bound)?;
        }
        Expr::Let(definitions, body) => {
            for definition in definitions {
                let name = std::slice::from_ref(&definition.name);
                if definition.recursive {
                    declare_all(name, scope, bound)?;
                }
                let inner = bound.len();
                declare_all(&definition.parameters, scope, bound)?;
                scopes(&definition.body, scope, bound)?;
                bound.truncate(inner);
                if !definition.recursive {
                    declare_all(name, scope, bound)?;
                }
            }
            scopes(body, scope, bound)?;
        }
        Expr::Call(name, args) if name == "$Lambda" => {
            if let [Expr::Tuple(names), body] = args.as_slice() {
                for name in names {
                    if let Expr::Name(name) = name {
                        declare_all(std::slice::from_ref(name), scope, bound)?;
                    }
                }
                scopes(body, scope, bound)?;
            }
        }
        Expr::Call(_, args) | Expr::Set(args) | Expr::Tuple(args) => {
            args.iter().try_for_each(|a| scopes(a, scope, bound))?
        }
        Expr::Record(fields) => fields
            .iter()
            .try_for_each(|(_, value)| scopes(value, scope, bound))?,
        Expr::Unary(_, a) => scopes(a, scope, bound)?,
        Expr::Binary(_, a, b) | Expr::Index(a, b) | Expr::Stutter(a, b) => {
            scopes(a, scope, bound)?;
            scopes(b, scope, bound)?;
        }
        Expr::If(a, b, c) => [a, b, c]
            .into_iter()
            .try_for_each(|e| scopes(e, scope, bound))?,
        Expr::Integer(_) | Expr::String(_) | Expr::Name(_) => {}
    }
    bound.truncate(depth);
    Ok(())
}

/// A binder's name, renamed only when a replacement could capture it.
fn fresh(
    name: &str,
    expression: &Expr,
    replacements: &std::collections::BTreeMap<String, Expr>,
) -> String {
    if !names(replacements.values()).contains(name) {
        return name.to_owned();
    }
    let used = names(replacements.values().chain([expression]));
    (0..)
        .map(|index| format!("__sub_{name}_{index}"))
        .find(|candidate| !used.contains(candidate.as_str()))
        .unwrap()
}

fn names<'a>(expressions: impl Iterator<Item = &'a Expr>) -> std::collections::BTreeSet<&'a str> {
    let mut used = std::collections::BTreeSet::new();
    let mut work: Vec<&Expr> = expressions.collect();
    while let Some(expression) = work.pop() {
        match expression {
            Expr::Name(name) => {
                used.insert(name.as_str());
            }
            Expr::Call(name, args) => {
                used.insert(name);
                work.extend(args);
            }
            Expr::Unary(_, child) => work.push(child),
            Expr::Binary(_, a, b) | Expr::Index(a, b) | Expr::Stutter(a, b) => {
                work.extend([a.as_ref(), b.as_ref()])
            }
            Expr::If(a, b, c) => work.extend([a.as_ref(), b.as_ref(), c.as_ref()]),
            Expr::Set(values) | Expr::Tuple(values) => work.extend(values),
            Expr::Record(fields) => work.extend(fields.iter().map(|(_, value)| value)),
            Expr::Bound {
                names,
                domain,
                body,
                ..
            } => {
                used.extend(names.iter().map(String::as_str));
                work.extend(domain.as_deref());
                work.push(body);
            }
            Expr::Let(definitions, body) => {
                work.push(body);
                for definition in definitions {
                    used.insert(&definition.name);
                    used.extend(definition.parameters.iter().map(String::as_str));
                    work.push(&definition.body);
                }
            }
            _ => {}
        }
    }
    used
}

#[cfg(test)]
mod tests;
