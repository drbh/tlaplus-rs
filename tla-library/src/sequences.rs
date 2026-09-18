//! The `Sequences` module over tuples.
use crate::{Error, integer, subsequence_range, text::sequence_operand};
use tla_ir::Builtin;
use tla_values::Value;

pub(crate) fn apply(op: Builtin, args: &[Value]) -> Result<Value, Error> {
    use Builtin::*;
    let a = &args[0];
    match op {
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
        Append => {
            let mut values = sequence_operand(a, "Append")?;
            values.push(args[1].clone());
            Ok(Value::tuple(values))
        }
        Concat => {
            let mut values = sequence(a)?;
            values.extend(sequence(&args[1])?);
            Ok(Value::tuple(values))
        }
        Subsequence => {
            let values = sequence_operand(a, "SubSeq")?;
            let range = subsequence_range(a, &args[1], &args[2], values.len())?;
            Ok(Value::tuple(values[range].to_vec()))
        }
        _ => Err(Error(format!("{op:?} is not a sequence operator"))),
    }
}

/// The values of a tuple, in order.
pub(crate) fn sequence(value: &Value) -> Result<Vec<Value>, Error> {
    let Value::Function(entries) = value else {
        return Err(Error("expected a sequence".into()));
    };
    if entries
        .iter()
        .enumerate()
        .any(|(i, (k, _))| *k != Value::position(i))
    {
        return Err(Error("function domain is not a sequence interval".into()));
    }
    Ok(entries.iter().map(|(_, v)| v.clone()).collect())
}
