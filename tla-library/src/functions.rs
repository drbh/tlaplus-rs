//! Functions and records: application, domains, construction and the
//! function-set enumeration `[S -> T]`.
use crate::{Error, anti_function, sequences::sequence};
use tlaplus_ir::Builtin;
use tlaplus_value_expr::Expr;
use tlaplus_values::Value;

pub(crate) fn apply(op: Builtin, args: &[Value]) -> Result<Value, Error> {
    use Builtin::*;
    let a = &args[0];
    let b = || &args[1];
    match op {
        Apply => Ok(a.apply(b())?),
        Domain => {
            let Value::Function(entries) = a else {
                return Err(Error("DOMAIN expects a function".into()));
            };
            Ok(Value::set(
                entries.iter().map(|(k, _)| k.clone()).collect(),
            )?)
        }
        FunctionSingleton => Ok(Value::function(vec![(a.clone(), b().clone())])?),
        FunctionOverride => {
            let (Value::Function(left), Value::Function(right)) = (a, b()) else {
                return Err(Error("function override requires functions".into()));
            };
            let mut entries = left.to_vec();
            for (key, value) in right.iter() {
                if !left.iter().any(|(k, _)| k == key) {
                    entries.push((key.clone(), value.clone()));
                }
            }
            Ok(Value::function(entries)?)
        }
        IsInjective => {
            let Value::Function(entries) = a else {
                return Err(Error("IsInjective expects a function".into()));
            };
            for (index, (_, value)) in entries.iter().enumerate() {
                for (_, other) in &entries[index + 1..] {
                    if value.semantic_eq(other)? {
                        return Ok(Value::Bool(false));
                    }
                }
            }
            Ok(Value::Bool(true))
        }
        AntiFunction => anti_function(a),
        FunctionSet => function_set(a.elements()?, b().elements()?),
        _ => Err(Error(format!("{op:?} is not a function operator"))),
    }
}

/// `[S -> T]`: every function from `domain` into `range`.
fn function_set(domain: &[Value], range: &[Value]) -> Result<Value, Error> {
    let mut functions = vec![Vec::new()];
    for key in domain {
        functions = functions
            .into_iter()
            .flat_map(|entries| {
                range.iter().map(move |v| {
                    let mut result = entries.clone();
                    result.push((key.clone(), v.clone()));
                    result
                })
            })
            .collect();
    }
    Ok(Value::set(
        functions
            .into_iter()
            .map(Value::function)
            .collect::<Result<_, _>>()?,
    )?)
}

/// `[f1 |-> v1, ...]` from `<<field, value>>` pairs.
pub(crate) fn record(pairs: Vec<Value>) -> Result<Value, Error> {
    let mut entries = Vec::new();
    for pair in pairs {
        let pair = sequence(&pair)?;
        if pair.len() != 2 {
            return Err(Error("invalid record field".into()));
        }
        entries.push((pair[0].clone(), pair[1].clone()));
    }
    Ok(Value::function(entries)?)
}

/// `[f1 : S1, ...]` from `<<field, set>>` pairs, enumerated as a set of records.
pub(crate) fn record_set(pairs: &[Value]) -> Result<Value, Error> {
    let fields = pairs
        .iter()
        .map(|pair| {
            let pair = pair.sequence()?;
            if pair.len() != 2 {
                return Err(Error("invalid record set field".into()));
            }
            let Value::String(atom) = &pair[0] else {
                return Err(Error("record field must be a string".into()));
            };
            Ok((atom.clone(), Expr::atom(pair[1].clone())))
        })
        .collect::<Result<_, Error>>()?;
    Ok(Expr::Records(fields).normalize(100_000)?)
}
