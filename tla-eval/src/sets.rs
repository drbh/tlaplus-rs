use crate::deferred::runtime;
use crate::functional::with_binding;
use crate::functional::{LIMIT, projected, with_bindings};
use crate::{Error, Evaluator, Value};
use tla_ir::{Builtin as B, Expr as Ir, ExprId};
use tla_value_expr::Expr;

/// The domain expression of a function constructor `[x \in D |-> e]`.
fn function_domain(evaluator: &Evaluator<'_>, args: &[ExprId]) -> Option<ExprId> {
    match (
        args,
        args.first().map(|f| &evaluator.program.nodes[*f].expr),
    ) {
        ([_], Some(Ir::Function { domain, .. } | Ir::RecursiveFunction { domain, .. })) => *domain,
        _ => None,
    }
}

pub(crate) fn domain(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    locals: &[Option<Value>],
    depth: usize,
) -> Result<Value, Error> {
    let (id, mut locals) = projected(evaluator, id, locals, depth + 1)?;
    match &evaluator.program.nodes[id].expr {
        Ir::Function { domain, .. } | Ir::RecursiveFunction { domain, .. } => {
            let domain = domain.ok_or_else(|| Error("function value requires a domain".into()))?;
            Ok(runtime(evaluator, domain, &mut locals, depth + 1)?.normalize(LIMIT)?)
        }
        _ => Ok(runtime(evaluator, id, &mut locals, depth + 1)?
            .domain()?
            .normalize(LIMIT)?),
    }
}

pub(crate) fn applied(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    argument: &Expr,
    locals: &[Option<Value>],
    depth: usize,
) -> Result<Expr, Error> {
    stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
        applied_inner(evaluator, id, argument, locals, depth)
    })
}
fn applied_inner(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    argument: &Expr,
    locals: &[Option<Value>],
    depth: usize,
) -> Result<Expr, Error> {
    if depth >= evaluator.recursion_limit {
        return Err(Error("recursion limit exceeded".into()));
    }
    // A function held in a variable, constant or local, possibly behind
    // `ENABLED` scoping, needs no projection of the environment: apply its
    // value directly, as the generic arm would.
    let mut target = id;
    while let Some(Ir::Scoped { current, .. }) =
        evaluator.program.nodes.get(target).map(|n| &n.expr)
    {
        target = *current;
    }
    let direct = match &evaluator.program.nodes.get(target).map(|n| &n.expr) {
        Some(Ir::Local(slot)) => locals.get(*slot).cloned().flatten(),
        Some(Ir::Variable { slot, primed }) => {
            let state = if *primed {
                evaluator.next
            } else {
                evaluator.state
            };
            state.get(*slot).cloned().flatten()
        }
        Some(Ir::Constant(slot)) => evaluator.constants.get(*slot).cloned(),
        _ => None,
    };
    if let Some(function) = direct {
        return tla_library::apply(&Expr::atom(function), argument);
    }
    let (id, mut captures) = projected(evaluator, id, locals, depth + 1)?;
    let locals = &mut captures;
    let node = evaluator
        .program
        .nodes
        .get(id)
        .ok_or_else(|| Error("invalid expression".into()))?;
    match &node.expr {
        Ir::Scoped { current, .. } => applied(evaluator, *current, argument, locals, depth + 1),
        Ir::Function { slot, domain, body } | Ir::RecursiveFunction { slot, domain, body } => {
            if let Some(domain) = domain
                && !runtime(evaluator, *domain, locals, depth + 1)?.contains(argument)?
            {
                let value = runtime(evaluator, id, locals, depth + 1)?;
                return Err(Error(tla_library::function_domain_error(&value, argument)?));
            }
            let argument = argument.normalize(LIMIT)?;
            with_binding(locals, *slot, argument, |locals| {
                runtime(evaluator, *body, locals, depth + 1)
            })
        }
        Ir::Apply {
            op: B::Except,
            args,
        } => {
            let (&base, updates) = args
                .split_first()
                .ok_or_else(|| Error("EXCEPT without base".into()))?;
            let mut changed = Vec::<(Value, Value)>::new();
            for update in updates {
                let Ir::Apply { op: B::Tuple, args } = &evaluator.program.nodes[*update].expr
                else {
                    return Err(Error("invalid EXCEPT update".into()));
                };
                if args.len() != 2 {
                    return Err(Error("invalid EXCEPT update arity".into()));
                }
                let path = evaluator.eval(args[0], locals, depth + 1)?.sequence()?;
                let Some((key, tail)) = path.split_first() else {
                    return Err(Error("EXCEPT requires a path".into()));
                };
                let original = if let Some((_, v)) = changed.iter().find(|(k, _)| k == key) {
                    v.clone()
                } else {
                    let (base, mut scope) = projected(evaluator, base, locals, depth + 1)?;
                    let key = Expr::atom(key.clone());
                    let contains = match &evaluator.program.nodes[base].expr {
                        Ir::Function {
                            domain: Some(domain),
                            ..
                        }
                        | Ir::RecursiveFunction {
                            domain: Some(domain),
                            ..
                        } => member(evaluator, *domain, &key, &mut scope, depth + 1)?,
                        _ => runtime(evaluator, base, &mut scope, depth + 1)?
                            .domain()?
                            .contains(&key)?,
                    };
                    if !contains {
                        continue;
                    }
                    applied(evaluator, base, &key, &scope, depth + 1)?.normalize(LIMIT)?
                };
                let mut previous = original.clone();
                let mut valid = true;
                for key in tail {
                    match previous.apply(key) {
                        Ok(v) => previous = v,
                        Err(_) => {
                            valid = false;
                            break;
                        }
                    }
                }
                if !valid {
                    continue;
                }
                let nested = Evaluator {
                    at: Some(previous),
                    ..evaluator.clone()
                };
                let replacement = nested.eval(args[1], locals, depth + 1)?;
                let updated = original.except(tail, replacement)?;
                if let Some((_, v)) = changed.iter_mut().find(|(k, _)| k == key) {
                    *v = updated;
                } else {
                    changed.push((key.clone(), updated));
                }
            }
            let key = argument.normalize(LIMIT)?;
            if let Some((_, value)) = changed.into_iter().find(|(k, _)| *k == key) {
                Ok(Expr::atom(value))
            } else {
                applied(evaluator, base, argument, locals, depth + 1)
            }
        }
        _ => tla_library::apply(&runtime(evaluator, id, locals, depth + 1)?, argument),
    }
}

