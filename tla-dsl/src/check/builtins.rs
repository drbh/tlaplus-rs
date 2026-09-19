//! Typed collection primitives. No implicit conversion between maps and sequences.
use super::expressions::{infer, unify};
use super::*;
pub(super) fn is_builtin(name: &str) -> bool {
    "len head tail append concat slice size contains union intersect difference subset powerset domain put sequences sequences_up_to functions product product3 entry merge".split_whitespace().any(|n| n == name)
}
pub(super) fn infer_call(
    at: Location,
    name: &str,
    args: &[Expr],
    env: &Environment,
) -> Result<(Type, u8), Error> {
    let values = args
        .iter()
        .map(|a| infer(a, env))
        .collect::<Result<Vec<_>, _>>()?;
    let level = values.iter().map(|(_, l)| *l).max().unwrap_or(0);
    if level > 2 {
        return Err(error(
            at,
            "collection operations cannot consume temporal formulas",
        ));
    }
    let types = values.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>();
    let int = Type::Int;
    let result = match (name, types.as_slice()) {
        ("len", [Type::Seq(_)]) | ("size", [Type::Set(_)]) => Some(int),
        ("head", [Type::Seq(t)]) => Some(*t.clone()),
        ("tail", [Type::Seq(t)]) => Some(Type::Seq(t.clone())),
        ("append", [Type::Seq(t), v]) => unify(t, v).map(|t| Type::Seq(Box::new(t))),
        ("concat", [Type::Seq(a), Type::Seq(b)]) => unify(a, b).map(|t| Type::Seq(Box::new(t))),
        ("slice", [Type::Seq(t), Type::Int, Type::Int]) => Some(Type::Seq(t.clone())),
        ("contains", [Type::Set(t), v]) => unify(t, v).map(|_| Type::Bool),
        ("union" | "intersect" | "difference", [Type::Set(a), Type::Set(b)]) => {
            unify(a, b).map(|t| Type::Set(Box::new(t)))
        }
        ("subset", [Type::Set(a), Type::Set(b)]) => unify(a, b).map(|_| Type::Bool),
        ("powerset", [t @ Type::Set(_)]) => Some(Type::Set(Box::new(t.clone()))),
        ("domain", [Type::Seq(_)]) => Some(Type::Set(Box::new(Type::Int))),
        ("domain", [Type::Map(k, _)]) => Some(Type::Set(k.clone())),
        ("put", [Type::Map(k, v), key, value]) => {
            unify(k, key).and_then(|k| unify(v, value).map(|v| Type::Map(Box::new(k), Box::new(v))))
        }
        ("put", [Type::Seq(v), Type::Int, value]) => {
            unify(v, value).map(|v| Type::Seq(Box::new(v)))
        }
        ("sequences" | "sequences_up_to", [Type::Set(t), Type::Int]) => {
            Some(Type::Set(Box::new(Type::Seq(t.clone()))))
        }
        ("functions", [Type::Set(k), Type::Set(v)]) => {
            Some(Type::Set(Box::new(Type::Map(k.clone(), v.clone()))))
        }
        ("product", [Type::Set(a), Type::Set(b)]) => Some(Type::Set(Box::new(Type::Tuple(vec![
            *a.clone(),
            *b.clone(),
        ])))),
        ("product3", [Type::Set(a), Type::Set(b), Type::Set(c)]) => {
            Some(Type::Set(Box::new(Type::Tuple(vec![
                *a.clone(),
                *b.clone(),
                *c.clone(),
            ]))))
        }
        ("entry", [k, v]) if !has_function(k) && !has_function(v) => {
            Some(Type::Map(Box::new(k.clone()), Box::new(v.clone())))
        }
        ("merge", [a @ Type::Map(..), b @ Type::Map(..)]) => unify(a, b),
        _ => None,
    };
    result
        .map(|t| (t, level))
        .ok_or_else(|| error(at, format!("invalid arguments for {name}: {types:?}")))
}
