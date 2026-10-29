//! Temporal decomposition of a specification before lowering: `SPECIFICATION`
//! and `PROPERTY` formulas are split into predicates added to the module and
//! templates instantiated once constants are known.
mod template;

pub use template::Template;

use crate::{Config, Error};
use std::collections::BTreeSet;
use tlaplus_syntax::{Definition, Expr, Module};

/// A specification's decomposition. Predicates are added to the module as
/// definitions named here; temporal formulas are instantiated once constants
/// are known, because quantifiers range over constant sets.
#[derive(Default)]
pub struct Temporal {
    /// Fairness and other temporal conjuncts of the specification.
    pub assumptions: Vec<Template>,
    pub properties: Vec<(String, Template)>,
    /// Conjuncts of properties TLC checks as safety: property name and the
    /// definition holding the conjunct's predicate.
    pub action_properties: Vec<(String, String)>,
    pub invariants: Vec<(String, String)>,
    pub initial_properties: Vec<(String, String)>,
}

pub fn prepare(module: &mut Module, config: &mut Config) -> Result<Temporal, Error> {
    let mut result = Temporal::default();
    let original = module.clone();
    if let Some(name) = config.specification.take() {
        let expression = expand(&original, &Expr::Name(name))?;
        let mut conjuncts = Vec::new();
        flatten(&original, &expression, &mut conjuncts)?;
        let mut initial = Vec::new();
        let mut next = None;
        for conjunct in conjuncts {
            if let Expr::Unary(op, child) = &conjunct
                && op == "[]"
                && let Expr::Stutter(action, _) = child.as_ref()
            {
                if next.replace(action.as_ref().clone()).is_some() {
                    return Err(Error(
                        "multiple next-state relations in SPECIFICATION".into(),
                    ));
                }
            } else if temporal(&original, &conjunct, &mut BTreeSet::new()) {
                let template = template::translate(&original, module, &conjunct, &[])?;
                result.assumptions.push(template);
            } else {
                initial.push(conjunct);
            }
        }
        let initial = initial
            .into_iter()
            .reduce(|a, b| Expr::Binary("/\\".into(), Box::new(a), Box::new(b)))
            .unwrap_or_else(|| Expr::Name("TRUE".into()));
        let next = next.ok_or_else(|| Error("SPECIFICATION requires [][Next]_vars".into()))?;
        config.init = Some(add(module, "Init", initial, &[]));
        config.next = Some(add(module, "Next", next, &[]));
    }
    // Like TLC, each conjunct of a property is checked by the cheapest applicable
    // means: `[][A]_v` as an action property, `[]P` as an invariant, a state
    // predicate on initial states, and anything else as a temporal formula.
    for name in std::mem::take(&mut config.properties) {
        let expression = expand(&original, &Expr::Name(name.clone()))?;
        let mut conjuncts = Vec::new();
        flatten(&original, &expression, &mut conjuncts)?;
        for conjunct in conjuncts {
            let is_temporal = |e: &Expr| temporal(&original, e, &mut BTreeSet::new());
            match &conjunct {
                Expr::Unary(op, action) if op == "[]" && matches!(**action, Expr::Stutter(..)) => {
                    let predicate = add(module, "ActionProperty", action.as_ref().clone(), &[]);
                    result.action_properties.push((name.clone(), predicate));
                }
                Expr::Unary(op, predicate) if op == "[]" && !is_temporal(predicate) => {
                    let predicate = add(module, "Invariant", predicate.as_ref().clone(), &[]);
                    result.invariants.push((name.clone(), predicate));
                }
                conjunct if !is_temporal(conjunct) => {
                    let predicate = add(module, "Initial", conjunct.clone(), &[]);
                    result.initial_properties.push((name.clone(), predicate));
                }
                conjunct => {
                    let template = template::translate(&original, module, conjunct, &[])?;
                    result.properties.push((name.clone(), template));
                }
            }
        }
    }
    Ok(result)
}

