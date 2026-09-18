use crate::deferred::runtime;
use crate::functional::{LIMIT, projected};
use crate::{Error, Evaluator, Value, bind};
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
            let mut nested = locals.clone();
            bind(&mut nested, *slot, argument.normalize(LIMIT)?);
            runtime(evaluator, *body, &mut nested, depth + 1)
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
                let mut nested = locals.clone();
                bind(&mut nested, *slot, value.normalize(LIMIT)?);
                Ok(evaluator.eval(*body, &mut nested, depth + 1)?.boolean()?)
            }
            Ir::Call { operator, args } => {
                let (body, mut nested) = enter(evaluator, *operator, args, locals, depth)?;
                member(evaluator, body, value, &mut nested, depth + 1)
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

/// An operator's body and the locals binding its parameters to evaluated arguments.
pub(crate) fn enter(
    evaluator: &Evaluator<'_>,
    operator: usize,
    args: &[ExprId],
    locals: &[Option<Value>],
    depth: usize,
) -> Result<(ExprId, Vec<Option<Value>>), Error> {
    let op = evaluator
        .program
        .operators
        .get(operator)
        .ok_or_else(|| Error("invalid operator".into()))?;
    if op.parameters.len() != args.len() {
        return Err(Error("operator arity mismatch".into()));
    }
    let mut nested = locals.to_vec();
    for (slot, arg) in op.parameters.iter().zip(args) {
        let value = evaluator.eval(*arg, &mut locals.to_vec(), depth + 1)?;
        bind(&mut nested, *slot, value);
    }
    Ok((op.body, nested))
}
