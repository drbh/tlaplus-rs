//! Logic, comparison and integer arithmetic.
use crate::{BooleanContext, Error, boolean, integer, integer_comparison};
use tla_ir::Builtin;
use tla_value_expr::Expr;
use tla_values::Value;

pub(crate) fn apply(op: Builtin, args: &[Value]) -> Result<Value, Error> {
    use Builtin::*;
    let a = &args[0];
    let b = || &args[1];
    match op {
        Not => Ok(Value::Bool(!boolean(a, BooleanContext::Not, None)?)),
        Neg => integer(a.integer()?.checked_neg()),
        Equivalent => Ok(Value::Bool(a.boolean()? == b().boolean()?)),
        Eq | Ne => Ok(Value::Bool(
            Expr::atom(a.clone()).semantic_eq(&Expr::atom(b().clone()))? == (op == Eq),
        )),
        Lt | Le | Gt | Ge | NaturalsGe => Ok(Value::Bool(integer_comparison(
            op,
            &Expr::atom(a.clone()),
            &Expr::atom(b().clone()),
        )?)),
        Add | Sub | Mul | Div | Mod | Pow => binary(op, a.integer()?, b().integer()?),
        _ => Err(Error(format!("{op:?} is not an arithmetic operator"))),
    }
}

/// `+ - * \div % ^` with TLC's checks: division by zero, non-positive
/// modulus, and overflow are errors; `\div` rounds toward negative infinity.
fn binary(op: Builtin, x: i32, y: i32) -> Result<Value, Error> {
    use Builtin::*;
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
