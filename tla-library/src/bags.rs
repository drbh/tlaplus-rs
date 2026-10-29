//! The `Bags` module: multisets as functions into positive counts.
use tlaplus_ir::Builtin;
use tlaplus_values::{Error, Value};

/// The `Bags` operator a builtin stands for.
pub(crate) fn op(builtin: Builtin) -> BagOp {
    match builtin {
        Builtin::BagAdd => BagOp::Add,
        Builtin::BagDifference => BagOp::Difference,
        Builtin::BagSubset => BagOp::Subset,
        Builtin::BagUnion => BagOp::Union,
        other => unreachable!("{other:?} is not a Bags operator"),
    }
}

/// The unary and binary bag operators: tests, conversions and counts.
pub(crate) fn apply(builtin: Builtin, args: &[Value]) -> Result<Value, crate::Error> {
    use Builtin::*;
    let a = &args[0];
    let bag = |value: &Value| match value {
        Value::Function(entries) => Ok(entries.clone()),
        _ => Err(crate::Error("expected a bag".into())),
    };
    match builtin {
        SetToBag => Ok(Value::function(
            a.elements()?
                .iter()
                .map(|key| (key.clone(), Value::Int(1)))
                .collect(),
        )?),
        IsABag => Ok(Value::Bool(matches!(
            a,
            Value::Function(entries)
                if entries.iter().all(|(_, value)| matches!(value, Value::Int(n) if *n > 0))
        ))),
        BagToSet | BagCardinality => {
            let entries = bag(a)?;
            let mut count = 0i32;
            for (_, value) in entries.iter() {
                let n = value.integer()?;
                if n <= 0 {
                    return Err(crate::Error("bag counts must be positive".into()));
                }
                count = count
                    .checked_add(n)
                    .ok_or_else(|| crate::Error("bag size overflow".into()))?;
            }
            if builtin == BagCardinality {
                Ok(Value::Int(count))
            } else {
                Ok(Value::set(
                    entries.iter().map(|(key, _)| key.clone()).collect(),
                )?)
            }
        }
        BagIn | CopiesIn => {
            let entries = bag(&args[1])?;
            let count = entries
                .iter()
                .find(|(key, _)| key == a)
                .map_or(Ok(0), |(_, v)| v.integer())?;
            if builtin == BagIn {
                Ok(Value::Bool(count > 0))
            } else {
                Ok(Value::Int(count))
            }
        }
        _ => Err(crate::Error(format!("{builtin:?} is not a bag operator"))),
    }
}
#[derive(Clone, Copy, Debug)]
pub enum BagOp {
    Add,
    Difference,
    Subset,
    Union,
}
pub fn bag(op: BagOp, args: &[Value]) -> Result<Value, Error> {
    let arity = if matches!(op, BagOp::Union) { 1 } else { 2 };
    if args.len() != arity {
        return Err(Error::Type(match op {
            BagOp::Add => "(+) expects two bags",
            BagOp::Difference => "(-) expects two bags",
            BagOp::Subset => "\\sqsubseteq expects two bags",
            BagOp::Union => "BagUnion expects one set of bags",
        }));
    }
    if matches!(op, BagOp::Union) {
        let values = args[0].elements()?;
        let Some(first) = values.first() else {
            return Value::function(vec![]);
        };
        if values.len() == 1 {
            return Ok(first.clone());
        }
        let mut entries = first.entries()?.to_vec();
        for value in &values[1..] {
            merge(&mut entries, value.entries()?)?;
        }
        return Value::function(entries);
    }
    let a = args[0].entries()?;
    let b = args[1].entries()?;
    // `(+)` builds a bag from both operands, so both must be bags. `(-)` and
    // `\\sqsubseteq` only read counts, and TLC applies them to any functions
    // into integers, so they are not checked.
    if matches!(op, BagOp::Add) {
        if a.iter()
            .chain(b)
            .any(|(_, v)| !matches!(v,Value::Int(n) if *n>0))
        {
            return Err(Error::Type("bag with positive counts"));
        }
        let mut entries = a.to_vec();
        merge(&mut entries, b)?;
        return Value::function(entries);
    }
    // How many of `key` remain after removing `b`'s copies.
    let remaining = |key: &Value, count: &Value| -> Result<i32, Error> {
        let mut n = count.integer()?;
        for (other, taken) in b {
            if key.semantic_eq(other)? {
                n = n.checked_sub(taken.integer()?).ok_or(Error::Overflow)?;
                break;
            }
        }
        Ok(n)
    };
    if matches!(op, BagOp::Subset) {
        for (key, count) in a {
            if remaining(key, count)? > 0 {
                return Ok(Value::Bool(false));
            }
        }
        return Ok(Value::Bool(true));
    }
    let mut result = Vec::new();
    for (key, count) in a {
        let n = remaining(key, count)?;
        if n > 0 {
            result.push((key.clone(), Value::Int(n)));
        }
    }
    Value::function(result)
}
fn merge(entries: &mut Vec<(Value, Value)>, other: &[(Value, Value)]) -> Result<(), Error> {
    for (key, value) in other {
        let mut found = false;
        for (old_key, old_value) in entries.iter_mut() {
            if key.semantic_eq(old_key)? {
                *old_value = Value::Int(
                    old_value
                        .integer()?
                        .checked_add(value.integer()?)
                        .ok_or(Error::Overflow)?,
                );
                found = true;
                break;
            }
        }
        if !found {
            entries.push((key.clone(), value.clone()));
        }
    }
    Ok(())
}
