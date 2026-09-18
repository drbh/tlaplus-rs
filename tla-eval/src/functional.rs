use crate::deferred::runtime;
use crate::sets::{applied, member};
use crate::{Error, Evaluator, Value};
use tla_ir::{Builtin, Builtin as B, Expr as Ir, ExprId, Literal};
use tla_value_expr::Expr;
pub(crate) const LIMIT: usize = 100_000;
impl Evaluator<'_> {
    pub fn enumerate_with_locals(
        &self,
        domain: ExprId,
        locals: &[Option<Value>],
    ) -> Result<Vec<Value>, Error> {
        enumerate(self, domain, &mut locals.to_vec(), 0)
    }
    pub fn contains_with_locals(
        &self,
        domain: ExprId,
        value: Value,
        locals: &[Option<Value>],
    ) -> Result<bool, Error> {
        member(self, domain, &Expr::atom(value), &mut locals.to_vec(), 0)
    }
    pub fn apply_with_locals(
        &self,
        function: ExprId,
        argument: Value,
        locals: &[Option<Value>],
    ) -> Result<Value, Error> {
        Ok(applied(self, function, &Expr::atom(argument), locals, 0)?.normalize(LIMIT)?)
    }
}
pub(crate) fn enumerate(
    evaluator: &Evaluator<'_>,
    domain: ExprId,
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Vec<Value>, Error> {
    let domain = runtime(evaluator, domain, locals, depth + 1)?;
    if matches!(domain, Expr::Infinite(_) | Expr::Sequences(_)) {
        return Err(Error(format!(
            "non-enumerable domain: {}",
            tla_library::stringify(&domain)?
        )));
    }
    let values = domain
        .enumerate(usize::MAX)?
        .into_iter()
        .map(|value| value.normalize(usize::MAX).map_err(Error::from))
        .collect::<Result<_, _>>()?;
    Ok(Value::set(values)?.elements()?.to_vec())
}

pub(crate) fn invoke(
    evaluator: &crate::Evaluator<'_>,
    function: tla_ir::ExprId,
    values: Vec<Value>,
    locals: &[Option<Value>],
    depth: usize,
) -> Result<Value, Error> {
    let tla_ir::Expr::Lambda { parameters, body } = &evaluator.program.nodes[function].expr else {
        return Err(Error("expected operator argument".into()));
    };
    if parameters.len() != values.len() {
        return Err(Error("operator argument arity mismatch".into()));
    }
    let mut nested = locals.to_vec();
    for (slot, value) in parameters.iter().zip(values) {
        crate::bind(&mut nested, *slot, value);
    }
    evaluator.eval(*body, &mut nested, depth + 1)
}

pub(crate) fn sequence_operator(
    evaluator: &crate::Evaluator<'_>,
    expression: &tla_ir::Expr,
    args: &[tla_ir::ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Value, Error> {
    if args.len() != 2 {
        return Err(Error("sequence operator expects two arguments".into()));
    }
    let input = evaluator.eval(args[0], locals, depth + 1)?.sequence()?;
    if matches!(expression, Ir::Apply { op: B::SortSeq, .. })
        && !matches!(evaluator.program.nodes[args[1]].expr, Ir::Lambda { .. })
    {
        let value = runtime(evaluator, args[1], locals, depth + 1)?;
        return Err(Error(format!(
            "The second argument of SortSeq should be a operator, but instead it is:\n{}",
            tla_library::stringify(&value)?
        )));
    }
    let mut output = Vec::new();
    for value in input {
        if matches!(
            expression,
            tla_ir::Expr::Apply {
                op: Builtin::SelectSeq,
                ..
            }
        ) {
            if invoke(evaluator, args[1], vec![value.clone()], locals, depth + 1)?.boolean()? {
                output.push(value);
            }
        } else {
            let mut index = output.len();
            while index > 0
                && invoke(
                    evaluator,
                    args[1],
                    vec![value.clone(), output[index - 1].clone()],
                    locals,
                    depth + 1,
                )?
                .boolean()?
            {
                index -= 1;
            }
            output.insert(index, value);
        }
    }
    Ok(Value::tuple(output))
}
pub(crate) fn projected(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    locals: &[Option<Value>],
    depth: usize,
) -> Result<(ExprId, Vec<Option<Value>>), Error> {
    if depth >= evaluator.recursion_limit {
        return Err(Error("recursion limit exceeded".into()));
    }
    let mut nested = locals.to_vec();
    let node = evaluator
        .program
        .nodes
        .get(id)
        .ok_or_else(|| Error("invalid expression".into()))?;
    let next = match &node.expr {
        Ir::Scoped { current, .. } => *current,
        Ir::If {
            condition,
            then_expr,
            else_expr,
        } => {
            if evaluator
                .eval(*condition, &mut nested, depth + 1)?
                .boolean()?
            {
                *then_expr
            } else {
                *else_expr
            }
        }
        Ir::Call { operator, args } => {
            let op = evaluator
                .program
                .operators
                .get(*operator)
                .ok_or_else(|| Error("invalid operator".into()))?;
            if op.parameters.len() != args.len() {
                return Err(Error("operator arity mismatch".into()));
            }
            for (slot, arg) in op.parameters.iter().zip(args) {
                let value = evaluator.eval(*arg, &mut locals.to_vec(), depth + 1)?;
                bind(&mut nested, *slot, value);
            }
            op.body
        }
        Ir::Apply { op: B::Apply, args } if args.len() == 2 => {
            let (function, mut captures) = projected(evaluator, args[0], locals, depth + 1)?;
            let (Ir::Function { slot, domain, body }
            | Ir::RecursiveFunction { slot, domain, body }) =
                &evaluator.program.nodes[function].expr
            else {
                return Ok((id, nested));
            };
            let argument = runtime(evaluator, args[1], &mut nested, depth + 1)?;
            if let Some(domain) = domain
                && !runtime(evaluator, *domain, &mut captures, depth + 1)?.contains(&argument)?
            {
                let value = runtime(evaluator, function, &mut captures, depth + 1)?;
                return Err(Error(tla_library::function_domain_error(
                    &value, &argument,
                )?));
            }
            bind(&mut captures, *slot, argument.normalize(LIMIT)?);
            return projected(evaluator, *body, &captures, depth + 1);
        }
        _ => return Ok((id, nested)),
    };
    projected(evaluator, next, &nested, depth + 1)
}

pub fn materialize(literal: &Literal) -> Result<Value, Error> {
    tla_library::materialize(literal)
}

pub(crate) fn deferred_operator(
    evaluator: &Evaluator<'_>,
    op: B,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Expr, Error> {
    let arity = if op == B::Print { 2 } else { 1 };
    if args.len() != arity {
        return Err(Error(format!("{op:?} expects {arity} operands")));
    }
    let mut values = args
        .iter()
        .map(|id| runtime(evaluator, *id, locals, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    if op == B::PowerSet {
        return Ok(Expr::Subsets {
            base: Box::new(values.remove(0)),
            size: None,
        });
    }
    tla_library::stringify(&values[0])?;
    Ok(if op == B::PrintT {
        Expr::Canonical(Value::Bool(true))
    } else {
        values.remove(1)
    })
}

pub(crate) fn assert_error(
    evaluator: &Evaluator<'_>,
    message: Option<&str>,
    body: ExprId,
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Value, Error> {
    let expected =
        message.ok_or_else(|| Error("AssertError requires a string literal message".into()))?;
    let Err(error) = evaluator.eval(body, locals, depth + 1) else {
        return Ok(Value::Bool(false));
    };
    let actual = tla_library::faithful_error_text(&error.0).ok_or_else(|| {
        Error(format!(
            "AssertError cannot compare this error text: {error}"
        ))
    })?;
    Ok(Value::Bool(expected == actual))
}
pub(crate) fn sample(
    evaluator: &Evaluator<'_>,
    op: B,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Value, Error> {
    let (ids, argument) = match (op, args) {
        (B::RandomElement, [set]) => (&args[..0], *set),
        (B::RandomSubset, [_, set]) => (&args[..1], *set),
        (B::RandomSetOfSubsets, [_, _, set]) => (&args[..2], *set),
        _ => return Err(Error(format!("{op:?} operand count mismatch"))),
    };
    let mut counts = Vec::new();
    for id in ids {
        counts.push(evaluator.eval(*id, locals, depth + 1)?);
    }
    let set = runtime(evaluator, argument, locals, depth + 1)?;
    let mut random = evaluator
        .random
        .ok_or_else(|| Error(format!("{op:?} requires a random session")))?
        .lock()
        .map_err(|_| Error("random session poisoned".into()))?;
    Ok(random.sample(&counts, &set, LIMIT)?)
}

impl Evaluator<'_> {
    pub(crate) fn string(&self, text: String) -> Result<Value, Error> {
        tla_runtime::intern_string(&self.strings, text, self.intern_new_strings).map_err(Error)
    }
}

pub fn bind(locals: &mut Vec<Option<Value>>, slot: usize, value: Value) {
    if locals.len() <= slot {
        locals.resize(slot + 1, None);
    }
    locals[slot] = Some(value);
}
