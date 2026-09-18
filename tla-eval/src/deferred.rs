use crate::sets::{applied, member};
use crate::{Error, Evaluator, Value, bind};
use tla_ir::{Builtin as B, Expr as Ir, ExprId};
use tla_value_expr::{Expr, FunctionKind, InfiniteSet, SetOp};
const LIMIT: usize = 100_000;
pub(crate) fn validate_tuple_binding(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    slot: usize,
    value: &Value,
    exact: bool,
) -> Result<(), Error> {
    let mut pending = vec![id];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = evaluator
            .program
            .nodes
            .get(id)
            .ok_or_else(|| Error("invalid expression".into()))?;
        let children: Vec<ExprId> = match &node.expr {
            Ir::Scoped { current, .. } => vec![*current],
            Ir::Apply {
                op: B::Destructure | B::DestructureExact,
                args,
            } if args.len() == 2 => {
                if matches!(evaluator.program.nodes[args[0]].expr, Ir::Local(local) if local == slot)
                {
                    let expected = evaluator.eval(args[1], &mut Vec::new(), 0)?.integer()?;
                    let actual = value.sequence()?.len();
                    let expected = usize::try_from(expected).unwrap_or(usize::MAX);
                    if actual < expected || (exact && actual != expected) {
                        return Err(Error("tuple pattern arity mismatch".into()));
                    }
                }
                args.clone()
            }
            Ir::Apply { args, .. } => args.clone(),
            Ir::If {
                condition,
                then_expr,
                else_expr,
            } => vec![*condition, *then_expr, *else_expr],
            Ir::Call { args, .. } => args.clone(),
            Ir::Invoke { function, args, .. } => {
                let mut children = vec![*function];
                children.extend(args);
                children
            }
            Ir::Quantify { domain, body, .. }
            | Ir::Function { domain, body, .. }
            | Ir::RecursiveFunction { domain, body, .. }
            | Ir::Choose { domain, body, .. } => domain.iter().copied().chain([*body]).collect(),
            Ir::Except {
                function,
                key,
                value,
            } => vec![*function, *key, *value],
            Ir::Lambda { body, .. } | Ir::AssertError { body, .. } => vec![*body],
            _ => Vec::new(),
        };
        pending.extend(children);
    }
    Ok(())
}
pub(crate) fn runtime(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Expr, Error> {
    stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
        runtime_inner(evaluator, id, locals, depth)
    })
}
fn runtime_inner(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Expr, Error> {
    if depth >= evaluator.recursion_limit {
        return Err(Error("recursion limit exceeded".into()));
    }
    let node = evaluator
        .program
        .nodes
        .get(id)
        .ok_or_else(|| Error("invalid expression".into()))?;
    let evaluate = |id, locals: &mut Vec<Option<Value>>| runtime(evaluator, id, locals, depth + 1);
    Ok(match &node.expr {
        Ir::Scoped { current, .. } => evaluate(*current, locals)?,
        Ir::Literal(literal) => tla_library::runtime_literal(literal)?,
        Ir::If {
            condition,
            then_expr,
            else_expr,
        } => evaluate(
            if evaluator.if_guard(id, evaluator.eval(*condition, locals, depth + 1)?)? {
                *then_expr
            } else {
                *else_expr
            },
            locals,
        )?,
        Ir::Call { operator, args } => {
            let (body, mut nested) = crate::sets::enter(evaluator, *operator, args, locals, depth)?;
            evaluate(body, &mut nested)?
        }
        Ir::Function { slot, domain, body } => {
            let domain = evaluate(
                domain.ok_or_else(|| Error("function value requires a domain".into()))?,
                locals,
            )?;
            let mut entries = Vec::new();
            for value in domain.sorted(LIMIT)? {
                let value = value.normalize(LIMIT)?;
                let mut nested = locals.clone();
                bind(&mut nested, *slot, value.clone());
                entries.push((value, evaluate(*body, &mut nested)?));
            }
            Expr::Function {
                kind: FunctionKind::Lambda,
                entries,
            }
        }
        Ir::RecursiveFunction { .. } => {
            return Err(Error("recursive function requires application".into()));
        }
        Ir::Apply { op, args } => {
            let unary = |locals: &mut Vec<Option<Value>>| -> Result<Expr, Error> {
                if args.len() != 1 {
                    return Err(Error("unary operator arity mismatch".into()));
                }
                evaluate(args[0], locals)
            };
            match op {
                B::Print | B::PrintT => {
                    crate::functional::deferred_operator(evaluator, *op, args, locals, depth)?
                }
                B::Apply if args.len() == 2 => {
                    let argument = evaluate(args[1], locals)?;
                    applied(evaluator, args[0], &argument, locals, depth + 1)?
                }
                B::Any if args.is_empty() => Expr::Infinite(InfiniteSet::Any),
                B::Naturals if args.is_empty() => Expr::Infinite(InfiniteSet::Naturals),
                B::Integers if args.is_empty() => Expr::Infinite(InfiniteSet::Integers),
                B::Strings if args.is_empty() => Expr::Infinite(InfiniteSet::Strings),
                B::Set => Expr::Set(
                    args.iter()
                        .map(|id| evaluate(*id, locals))
                        .collect::<Result<_, _>>()?,
                ),
                B::Tuple => Expr::tuple(
                    args.iter()
                        .map(|id| evaluate(*id, locals))
                        .collect::<Result<_, _>>()?,
                ),
                B::Range if args.len() == 2 => Expr::Interval(
                    evaluator.eval(args[0], locals, depth + 1)?.integer()?,
                    evaluator.eval(args[1], locals, depth + 1)?.integer()?,
                ),
                B::Union | B::Intersection | B::Difference if args.len() == 2 => Expr::Binary(
                    match op {
                        B::Union => SetOp::Union,
                        B::Intersection => SetOp::Intersection,
                        _ => SetOp::Difference,
                    },
                    Box::new(evaluate(args[0], locals)?),
                    Box::new(evaluate(args[1], locals)?),
                ),
                B::PowerSet => Expr::Subsets {
                    base: Box::new(unary(locals)?),
                    size: None,
                },
                B::UnionAll => Expr::Union(Box::new(unary(locals)?)),
                B::Sequences => Expr::Sequences(Box::new(unary(locals)?)),
                B::FunctionSet if args.len() == 2 => Expr::Functions {
                    domain: Box::new(evaluate(args[0], locals)?),
                    range: Box::new(evaluate(args[1], locals)?),
                },
                B::Cartesian => Expr::Product(
                    args.iter()
                        .map(|id| evaluate(*id, locals))
                        .collect::<Result<_, _>>()?,
                ),
                B::Record | B::RecordSet => {
                    let mut entries = Vec::new();
                    for arg in args {
                        let pair = evaluate(*arg, locals)?.sequence()?;
                        if pair.len() != 2 {
                            return Err(Error("record field arity mismatch".into()));
                        }
                        entries.push((pair[0].normalize(LIMIT)?, pair[1].clone()));
                    }
                    if *op == B::RecordSet {
                        Expr::Records(
                            entries
                                .into_iter()
                                .map(|(key, value)| {
                                    let Value::String(atom) = key else {
                                        return Err(Error("record field must be a string".into()));
                                    };
                                    Ok((atom, value))
                                })
                                .collect::<Result<_, Error>>()?,
                        )
                    } else {
                        Expr::Function {
                            kind: FunctionKind::Record,
                            entries,
                        }
                    }
                }
                _ => Expr::atom(evaluator.eval(id, locals, depth + 1)?),
            }
        }
        _ => Expr::atom(evaluator.eval(id, locals, depth + 1)?),
    })
}
/// The builtins the evaluator handles itself rather than through the
/// standard library: TLC's side effects and registers first, then the
/// comparisons and accessors that work on deferred sets and functions
/// without materializing them. Returns `None` when `op` has no such form and
/// must be applied to finite values instead.
pub(crate) fn query(
    evaluator: &Evaluator<'_>,
    op: B,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Option<Value>, Error> {
    let polynomial = evaluator
        .program
        .fingerprint_polynomial
        .unwrap_or(tla_ir::DEFAULT_FINGERPRINT_POLYNOMIAL);
    if op == B::TlcEvalDefinition {
        // The lowering replaces every well-formed use; one that survives has no definition.
        return Err(Error("invalid TLCEvalDefinition".into()));
    }
    if op == B::Memo {
        return memoized(evaluator, args, locals, depth, polynomial).map(Some);
    }
    // Operators whose operands are evaluated lazily so nested deferred forms
    // survive until this point: `SUBSET` of a symbolic set, and `Print`,
    // whose operand must not be forced early. The result is enumerated here.
    if matches!(op, B::PowerSet | B::Print | B::PrintT) {
        return Ok(Some(
            crate::functional::deferred_operator(evaluator, op, args, locals, depth)?
                .normalize(LIMIT)?,
        ));
    }
    if let B::RandomElement | B::RandomSubset | B::RandomSetOfSubsets = op {
        return crate::functional::sample(evaluator, op, args, locals, depth).map(Some);
    }
    if matches!(op, B::TlcSet | B::TlcGetOrDefault | B::TlcGet) {
        return tlc_registers(evaluator, op, args, locals, depth).map(Some);
    }
    if op == B::TlcFingerprint {
        let [argument] = args else {
            return Err(Error("TLCFP expects one operand".into()));
        };
        let value = runtime(evaluator, *argument, locals, depth + 1)?.normalize(LIMIT)?;
        return Ok(Some(Value::Int(value.fingerprint(polynomial))));
    }
    if op == B::JsonSerialize {
        let [filename, value] = args else {
            return Err(Error("JsonSerialize expects two operands".into()));
        };
        if !matches!(
            evaluator.eval(*filename, locals, depth + 1)?,
            Value::String(_)
        ) {
            return Err(Error("JsonSerialize expects a filename string".into()));
        }
        // The value is evaluated for its errors; the checker never writes the file.
        return runtime(evaluator, *value, locals, depth + 1).map(|_| Some(Value::Bool(true)));
    }
    if op == B::Enabled {
        let [action] = args else {
            return Err(Error("ENABLED expects one operand".into()));
        };
        let enabled = evaluator
            .enabled
            .ok_or_else(|| Error("ENABLED requires a model".into()))?;
        return Ok(Some(Value::Bool(enabled(*action, locals)?)));
    }
    if matches!(
        op,
        B::ToString | B::ToJson | B::ToJsonArray | B::ToJsonObject
    ) {
        return text_conversion(evaluator, op, args, locals, depth).map(Some);
    }
    if !matches!(
        op,
        B::Eq
            | B::Ne
            | B::Lt
            | B::Le
            | B::Gt
            | B::Ge
            | B::NaturalsGe
            | B::In
            | B::NotIn
            | B::SubsetEq
            | B::Cardinality
            | B::IsFiniteSet
            | B::Domain
            | B::Apply
    ) {
        return Ok(None);
    }
    let arity = if matches!(op, B::Cardinality | B::IsFiniteSet | B::Domain) {
        1
    } else {
        2
    };
    if args.len() != arity {
        return Err(Error("operator arity mismatch".into()));
    }
    if matches!(op, B::In | B::NotIn) {
        let value = runtime(evaluator, args[0], locals, depth + 1)?;
        return Ok(Some(Value::Bool(
            member(evaluator, args[1], &value, locals, depth + 1)? == (op == B::In),
        )));
    }
    if op == B::Apply {
        let argument = runtime(evaluator, args[1], locals, depth + 1)?;
        return Ok(Some(
            applied(evaluator, args[0], &argument, locals, depth + 1)?.normalize(LIMIT)?,
        ));
    }
    if op == B::Domain {
        return Ok(Some(crate::sets::domain(
            evaluator,
            args[0],
            locals,
            depth + 1,
        )?));
    }
    let a = runtime(evaluator, args[0], locals, depth + 1)?;
    let result = match op {
        B::Cardinality => {
            Value::Int(i32::try_from(a.cardinality()?).map_err(|e| Error(e.to_string()))?)
        }
        B::IsFiniteSet => Value::Bool(a.finite()?),
        _ => {
            let b = runtime(evaluator, args[1], locals, depth + 1)?;
            match op {
                B::Eq | B::Ne => Value::Bool(a.semantic_eq(&b)? == (op == B::Eq)),
                B::SubsetEq => Value::Bool(a.subset_of(&b, LIMIT)?),
                _ => Value::Bool(tla_library::integer_comparison(op, &a, &b)?),
            }
        }
    };
    Ok(Some(result))
}
/// `TLCSet`, `TLCGet` and `TLCGetOrDefault` over the model's registers.
/// Without a model (evaluating a constant expression on its own) the
/// fallback registers have no workers: a named `TLCSet` is accepted and
/// forgotten, an indexed one fails as inactive, and `TLCGet` answers only
/// `duration` and `level` (handled here); `-D` keys echo themselves, the
/// checker's statistics keys are unsupported, and everything else is
/// undefined.
fn tlc_registers(
    evaluator: &Evaluator<'_>,
    op: B,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Value, Error> {
    let empty = crate::Registers::default();
    let registers = evaluator.registers.unwrap_or(&empty);
    if matches!(op, B::TlcSet | B::TlcGetOrDefault) {
        let [key, value] = args else {
            return Err(Error(format!("{op:?} expects two operands")));
        };
        let key = evaluator.eval(*key, locals, depth + 1)?;
        let value = evaluator.eval(*value, locals, depth + 1)?;
        return if op == B::TlcSet {
            registers.set(evaluator.worker, &key, value)
        } else {
            registers.get_or_default(evaluator.worker, &key, value)
        }
        .map_err(Error::from);
    }
    // `TLCGet` also answers the run-time keys the checker keeps itself.
    if op == B::TlcGet {
        let [argument] = args else {
            return Err(Error("TLCGet expects one operand".into()));
        };
        let value = evaluator.eval(*argument, locals, depth + 1)?;
        return match value {
            Value::String(name) if name.text.as_ref() == "duration" => {
                Ok(Value::Int(evaluator.started.elapsed().as_secs() as i32))
            }
            Value::String(name) if name.text.as_ref() == "level" => i32::try_from(evaluator.level)
                .map(Value::Int)
                .map_err(|e| Error(e.to_string())),
            _ => registers.get(evaluator.worker, &value).map_err(Error::from),
        };
    }
    Err(Error(format!("{op:?} is not a register operator")))
}

/// `TLCCache(body, closure, metadata)`, lowered as `Memo`: a constant body
/// is cached by the values its closure captured. `metadata` is the
/// `<<site, seed>>` pair the lowering attaches, `seed` being the site's
/// initial entries, so entries are kept per call site and keyed by the
/// closure within it. `polynomial` is the fingerprint polynomial the cache
/// hashes keys with.
fn memoized(
    evaluator: &Evaluator<'_>,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
    polynomial: u64,
) -> Result<Value, Error> {
    let [body, closure, metadata] = args else {
        return Err(Error("invalid TLCCache expression".into()));
    };
    // TLC caches state-level bodies per state, which this cache cannot
    // express, so they are rejected rather than silently miscached; TLC never
    // caches action or temporal bodies, so those are simply evaluated.
    match evaluator.program.nodes[*body].level {
        tla_ir::Level::State => {
            return Err(Error(
                "TLCCache of a state-level expression is not supported".into(),
            ));
        }
        tla_ir::Level::Constant => {}
        _ => return evaluator.eval(*body, locals, depth + 1),
    }
    // Within a site the cache keys on the closure alone.
    let key = evaluator.eval(*closure, locals, depth + 1)?;
    let metadata = evaluator.eval(*metadata, locals, depth + 1)?;
    evaluator
        .cache
        .evaluate(&metadata, key, polynomial, || {
            evaluator.eval(*body, locals, depth + 1)
        })
        .map_err(Error::from)
}

/// `ToString` and the `Json` module's renderings of a value.
fn text_conversion(
    evaluator: &Evaluator<'_>,
    op: B,
    args: &[ExprId],
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<Value, Error> {
    if args.len() != 1 {
        return Err(Error("text conversion expects one operand".into()));
    }
    let text = if op == B::ToString {
        render(evaluator, args[0], locals, depth + 1)?
    } else {
        tla_library::json(
            &runtime(evaluator, args[0], locals, depth + 1)?,
            match op {
                B::ToJsonArray => tla_library::JsonMode::Array,
                B::ToJsonObject => tla_library::JsonMode::Object,
                _ => tla_library::JsonMode::Value,
            },
        )?
    };
    evaluator.string(text)
}

fn render(
    evaluator: &Evaluator<'_>,
    id: ExprId,
    locals: &mut Vec<Option<Value>>,
    depth: usize,
) -> Result<String, Error> {
    stacker::maybe_grow(64 * 1024, 2 * 1024 * 1024, || {
        if depth >= evaluator.recursion_limit {
            return Err(Error("recursion limit exceeded".into()));
        }
        match &evaluator
            .program
            .nodes
            .get(id)
            .ok_or_else(|| Error("invalid expression".into()))?
            .expr
        {
            Ir::Call { operator, args } => {
                let op = evaluator
                    .program
                    .operators
                    .get(*operator)
                    .ok_or_else(|| Error("invalid operator".into()))?;
                if args.len() != op.parameters.len() {
                    return Err(Error("operator arity mismatch".into()));
                }
                let mut nested = locals.clone();
                for (slot, arg) in op.parameters.iter().zip(args) {
                    bind(&mut nested, *slot, evaluator.eval(*arg, locals, depth + 1)?);
                }
                render(evaluator, op.body, &mut nested, depth + 1)
            }
            Ir::If {
                condition,
                then_expr,
                else_expr,
            } => render(
                evaluator,
                if evaluator.if_guard(id, evaluator.eval(*condition, locals, depth + 1)?)? {
                    *then_expr
                } else {
                    *else_expr
                },
                locals,
                depth + 1,
            ),
            Ir::Function { domain, body, .. } | Ir::RecursiveFunction { domain, body, .. } => {
                match runtime(evaluator, id, locals, depth + 1)
                    .and_then(|v| tla_library::stringify(&v).map_err(Error::from))
                {
                    Ok(text) => Ok(text),
                    Err(_error) => {
                        let Some((parameter, body)) = evaluator.program.function_labels.get(&id)
                        else {
                            let domain = domain
                                .and_then(|id| builtin_set_name(evaluator.program, id))
                                .unwrap_or("\\universe");
                            return Ok(format!(
                                "[x \\in {domain} |-> <expression {}>]",
                                evaluator.program.source(*body).unwrap_or("unknown")
                            ));
                        };
                        let domain = match domain {
                            Some(domain) => match builtin_set_name(evaluator.program, *domain) {
                                Some(name) => name.into(),
                                None => tla_library::stringify(&runtime(
                                    evaluator,
                                    *domain,
                                    locals,
                                    depth + 1,
                                )?)?,
                            },
                            None => "\\universe".into(),
                        };
                        Ok(format!(
                            "[{parameter} \\in {domain} |-> <expression {body}>]"
                        ))
                    }
                }
            }
            _ => Ok(tla_library::stringify(&runtime(
                evaluator,
                id,
                locals,
                depth + 1,
            )?)?),
        }
    })
}

/// The name of a built-in infinite set, for messages that cannot enumerate it.
fn builtin_set_name(program: &tla_ir::Program, id: ExprId) -> Option<&'static str> {
    match &program.nodes[id].expr {
        Ir::Apply {
            op: B::Naturals,
            args,
        } if args.is_empty() => Some("Nat"),
        Ir::Apply {
            op: B::Integers,
            args,
        } if args.is_empty() => Some("Int"),
        Ir::Apply {
            op: B::Strings,
            args,
        } if args.is_empty() => Some("STRING"),
        _ => None,
    }
}
