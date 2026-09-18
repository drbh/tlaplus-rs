//! Finite set operators, including the enumerations TLC materializes:
//! Cartesian products, power sets and permutations.
use crate::{Error, integer};
use tla_ir::Builtin;
use tla_values::Value;

pub(crate) fn apply(op: Builtin, args: &[Value]) -> Result<Value, Error> {
    use Builtin::*;
    let a = &args[0];
    let b = || &args[1];
    match op {
        In => Ok(Value::Bool(b().contains(a)?)),
        NotIn => Ok(Value::Bool(!b().contains(a)?)),
        SubsetEq => {
            for x in a.elements()? {
                if !b().contains(x)? {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
        Union | Intersection | Difference => {
            let other = b();
            let mut values = Vec::new();
            for x in a.elements()? {
                if op == Union || other.contains(x)? == (op == Intersection) {
                    values.push(x.clone());
                }
            }
            if op == Union {
                values.extend_from_slice(other.elements()?);
            }
            Ok(Value::set(values)?)
        }
        Range => Ok(Value::set(
            (a.integer()?..=b().integer()?).map(Value::Int).collect(),
        )?),
        Cardinality => integer(i32::try_from(a.cardinality()?).ok()),
        IsFiniteSet => {
            a.elements()?;
            Ok(Value::Bool(true))
        }
        UnionAll => {
            let mut values = Vec::new();
            for set in a.elements()? {
                values.extend_from_slice(set.elements()?);
            }
            Ok(Value::set(values)?)
        }
        PowerSet => power_set(a.elements()?),
        Permutations => permutations(a.elements()?),
        _ => Err(Error(format!("{op:?} is not a set operator"))),
    }
}

fn power_set(elements: &[Value]) -> Result<Value, Error> {
    let mut sets = vec![Vec::new()];
    for element in elements {
        let mut with = sets.clone();
        for set in &mut with {
            set.push(element.clone());
        }
        sets.extend(with);
    }
    Ok(Value::set(
        sets.into_iter().map(Value::set).collect::<Result<_, _>>()?,
    )?)
}

/// `Permutations(S)`: every bijection of `S` onto itself, as functions.
fn permutations(values: &[Value]) -> Result<Value, Error> {
    let mut permutations = vec![Vec::new()];
    for _ in values {
        let mut next = Vec::new();
        for prefix in permutations {
            for value in values {
                if !prefix.contains(value) {
                    let mut p = prefix.clone();
                    p.push(value.clone());
                    next.push(p);
                }
            }
        }
        permutations = next;
    }
    Ok(Value::set(
        permutations
            .into_iter()
            .map(|p: Vec<Value>| Value::function(values.iter().cloned().zip(p).collect()))
            .collect::<Result<_, _>>()?,
    )?)
}

/// `S1 \X S2 \X ...` over any number of sets.
pub(crate) fn cartesian(sets: Vec<Value>) -> Result<Value, Error> {
    let mut tuples = vec![Vec::new()];
    for set in sets {
        let elements = set.elements()?;
        tuples = tuples
            .into_iter()
            .flat_map(|prefix| {
                elements.iter().map(move |v| {
                    let mut tuple = prefix.clone();
                    tuple.push(v.clone());
                    tuple
                })
            })
            .collect();
    }
    Ok(Value::set(tuples.into_iter().map(Value::tuple).collect())?)
}
