//! TLC's own operators: evaluation hooks, model values, assertions, tuple
//! patterns, printing and the clock.
use crate::{Error, stringify};
use tlaplus_ir::Builtin;
use tlaplus_value_expr::Expr;
use tlaplus_values::Value;

/// `Print`, `PrintT` and `JavaTime`, which take their operands as given.
pub(crate) fn effect(op: Builtin, args: Vec<Value>) -> Result<Value, Error> {
    let expected = match op {
        Builtin::PrintT => 1,
        Builtin::Print => 2,
        _ => 0,
    };
    if args.len() != expected {
        return Err(Error(format!("{op:?} expects {expected} operands")));
    }
    Ok(match op {
        Builtin::PrintT => Value::Bool(true),
        Builtin::Print => args[args.len() - 1].clone(),
        _ => {
            let seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            Value::Int((seconds & 0x7fff_ffff) as i32)
        }
    })
}

pub(crate) fn apply(op: Builtin, args: &[Value]) -> Result<Value, Error> {
    use Builtin::*;
    let a = &args[0];
    match op {
        ModelValue => match a {
            Value::String(atom) => Ok(Value::Model(atom.clone())),
            _ => Err(Error("TLCModelValue expects a string".into())),
        },
        Assert if matches!(a, Value::Bool(true)) => Ok(Value::Bool(true)),
        Assert => Err(Error(format!(
            "assertion failed: {}",
            stringify(&Expr::Canonical(args[1].clone()))?
        ))),
        Destructure | DestructureExact => {
            let actual = a.sequence()?.len();
            let expected = usize::try_from(args[1].integer()?)
                .map_err(|_| Error("negative tuple pattern length".into()))?;
            if actual < expected || (op == DestructureExact && actual != expected) {
                return Err(Error("tuple pattern arity mismatch".into()));
            }
            Ok(a.clone())
        }
        Eval | NoOp => Ok(a.clone()),
        _ => Err(Error(format!("{op:?} is not a TLC operator"))),
    }
}