/// Adds a definition over the bound names in scope and returns its name.
pub(crate) fn add(module: &mut Module, label: &str, body: Expr, parameters: &[String]) -> String {
    let mut index = module.definitions.len();
    let name = loop {
        let name = format!("__rewrite{label}{index}");
        if !module.definitions.iter().any(|d| d.name == name) {
            break name;
        }
        index += 1;
    };
    module.definitions.push(Definition {
        name: name.clone(),
        parameters: parameters.to_vec(),
        arities: vec![0; parameters.len()],
        body,
        ..Definition::default()
    });
    name
}

/// Conjuncts, looking through references to zero-parameter definitions.
fn flatten(module: &Module, expression: &Expr, output: &mut Vec<Expr>) -> Result<(), Error> {
    match expression {
        Expr::Binary(op, left, right) if op == "/\\" => {
            flatten(module, left, output)?;
            flatten(module, right, output)
        }
        Expr::Name(_) | Expr::Call(..) => {
            let expanded = expand(module, expression)?;
            if expanded == *expression {
                output.push(expanded);
                Ok(())
            } else {
                flatten(module, &expanded, output)
            }
        }
        _ => {
            output.push(expression.clone());
            Ok(())
        }
    }
}

/// The body of a referenced definition with its arguments substituted, or the
/// expression itself.
pub(crate) fn expand(module: &Module, expression: &Expr) -> Result<Expr, Error> {
    let mut expression = expression.clone();
    let mut seen = BTreeSet::new();
    loop {
        let (name, args) = match &expression {
            Expr::Name(name) => (name, &[][..]),
            Expr::Call(name, args) => (name, args.as_slice()),
            _ => return Ok(expression),
        };
        let Some(definition) = definition(module, name) else {
            return Ok(expression);
        };
        if definition.standard || definition.parameters.len() != args.len() {
            return Ok(expression);
        }
        if !seen.insert(name.clone()) {
            return Err(Error(format!("recursive operator {name}")));
        }
        let replacements = definition
            .parameters
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect();
        expression = tlaplus_syntax::substitute(&definition.body, &replacements)
            .map_err(|e| Error(e.to_string()))?;
    }
}

pub(crate) fn definition<'a>(module: &'a Module, name: &str) -> Option<&'a Definition> {
    module.definitions.iter().find(|d| d.name == name)
}

/// Whether a formula has temporal level, looking through operator definitions.
pub(crate) fn temporal(module: &Module, expression: &Expr, active: &mut BTreeSet<String>) -> bool {
    let mut nested = |e: &Expr| temporal(module, e, active);
    match expression {
        Expr::Unary(op, child) => matches!(op.as_str(), "[]" | "<>") || nested(child),
        Expr::Binary(op, left, right) => op == "~>" || nested(left) || nested(right),
        Expr::Call(name, _) if matches!(name.as_str(), "WF_" | "SF_") => true,
        Expr::Call(name, args) => args.iter().any(&mut nested) || reference(module, name, active),
        Expr::Name(name) => reference(module, name, active),
        Expr::Bound {
            kind, domain, body, ..
        } => {
            matches!(kind.as_str(), "\\EE" | "\\AA")
                || domain.as_deref().is_some_and(&mut nested)
                || nested(body)
        }
        Expr::If(a, b, c) => nested(a) || nested(b) || nested(c),
        Expr::Let(definitions, body) => definitions.iter().any(|d| nested(&d.body)) || nested(body),
        Expr::Set(values) | Expr::Tuple(values) => values.iter().any(nested),
        Expr::Record(fields) => fields.iter().any(|(_, value)| nested(value)),
        Expr::Index(a, b) => nested(a) || nested(b),
        Expr::Stutter(..) | Expr::Integer(_) | Expr::String(_) => false,
    }
}

fn reference(module: &Module, name: &str, active: &mut BTreeSet<String>) -> bool {
    let Some(definition) = definition(module, name) else {
        return false;
    };
    if definition.standard || !active.insert(name.into()) {
        return false;
    }
    let result = temporal(module, &definition.body, active);
    active.remove(name);
    result
}