pub(crate) fn member(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    value: &Expr,
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<bool, Error> {
    stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
        if depth >= evaluator.recursion_limit {
            return Err(Error("recursion limit exceeded".into()));
        }
        match &evaluator
            .program
            .nodes
            .get(id)
            .ok_or_else(|| Error("invalid expression".into()))?
            .expr
        {
            Ir::Quantify {
                kind: tla_ir::Quantifier::SetFilter,
                slot,
                domain,
                body,
            } => {
                if !member(
                    evaluator,
                    domain.ok_or_else(|| Error("unbounded filter domain".into()))?,
                    value,
                    locals,
                    depth + 1,
                )? {
                    return Ok(false);
                }
                let value = value.normalize(LIMIT)?;
                Ok(with_binding(locals, *slot, value, |locals| {
                    evaluator.eval(*body, locals, depth + 1)
                })?
                .boolean()?)
            }
            Ir::Call { operator, args } => {
                entered(evaluator, *operator, args, locals, depth, |body, locals| {
                    member(evaluator, body, value, locals, depth + 1)
                })?
            }
            Ir::If {
                condition,
                then_expr,
                else_expr,
            } => {
                let yes = evaluator.if_guard(id, evaluator.eval(*condition, locals, depth + 1)?)?;
                let branch = if yes { *then_expr } else { *else_expr };
                member(evaluator, branch, value, locals, depth + 1)
            }
            // Membership in the domain of `[x \in D |-> e]` need not enumerate D.
            Ir::Apply {
                op: B::Domain,
                args,
            } if function_domain(evaluator, args).is_some() => {
                let domain = function_domain(evaluator, args).unwrap_or_default();
                member(evaluator, domain, value, locals, depth + 1)
            }
            // Like TLC, `f \in [D -> R]` checks each value's membership in R, which
            // need not be enumerable (e.g. `{n \in Nat : n > 0}`).
            Ir::Apply {
                op: B::FunctionSet,
                args,
            } if args.len() == 2 && matches!(value.normalize(LIMIT), Ok(Value::Function(_))) => {
                let function = value.normalize(LIMIT)?;
                let domain = runtime(evaluator, args[0], locals, depth + 1)?;
                if !domain.semantic_eq(&Expr::atom(function.domain()?))? {
                    return Ok(false);
                }
                for (_, image) in function.entries()? {
                    let image = Expr::atom(image.clone());
                    if !member(evaluator, args[1], &image, locals, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ir::Apply { op, args }
                if args.len() == 2 && matches!(op, B::Union | B::Intersection | B::Difference) =>
            {
                let first = member(evaluator, args[0], value, locals, depth + 1)?;
                Ok(match op {
                    B::Union => first || member(evaluator, args[1], value, locals, depth + 1)?,
                    B::Intersection => {
                        first && member(evaluator, args[1], value, locals, depth + 1)?
                    }
                    _ => first && !member(evaluator, args[1], value, locals, depth + 1)?,
                })
            }
            _ => tla_library::contains(&runtime(evaluator, id, locals, depth + 1)?, value),
        }
    })
}

/// Runs `body` on an operator's body with its parameters bound to the
/// evaluated arguments, then restores the environment. Arguments are
/// evaluated in the caller's environment before any parameter is bound.
pub(crate) fn entered<R>(
    evaluator: &Evaluator<'_>,
    operator: usize,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
    body: impl FnOnce(ExprId, &mut Vec<Option<Value>>) -> R,
) -> Result<R, Error> {
    let op = evaluator
        .program
        .operators
        .get(operator)
        .ok_or_else(|| Error("invalid operator".into()))?;
    if op.parameters.len() != args.len() {
        return Err(Error("operator arity mismatch".into()));
    }
    let values = args
        .iter()
        .map(|arg| evaluator.eval(*arg, locals, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    let bindings = op.parameters.iter().copied().zip(values).collect();
    Ok(with_bindings(locals, bindings, |locals| {
        body(op.body, locals)
    }))
}
