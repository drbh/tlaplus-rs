//! Expression types, empty-collection unification, and state/temporal levels.
use super::*;
pub(super) fn unify(a: &Type, b: &Type) -> Option<Type> {
    Some(match (a, b) {
        (Type::Unknown, t) | (t, Type::Unknown) => t.clone(),
        (Type::Tuple(a), Type::Tuple(b)) if a.len() == b.len() => Type::Tuple(
            a.iter()
                .zip(b)
                .map(|(a, b)| unify(a, b))
                .collect::<Option<Vec<_>>>()?,
        ),
        (Type::Tuple(items), Type::Seq(t)) | (Type::Seq(t), Type::Tuple(items)) => Type::Seq(
            Box::new(items.iter().try_fold(*t.clone(), |a, b| unify(&a, b))?),
        ),
        (Type::Set(a), Type::Set(b)) => Type::Set(Box::new(unify(a, b)?)),
        (Type::Seq(a), Type::Seq(b)) => Type::Seq(Box::new(unify(a, b)?)),
        (Type::Map(ak, av), Type::Map(bk, bv)) => {
            Type::Map(Box::new(unify(ak, bk)?), Box::new(unify(av, bv)?))
        }
        _ if a == b => a.clone(),
        _ => return None,
    })
}
pub(super) fn expect(
    expr: &Expr,
    env: &Environment,
    expected: &Type,
    maximum: u8,
) -> Result<(), Error> {
    let (ty, level) = infer(expr, env)?;
    let base = crate::refinements::erase(expected);
    if unify(&ty, &base).is_none() {
        return Err(error(
            expr.at,
            format!("expected {expected:?}, found {ty:?}"),
        ));
    }
    if level > maximum {
        return Err(error(
            expr.at,
            match level {
                3 => "temporal expressions are only allowed in properties",
                2 => "after(...) is only allowed in transition properties",
                _ => "initializers must be constant expressions",
            },
        ));
    }
    Ok(())
}
pub(super) fn infer(expr: &Expr, env: &Environment) -> Result<(Type, u8), Error> {
    if expr.at.line != 0
        && matches!(&expr.kind, Kind::Name(n) | Kind::Call(n, _) if n.starts_with('_'))
    {
        return Err(error(expr.at, "compiler-generated names are private"));
    }
    match &expr.kind {
        Kind::Record(name, fields) => super::records::infer_record(name, fields, env, expr.at),
        Kind::Field(value, field) => super::records::infer_field(value, field, env),
        Kind::TypeTest(ty, value) | Kind::TotalTest(ty, value) => {
            expect(value, env, ty, 2)?;
            Ok((Type::Bool, infer(value, env)?.1))
        }
        Kind::Int(value) => {
            value.parse::<i32>().map_err(|_| {
                error(
                    expr.at,
                    "integer literal exceeds the prototype's signed 32-bit backend range",
                )
            })?;
            Ok((Type::Int, 0))
        }
        Kind::String(_) => Ok((Type::String, 0)),
        Kind::Bool(_) => Ok((Type::Bool, 0)),
        Kind::Name(name) => env.get(name).cloned().ok_or_else(|| {
            error(
                expr.at,
                format!(
                    "unknown value {name:?}; unbounded refinement choices need an explicit domain"
                ),
            )
        }),
        Kind::Unary(op, body) => unary(op, body, env),
        Kind::Binary(op, a, b) => binary(op, a, b, env),
        Kind::Tuple(items) => {
            let types = items
                .iter()
                .map(|i| infer(i, env))
                .collect::<Result<Vec<_>, _>>()?;
            let level = types.iter().map(|(_, l)| *l).max().unwrap_or(0);
            if level > 2 || types.iter().any(|(t, _)| has_function(t)) {
                return Err(error(expr.at, "tuples must contain values"));
            }
            Ok((
                Type::Tuple(types.into_iter().map(|(t, _)| t).collect()),
                level,
            ))
        }
        Kind::Collection(set, items) => {
            let mut ty = Type::Unknown;
            let mut level = 0;
            for item in items {
                let (next, l) = infer(item, env)?;
                if l > 2 || has_function(&next) {
                    return Err(error(item.at, "collections must contain values"));
                }
                ty = unify(&ty, &next)
                    .ok_or_else(|| error(item.at, "collection elements must have the same type"))?;
                level = level.max(l);
            }
            Ok((
                if *set {
                    Type::Set(Box::new(ty))
                } else {
                    Type::Seq(Box::new(ty))
                },
                level,
            ))
        }
        Kind::Index(base, key) => {
            let (ty, level) = infer(base, env)?;
            let (key_ty, value_ty) = match ty {
                Type::Seq(v) => (Type::Int, *v),
                Type::Tuple(items) => {
                    let Kind::Int(index) = &key.kind else {
                        return Err(error(
                            key.at,
                            "heterogeneous tuple indices must be integer literals",
                        ));
                    };
                    let index = index
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|n| items.get(n))
                        .ok_or_else(|| error(key.at, "tuple index out of range"))?;
                    (Type::Int, index.clone())
                }
                Type::Map(k, v) => (*k, *v),
                _ => return Err(error(base.at, "indexing requires a sequence or map")),
            };
            expect(key, env, &key_ty, 2)?;
            Ok((value_ty, level.max(infer(key, env)?.1)))
        }
        Kind::Call(name, args) => call(expr.at, name, args, env),
        Kind::If(condition, yes, no) => {
            expect(condition, env, &Type::Bool, 2)?;
            let (a, al) = infer(yes, env)?;
            let (b, bl) = infer(no, env)?;
            if (infer(condition, env)?.1 == 2 && al.max(bl) == 3)
                || (al == 2 && bl == 3)
                || (al == 3 && bl == 2)
            {
                return Err(error(expr.at, "cannot mix action and temporal branches"));
            }
            if has_function(&a) {
                return Err(error(expr.at, "if branches must return values"));
            }
            let ty = unify(&a, &b)
                .ok_or_else(|| error(expr.at, "if branches must have the same type"))?;
            Ok((ty, al.max(bl).max(infer(condition, env)?.1)))
        }
        Kind::Binder(op, name, domain, body) => binder(op, name, domain, body, env),
        Kind::Lambda(name, ty, body) => {
            let mut local = env.clone();
            let mut names = env.keys().cloned().collect();
            declare(name, &mut names)?;
            if has_function(ty) {
                return Err(error(name.at, "lambda parameters must be values"));
            }
            let enums = env
                .values()
                .filter_map(|(t, _)| {
                    if let Type::Set(t) = t
                        && let Type::Enum(n) = t.as_ref()
                    {
                        Some(n.clone())
                    } else {
                        None
                    }
                })
                .collect();
            check_type(ty, &enums, name.at)?;
            local.insert(name.text.clone(), (crate::refinements::erase(ty), 0));
            let (result, level) = infer(body, &local)?;
            if level != 0 || has_function(&result) {
                return Err(error(body.at, "lambda must be pure and return a value"));
            }
            Ok((Type::Function(vec![ty.clone()], Box::new(result)), 0))
        }
    }
}
fn unary(op: &str, body: &Expr, env: &Environment) -> Result<(Type, u8), Error> {
    let (ty, level) = infer(body, env)?;
    if op == "after" {
        if level != 1 {
            return Err(error(
                body.at,
                "after requires a state expression without nested after",
            ));
        }
        return Ok((ty, 2));
    }
    let expected = if op == "-" { Type::Int } else { Type::Bool };
    if ty != expected {
        return Err(error(
            body.at,
            format!("{op} expects {expected:?}, found {ty:?}"),
        ));
    }
    if matches!(op, "always" | "eventually") {
        if level == 2 {
            return Err(error(
                body.at,
                "use transition ... over (...) for action properties",
            ));
        }
        Ok((ty, 3))
    } else {
        Ok((ty, level))
    }
}
fn binary(op: &str, a: &Expr, b: &Expr, env: &Environment) -> Result<(Type, u8), Error> {
    let (at, al) = infer(a, env)?;
    let (bt, bl) = infer(b, env)?;
    if op == "is" {
        let Type::Set(element) = bt else {
            return Err(error(b.at, "right side of is must be a value set"));
        };
        if unify(&at, &element).is_none() || al.max(bl) > 2 || has_function(&at) {
            return Err(error(a.at, "incompatible value type or level for is"));
        }
        return Ok((Type::Bool, al.max(bl)));
    }
    let boolean = matches!(op, "&&" | "||" | "=>");
    let equality = matches!(op, "==" | "!=");
    if unify(&at, &bt).is_none()
        || (boolean && at != Type::Bool)
        || (!boolean && !equality && at != Type::Int)
        || has_function(&at)
    {
        return Err(error(
            a.at,
            format!("incompatible operands for {op}: {at:?} and {bt:?}"),
        ));
    }
    let level = al.max(bl);
    if (al == 2 && bl == 3) || (al == 3 && bl == 2) {
        return Err(error(
            a.at,
            "action predicates must be wrapped in a transition property, not mixed with temporal formulas",
        ));
    }
    if !boolean && level == 3 {
        return Err(error(
            a.at,
            "temporal formulas can only be combined with Boolean operators",
        ));
    }
    let ty = if op == "..=" {
        Type::Set(Box::new(Type::Int))
    } else if matches!(op, "+" | "-" | "*" | "%") {
        Type::Int
    } else {
        Type::Bool
    };
    Ok((ty, level))
}
fn binder(
    op: &str,
    name: &Name,
    domain: &Expr,
    body: &Expr,
    env: &Environment,
) -> Result<(Type, u8), Error> {
    let (domain_ty, dl) = infer(domain, env)?;
    let Type::Set(element) = domain_ty else {
        return Err(error(domain.at, "binder domain must be a set"));
    };
    if dl > 1 {
        return Err(error(
            domain.at,
            "binder domain must be constant or current-state",
        ));
    }
    let mut names = env.keys().cloned().collect();
    declare(name, &mut names)?;
    let mut local = env.clone();
    local.insert(name.text.clone(), (*element.clone(), 0));
    let (ty, level) = infer(body, &local)?;
    if has_function(&ty) {
        return Err(error(body.at, "binder body must return values"));
    }
    let result = match op {
        "forall" | "exists" | "filter" | "choose" => {
            if ty != Type::Bool {
                return Err(error(body.at, "predicate must be Bool"));
            }
            if level > 1 && !matches!(op, "forall" | "exists") {
                return Err(error(
                    body.at,
                    "filter and choose predicates cannot be action or temporal formulas",
                ));
            }
            if level == 3 && dl != 0 {
                return Err(error(
                    domain.at,
                    "temporal quantifiers require constant domains",
                ));
            }
            match op {
                "filter" => Type::Set(element),
                "choose" => *element,
                _ => Type::Bool,
            }
        }
        "map" => Type::Map(element, Box::new(ty)),
        "sequence" => {
            if *element != Type::Int {
                return Err(error(domain.at, "sequence indices must be integers"));
            }
            // A sequence comprehension has an explicit one-based range, not an arbitrary set.
            if !matches!(&domain.kind, Kind::Binary(op,lo,_) if op == "..=" && matches!(&lo.kind,Kind::Int(n) if n == "1"))
            {
                return Err(error(
                    domain.at,
                    "sequence domain must have the form 1..=length",
                ));
            }
            Type::Seq(Box::new(ty))
        }
        "set_of" => Type::Set(Box::new(ty)),
        _ => unreachable!(),
    };
    if level == 3 && !matches!(op, "forall" | "exists") {
        return Err(error(
            body.at,
            "collection comprehensions cannot be temporal",
        ));
    }
    Ok((result, dl.max(level)))
}
fn call(at: Location, name: &str, args: &[Expr], env: &Environment) -> Result<(Type, u8), Error> {
    if builtins::is_builtin(name) {
        return builtins::infer_call(at, name, args, env);
    }
    let Some((Type::Function(params, result), function_level)) = env.get(name) else {
        return Err(error(at, format!("unknown function {name:?}")));
    };
    if params.len() != args.len() {
        return Err(error(
            at,
            format!("{name} expects {} arguments", params.len()),
        ));
    }
    let mut level = *function_level;
    for (arg, ty) in args.iter().zip(params) {
        expect(arg, env, ty, 2)?;
        level = level.max(infer(arg, env)?.1);
    }
    Ok((*result.clone(), level))
}
