//! The TLA+ standard modules and TLC's built-in operators over finite values.
//! [`apply_builtin`] checks an operator's arity and hands it to the module for
//! its family: arithmetic and logic, sets, functions and records, sequences,
//! bags, and TLC's own operators.
mod arithmetic;
mod bags;
mod functions;
mod sequences;
mod sets;
mod text;
mod tlc;
pub use bags::{BagOp, bag};
/// Rendering, boolean coercion and error formatting shared with the
/// evaluator, so that messages match TLC's wording in both places.
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

/// Applies a built-in operator to already evaluated operands.
pub fn apply_builtin(op: Builtin, args: Vec<Value>) -> Result<Value, Error> {
    use Builtin::*;
    match op {
        BagAdd | BagDifference | BagSubset | BagUnion => Ok(bags::bag(bags::op(op), &args)?),
        Set => Ok(Value::set(args)?),
        Tuple => Ok(Value::tuple(args)),
        Cartesian => sets::cartesian(args),
        Record => functions::record(args),
        RecordSet => functions::record_set(&args),
        Print | PrintT | JavaTime => tlc::effect(op, args),
        _ => {
            let Some(expected) = arity(op) else {
                return Err(Error(format!("unsupported builtin {op:?}")));
            };
            if args.len() != expected {
                return Err(Error(format!("{op:?} expects {expected} operands")));
            }
            match op {
                Eval | NoOp | ModelValue | Assert | Destructure | DestructureExact => {
                    tlc::apply(op, &args)
                }
                Not | Neg | Equivalent | Eq | Ne | Lt | Le | Gt | Ge | NaturalsGe | Add | Sub
                | Mul | Div | Mod | Pow => arithmetic::apply(op, &args),
                In | NotIn | SubsetEq | Union | Intersection | Difference | Range | Cardinality
                | UnionAll | PowerSet | IsFiniteSet | Permutations => sets::apply(op, &args),
                Apply | Domain | FunctionSingleton | FunctionOverride | IsInjective
                | AntiFunction | FunctionSet => functions::apply(op, &args),
                Len | Head | Tail | Append | Concat | Subsequence => sequences::apply(op, &args),
                SetToBag | IsABag | BagToSet | BagCardinality | BagIn | CopiesIn => {
                    bags::apply(op, &args)
                }
                _ => Err(Error(format!("unsupported builtin {op:?}"))),
            }
        }
    }
}

/// How many operands a fixed-arity operator takes, or `None` for a builtin
/// this library does not apply.
fn arity(op: Builtin) -> Option<usize> {
    use Builtin::*;
    Some(match op {
        Eval | NoOp | ModelValue | AntiFunction | Not | Neg | Domain | Cardinality | Len | Head
        | Tail | UnionAll | PowerSet | IsInjective | IsFiniteSet | Permutations | IsABag
        | BagToSet | SetToBag | BagCardinality => 1,
        Subsequence => 3,
        Assert | Destructure | DestructureExact | Equivalent | Eq | Ne | Lt | Le | Gt | Ge
        | NaturalsGe | Add | Sub | Mul | Div | Mod | Pow | In | NotIn | SubsetEq | Union
        | Intersection | Difference | Range | Apply | FunctionSingleton | FunctionOverride
        | FunctionSet | Append | Concat | BagIn | CopiesIn => 2,
        _ => return None,
    })
}

fn integer(value: Option<i32>) -> Result<Value, Error> {
    value
        .map(Value::Int)
        .ok_or_else(|| Error("integer overflow or invalid arithmetic".into()))
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
