mod bags;
mod text;
pub use bags::{BagOp, bag};
use text::sequence_operand;
pub use text::{
    BooleanContext, JsonMode, anti_function, apply, boolean, contains, contextual_error,
    faithful_error_text, function_domain_error, integer_comparison, json, stringify,
    subsequence_range, validate_method_args,
};

pub use tla_values::Value;
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<tla_values::Error> for Error {
    fn from(error: tla_values::Error) -> Self {
        Self(error.to_string())
    }
}
use tla_ir::{Builtin, Literal};

pub fn apply_builtin(op: Builtin, args: Vec<Value>) -> Result<Value, Error> {
    use Builtin::*;
    if matches!(op, BagAdd | BagDifference | BagSubset | BagUnion) {
        return Ok(crate::bag(
            match op {
                BagAdd => crate::BagOp::Add,
                BagDifference => crate::BagOp::Difference,
                BagSubset => crate::BagOp::Subset,
                _ => crate::BagOp::Union,
            },
            &args,
        )?);
    }
    let arity = |n| {
        if args.len() == n {
            Ok(())
        } else {
            Err(Error(format!("{op:?} expects {n} operands")))
        }
    };
    if op == JavaTime {
        arity(0)?;
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        return Ok(Value::Int((seconds & 0x7fff_ffff) as i32));
    }
    if op == RecordSet {
        let fields = args
            .iter()
            .map(|pair| {
                let pair = pair.sequence()?;
                if pair.len() != 2 {
                    return Err(Error("invalid record set field".into()));
                }
                let Value::String(atom) = &pair[0] else {
                    return Err(Error("record field must be a string".into()));
                };
                Ok((atom.clone(), tla_value_expr::Expr::atom(pair[1].clone())))
            })
            .collect::<Result<_, Error>>()?;
        return Ok(tla_value_expr::Expr::Records(fields).normalize(100_000)?);
    }
    if matches!(op, Cartesian | Record) {
        if op == Record {
            let mut entries = Vec::new();
            for pair in args {
                let pair = sequence(&pair)?;
                if pair.len() != 2 {
                    return Err(Error("invalid record field".into()));
                }
                entries.push((pair[0].clone(), pair[1].clone()));
            }
            return Ok(Value::function(entries)?);
        }
        let mut tuples = vec![Vec::new()];
        for set in args {
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
        return Ok(Value::set(tuples.into_iter().map(Value::tuple).collect())?);
    }
    if matches!(op, Print | PrintT) {
        arity(if op == PrintT { 1 } else { 2 })?;
        return Ok(if op == PrintT {
            Value::Bool(true)
        } else {
            args[args.len() - 1].clone()
        });
    }
    if matches!(op, Set | Tuple) {
        return Ok(if op == Set {
            Value::set(args)?
        } else {
            Value::tuple(args)
        });
    }
    let unary = matches!(
        op,
        Eval | NoOp
            | ModelValue
            | AntiFunction
            | Not
            | Neg
            | Domain
            | Cardinality
            | Len
            | Head
            | Tail
            | UnionAll
            | PowerSet
            | IsInjective
            | IsFiniteSet
            | Permutations
            | IsABag
            | BagToSet
            | SetToBag
            | BagCardinality
    );
    arity(if unary {
        1
    } else if matches!(op, Subsequence) {
        3
    } else {
        2
    })?;
    let a = &args[0];
    let b = args.get(1);
    let second = || b.ok_or_else(|| Error("missing operand".into()));
    let bool_value = |value| Ok(Value::Bool(value));
    let integer = |value: Option<i32>| {
        value
            .map(Value::Int)
            .ok_or_else(|| Error("integer overflow or invalid arithmetic".into()))
    };
    match op {
        Eval | NoOp => Ok(a.clone()),
        ModelValue => match a {
            Value::String(atom) => Ok(Value::Model(atom.clone())),
            _ => Err(Error("TLCModelValue expects a string".into())),
        },
        Assert if matches!(a, Value::Bool(true)) => Ok(Value::Bool(true)),
        Assert => Err(Error(format!(
            "assertion failed: {}",
            stringify(&tla_value_expr::Expr::Canonical(second()?.clone()))?
        ))),
        Destructure | DestructureExact => {
            let actual = a.sequence()?.len();
            let expected = usize::try_from(second()?.integer()?)
                .map_err(|_| Error("negative tuple pattern length".into()))?;
            if actual < expected || (op == DestructureExact && actual != expected) {
                return Err(Error("tuple pattern arity mismatch".into()));
            }
            Ok(a.clone())
        }
        FunctionSingleton => Ok(Value::function(vec![(a.clone(), second()?.clone())])?),
        FunctionOverride => {
            let (Value::Function(left), Value::Function(right)) = (a, second()?) else {
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
                        return bool_value(false);
                    }
                }
            }
            bool_value(true)
        }
        AntiFunction => crate::anti_function(a),
        IsFiniteSet => {
            a.elements()?;
            bool_value(true)
        }
        SetToBag => Ok(Value::function(
            a.elements()?
                .iter()
                .map(|key| (key.clone(), Value::Int(1)))
                .collect(),
        )?),
        IsABag => bool_value(
            matches!(a,Value::Function(entries) if entries.iter().all(|(_,value)| matches!(value,Value::Int(n) if *n > 0))),
        ),
        BagToSet | BagCardinality => {
            let Value::Function(entries) = a else {
                return Err(Error("expected a bag".into()));
            };
            let mut count = 0i32;
            for (_, value) in entries.iter() {
                let n = value.integer()?;
                if n <= 0 {
                    return Err(Error("bag counts must be positive".into()));
                }
                count = count
                    .checked_add(n)
                    .ok_or_else(|| Error("bag size overflow".into()))?;
            }
            if op == BagCardinality {
                Ok(Value::Int(count))
            } else {
                Ok(Value::set(
                    entries.iter().map(|(key, _)| key.clone()).collect(),
                )?)
            }
        }
        BagIn | CopiesIn => {
            let Value::Function(entries) = second()? else {
                return Err(Error("expected a bag".into()));
            };
            let count = entries
                .iter()
                .find(|(key, _)| key == a)
                .map_or(Ok(0), |(_, v)| v.integer())?;
            if op == BagIn {
                bool_value(count > 0)
            } else {
                Ok(Value::Int(count))
            }
        }
        Permutations => {
            let values = a.elements()?;
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
        Not => bool_value(!boolean(a, BooleanContext::Not, None)?),
        Neg => integer(a.integer()?.checked_neg()),
        Equivalent => bool_value(a.boolean()? == second()?.boolean()?),
        Eq | Ne => bool_value(
            tla_value_expr::Expr::atom(a.clone())
                .semantic_eq(&tla_value_expr::Expr::atom(second()?.clone()))?
                == (op == Eq),
        ),
        Lt | Le | Gt | Ge | NaturalsGe => bool_value(integer_comparison(
            op,
            &tla_value_expr::Expr::atom(a.clone()),
            &tla_value_expr::Expr::atom(second()?.clone()),
        )?),
        Add | Sub | Mul | Div | Mod | Pow => {
            let (x, y) = (a.integer()?, second()?.integer()?);
            if op == Div && y == 0 {
                return Err(Error("division by zero".into()));
            }
            if op == Mod && y <= 0 {
                return Err(Error(format!("non-positive modulus: {y}")));
            }
            integer(match op {
                Add => x.checked_add(y),
                Sub => x.checked_sub(y),
                Mul => x.checked_mul(y),
                Div => x.checked_div(y).and_then(|q| {
                    let r = x.checked_rem(y)?;
                    if r != 0 && (r < 0) != (y < 0) {
                        q.checked_sub(1)
                    } else {
                        Some(q)
                    }
                }),
                Mod if y > 0 => x.checked_rem_euclid(y),
                Pow if x != 0 || y != 0 => u32::try_from(y).ok().and_then(|y| x.checked_pow(y)),
                _ => None,
            })
        }
        In => bool_value(second()?.contains(a)?),
        NotIn => bool_value(!second()?.contains(a)?),
        SubsetEq => {
            for x in a.elements()? {
                if !second()?.contains(x)? {
                    return bool_value(false);
                }
            }
            bool_value(true)
        }
        Union | Intersection | Difference => {
            let other = second()?;
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
            (a.integer()?..=second()?.integer()?)
                .map(Value::Int)
                .collect(),
        )?),
        Apply => Ok(a.apply(second()?)?),
        Domain => {
            let Value::Function(entries) = a else {
                return Err(Error("DOMAIN expects a function".into()));
            };
            Ok(Value::set(
                entries.iter().map(|(k, _)| k.clone()).collect(),
            )?)
        }
        Cardinality => integer(i32::try_from(a.cardinality()?).ok()),
        Len => integer(i32::try_from(sequence_operand(a, "Len")?.len()).ok()),
        Head => sequence_operand(a, "Head")?
            .into_iter()
            .next()
            .ok_or_else(|| Error("Head of empty sequence".into())),
        Tail => {
            let sequence = sequence_operand(a, "Tail")?;
            if sequence.is_empty() {
                return Err(Error("Tail of empty sequence".into()));
            }
            Ok(Value::tuple(sequence.into_iter().skip(1).collect()))
        }
        Append | Concat => {
            let mut values = if op == Append {
                sequence_operand(a, "Append")?
            } else {
                sequence(a)?
            };
            if op == Append {
                values.push(second()?.clone());
            } else {
                values.extend(sequence(second()?)?);
            }
            Ok(Value::tuple(values))
        }
        Subsequence => {
            let values = sequence_operand(a, "SubSeq")?;
            let range = subsequence_range(a, second()?, &args[2], values.len())?;
            Ok(Value::tuple(values[range].to_vec()))
        }
        UnionAll => {
            let mut values = Vec::new();
            for set in a.elements()? {
                values.extend_from_slice(set.elements()?);
            }
            Ok(Value::set(values)?)
        }
        PowerSet => {
            let elements = a.elements()?;
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
        FunctionSet => {
            let domain = a.elements()?;
            let range = second()?.elements()?;
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
        _ => Err(Error(format!("unsupported builtin {op:?}"))),
    }
}

fn sequence(value: &Value) -> Result<Vec<Value>, Error> {
    let Value::Function(entries) = value else {
        return Err(Error("expected a sequence".into()));
    };
    if entries
        .iter()
        .enumerate()
        .any(|(i, (k, _))| *k != Value::Int(i as i32 + 1))
    {
        return Err(Error("function domain is not a sequence interval".into()));
    }
    Ok(entries.iter().map(|(_, v)| v.clone()).collect())
}

pub fn literal(value: &Value) -> Literal {
    match value {
        Value::Bool(x) => Literal::Bool(*x),
        Value::Int(x) => Literal::Int(*x),
        Value::String(atom) => Literal::String {
            ordinal: atom.ordinal,
            text: atom.text.to_string(),
        },
        Value::Model(atom) => Literal::Model {
            ordinal: atom.ordinal,
            text: atom.text.to_string(),
        },
        Value::Set(values) => Literal::Set(values.iter().map(literal).collect()),
        Value::Function(entries) => Literal::Function(
            entries
                .iter()
                .map(|(k, v)| (literal(k), literal(v)))
                .collect(),
        ),
    }
}

pub fn materialize(literal: &Literal) -> Result<Value, Error> {
    Ok(match literal {
        Literal::Bool(x) => Value::Bool(*x),
        Literal::Int(x) => Value::Int(*x),
        Literal::String { ordinal, text } => Value::string(text.as_str(), *ordinal),
        Literal::Model { ordinal, text } => Value::model(text.as_str(), *ordinal),
        Literal::Set(xs) => Value::set(xs.iter().map(materialize).collect::<Result<_, _>>()?)?,
        Literal::Tuple(xs) => Value::tuple(xs.iter().map(materialize).collect::<Result<_, _>>()?),
        Literal::Function(xs) => Value::function(
            xs.iter()
                .map(|(k, v)| Ok((materialize(k)?, materialize(v)?)))
                .collect::<Result<_, Error>>()?,
        )?,
    })
}

pub fn runtime_literal(literal: &Literal) -> Result<tla_value_expr::Expr, Error> {
    use tla_value_expr::{Expr, FunctionKind};
    Ok(match literal {
        Literal::Set(values) => Expr::Set(
            values
                .iter()
                .map(runtime_literal)
                .collect::<Result<_, _>>()?,
        ),
        Literal::Tuple(values) => Expr::tuple(
            values
                .iter()
                .map(runtime_literal)
                .collect::<Result<_, _>>()?,
        ),
        Literal::Function(entries) => Expr::Function {
            kind: FunctionKind::Explicit,
            entries: entries
                .iter()
                .map(|(k, v)| Ok((crate::materialize(k)?, runtime_literal(v)?)))
                .collect::<Result<_, Error>>()?,
        },
        _ => Expr::atom(crate::materialize(literal)?),
    })
}
