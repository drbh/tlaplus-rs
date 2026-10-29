use super::{add, definition, temporal};
use crate::Error;
use std::collections::{BTreeMap, BTreeSet};
use tlaplus_syntax::{Expr, Module};

/// A temporal formula whose predicates are module definitions over the names
/// bound by enclosing quantifiers, in order.
#[derive(Clone, Debug)]
pub enum Template {
    /// A state predicate, or an action predicate of a step.
    Atom(String),
    Not(Box<Template>),
    And(Box<Template>, Box<Template>),
    Or(Box<Template>, Box<Template>),
    Always(Box<Template>),
    Eventually(Box<Template>),
    /// `WF_v(A)` or `SF_v(A)`; the definition is `A /\ v' # v`.
    Fairness {
        strong: bool,
        action: String,
        /// `ENABLED` of the action, as its own definition.
        enabled: String,
    },
    /// `\A x \in S : F` or `\E x \in S : F`; the definition is `S`.
    Quantified {
        universal: bool,
        domain: String,
        body: Box<Template>,
    },
}

use Template as T;

fn boxed(template: Template) -> Box<Template> {
    Box::new(template)
}

pub(crate) fn translate(
    original: &Module,
    module: &mut Module,
    expression: &Expr,
    bound: &[String],
) -> Result<Template, Error> {
    if !temporal(original, expression, &mut BTreeSet::new()) {
        return Ok(T::Atom(add(module, "Atom", expression.clone(), bound)));
    }
    let nested = |e: &Expr, module: &mut Module| translate(original, module, e, bound);
    Ok(match expression {
        Expr::Unary(op, child) => {
            let child = boxed(nested(child, module)?);
            match op.as_str() {
                "[]" => T::Always(child),
                "<>" => T::Eventually(child),
                "~" => T::Not(child),
                _ => return Err(Error(format!("unsupported temporal operator {op}"))),
            }
        }
        Expr::Binary(op, left, right) => {
            let left = nested(left, module)?;
            let right = nested(right, module)?;
            let implies = |a: &Template, b: &Template| {
                T::Or(boxed(T::Not(boxed(a.clone()))), boxed(b.clone()))
            };
            match op.as_str() {
                "/\\" => T::And(boxed(left), boxed(right)),
                "\\/" => T::Or(boxed(left), boxed(right)),
                "=>" => implies(&left, &right),
                "<=>" => T::And(boxed(implies(&left, &right)), boxed(implies(&right, &left))),
                "~>" => T::Always(boxed(implies(&left, &T::Eventually(boxed(right))))),
                _ => return Err(Error(format!("unsupported temporal connective {op}"))),
            }
        }
        Expr::Call(name, args) if matches!(name.as_str(), "WF_" | "SF_") => {
            let [variables, action] = args.as_slice() else {
                return Err(Error("fairness expects a subscript and an action".into()));
            };
            let changed = Expr::Unary(
                "~".into(),
                Box::new(Expr::Unary("UNCHANGED".into(), Box::new(variables.clone()))),
            );
            let taken = Expr::Binary("/\\".into(), Box::new(action.clone()), Box::new(changed));
            let enabled = Expr::Unary("ENABLED".into(), Box::new(taken.clone()));
            T::Fairness {
                strong: name == "SF_",
                action: add(module, "Fairness", taken, bound),
                enabled: add(module, "Enabled", enabled, bound),
            }
        }
        Expr::Name(name) | Expr::Call(name, _) => {
            let args = match expression {
                Expr::Call(_, args) => args.as_slice(),
                _ => &[],
            };
            let definition = definition(original, name)
                .ok_or_else(|| Error(format!("unresolved temporal operator {name}")))?;
            if definition.recursive {
                return Err(Error(format!(
                    "recursive temporal operator {name} is unsupported"
                )));
            }
            if definition.parameters.len() != args.len() {
                return Err(Error(format!("wrong number of arguments for {name}")));
            }
            let replacements: BTreeMap<_, _> = definition
                .parameters
                .iter()
                .cloned()
                .zip(args.iter().cloned())
                .collect();
            let body = tlaplus_syntax::substitute(&definition.body, &replacements)
                .map_err(|e| Error(e.to_string()))?;
            nested(&body, module)?
        }
        Expr::Bound {
            kind,
            names,
            tuple: false,
            domain: Some(domain),
            body,
        } if matches!(kind.as_str(), "\\A" | "\\E") && names.len() == 1 => {
            let domain = add(module, "Domain", domain.as_ref().clone(), bound);
            let inner: Vec<_> = bound.iter().chain(names).cloned().collect();
            T::Quantified {
                universal: kind == "\\A",
                domain,
                body: boxed(translate(original, module, body, &inner)?),
            }
        }
        Expr::If(condition, yes, no) => {
            let condition = nested(condition, module)?;
            let yes = nested(yes, module)?;
            let no = nested(no, module)?;
            T::Or(
                boxed(T::And(boxed(condition.clone()), boxed(yes))),
                boxed(T::And(boxed(T::Not(boxed(condition))), boxed(no))),
            )
        }
        _ => return Err(Error("unsupported temporal expression".into())),
    })
}
