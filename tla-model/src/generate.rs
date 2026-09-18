use crate::{BoundModel, Error, State, Value};
use std::ops::ControlFlow;
use tla_eval::{Evaluator, bind};
use tla_ir::{Builtin, Expr, ExprId, Quantifier};

type Partial = Vec<Option<Value>>;
type Flow = Result<ControlFlow<()>, Error>;
type Output<'a> = dyn FnMut(Partial) -> Flow + 'a;

pub(crate) fn visit(
    model: &BoundModel,
    predicates: &[ExprId],
    state: &State,
    initial: bool,
    output: &mut impl FnMut(State) -> Flow,
) -> Flow {
    let current: Vec<_> = state.iter().cloned().map(Some).collect();
    let generator = Generator {
        model,
        current: &current,
        initial,
        enabled: false,
        level: if initial { 0 } else { state.depth },
    };
    generator.all(
        predicates,
        vec![None; model.program.variables.len()],
        &[],
        0,
        &mut |candidate| {
            let values = model.complete_values(candidate)?;
            output(State {
                values,
                depth: if initial { 1 } else { state.depth + 1 },
            })
        },
    )
}

struct Generator<'a> {
    model: &'a BoundModel,
    current: &'a [Option<Value>],
    initial: bool,
    enabled: bool,
    level: usize,
}
pub(crate) fn evaluate(
    model: &BoundModel,
    id: ExprId,
    current: &[Option<Value>],
    next: &[Option<Value>],
    locals: &[Option<Value>],
    level: usize,
) -> Result<Value, Error> {
    with_evaluator(model, current, next, level, |evaluator| {
        evaluator.with_locals(id, locals)
    })
}
fn with_evaluator<R>(
    model: &BoundModel,
    current: &[Option<Value>],
    next: &[Option<Value>],
    level: usize,
    operation: impl FnOnce(&Evaluator<'_>) -> Result<R, Error>,
) -> Result<R, Error> {
    let enabled = |action, locals: &[Option<Value>]| {
        let generator = Generator {
            model,
            current,
            initial: false,
            enabled: true,
            level,
        };
        Ok(generator
            .solve(
                action,
                vec![None; model.program.variables.len() + model.program.enabled_variables],
                locals,
                0,
                &mut |_| Ok(ControlFlow::Break(())),
            )?
            .is_break())
    };
    let mut evaluator = model.evaluator(current, next, level);
    let random = model
        .runtime
        .randomized
        .then(|| model.runtime.random.current())
        .transpose()?;
    evaluator.random = random.as_deref();
    evaluator.enabled = Some(&enabled);
    operation(&evaluator)
}
impl Generator<'_> {
    fn if_guard(&self, id: ExprId, value: Value) -> Result<bool, Error> {
        use tla_eval::BooleanContext::{EnabledIf, InitialIf, NextIf};
        let context = match (self.enabled, self.initial) {
            (true, _) => EnabledIf,
            (false, true) => InitialIf,
            (false, false) => NextIf,
        };
        self.model.boolean(id, value, context)
    }
    fn evaluate(
        &self,
        id: ExprId,
        candidate: &[Option<Value>],
        locals: &[Option<Value>],
    ) -> Result<Value, Error> {
        let state = if self.initial {
            candidate
        } else {
            self.current
        };
        evaluate(self.model, id, state, candidate, locals, self.level)
    }
    fn assign(
        &self,
        mut candidate: Partial,
        slot: usize,
        value: Value,
        output: &mut Output<'_>,
    ) -> Flow {
        let target = candidate
            .get_mut(slot)
            .ok_or_else(|| Error("invalid state slot".into()))?;
        if let Some(previous) = target
            && !previous.semantic_eq(&value)?
        {
            return Ok(ControlFlow::Continue(()));
        }
        *target = Some(value);
        output(candidate)
    }
    fn all(
        &self,
        predicates: &[ExprId],
        candidate: Partial,
        locals: &[Option<Value>],
        depth: usize,
        output: &mut Output<'_>,
    ) -> Flow {
        if let Some((&head, rest)) = predicates.split_first() {
            self.solve(head, candidate, locals, depth + 1, &mut |candidate| {
                self.all(rest, candidate, locals, depth + 1, output)
            })
        } else {
            output(candidate)
        }
    }
    fn all_bindings(
        &self,
        binding: (usize, ExprId, usize),
        values: &[Value],
        candidate: Partial,
        locals: &[Option<Value>],
        output: &mut Output<'_>,
    ) -> Flow {
        if let Some((value, rest)) = values.split_first() {
            let mut nested = locals.to_vec();
            bind(&mut nested, binding.0, value.clone());
            stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
                self.solve(binding.1, candidate, &nested, binding.2, &mut |candidate| {
                    self.all_bindings(binding, rest, candidate, locals, output)
                })
            })
        } else {
            output(candidate)
        }
    }
    fn solve(
        &self,
        id: ExprId,
        candidate: Partial,
        locals: &[Option<Value>],
        depth: usize,
        output: &mut Output<'_>,
    ) -> Flow {
        if depth > 128 {
            return Err(Error("action recursion limit exceeded".into()));
        }
        let node = self
            .model
            .program
            .nodes
            .get(id)
            .ok_or_else(|| Error("invalid action expression".into()))?;
        match &node.expr {
            Expr::Scoped { current, .. } => {
                self.solve(*current, candidate, locals, depth + 1, output)
            }
            Expr::Apply {
                op: Builtin::Apply,
                args,
            } if args.len() == 2 && !self.initial => self.apply_function(
                args[0],
                self.evaluate(args[1], &candidate, locals)?,
                candidate,
                locals,
                depth + 1,
                output,
            ),
            Expr::Apply {
                op: Builtin::Case,
                args,
            } => {
                for branch in args {
                    let Expr::Apply {
                        op: Builtin::Tuple,
                        args,
                    } = &self.model.program.nodes[*branch].expr
                    else {
                        return Err(Error("invalid CASE branch".into()));
                    };
                    let [guard, body] = args.as_slice() else {
                        return Err(Error("invalid CASE branch arity".into()));
                    };
                    if self.evaluate(*guard, &candidate, locals)?.boolean()? {
                        return self.solve(*body, candidate, locals, depth + 1, output);
                    }
                }
                Err(Error("CASE has no true condition".into()))
            }
            Expr::Apply {
                op: Builtin::And,
                args,
            } => {
                let ordered = self.model.program.order_assignments(
                    args,
                    candidate.iter().map(Option::is_some).collect(),
                    self.initial,
                );
                self.all(&ordered, candidate, locals, depth + 1, output)
            }
            Expr::Apply {
                op: Builtin::Or,
                args,
            } => {
                for arg in args {
                    if self
                        .solve(*arg, candidate.clone(), locals, depth + 1, output)?
                        .is_break()
                    {
                        return Ok(ControlFlow::Break(()));
                    }
                }
                Ok(ControlFlow::Continue(()))
            }
            Expr::Apply {
                op: Builtin::Eq,
                args,
            } if args.len() == 2 => {
                for (left, right) in [(args[0], args[1]), (args[1], args[0])] {
                    if let Some(slot) = self.model.program.assignment_variable(left, self.initial) {
                        return self.assign(
                            candidate.clone(),
                            slot,
                            self.evaluate(right, &candidate, locals)?,
                            output,
                        );
                    }
                }
                self.filter(id, candidate, locals, output)
            }
            Expr::Apply {
                op: op @ (Builtin::In | Builtin::SubsetEq),
                args,
            } if args.len() == 2 => {
                if let Some(slot) = self
                    .model
                    .program
                    .assignment_variable(args[0], self.initial)
                {
                    let domain = self.evaluate(args[1], &candidate, locals)?;
                    let values = if *op == Builtin::SubsetEq {
                        tla_eval::SetExpr::subsets(&domain)?
                    } else {
                        tla_eval::SetExpr::finite(&domain)?
                    };
                    for value in values.iter()? {
                        if self
                            .assign(candidate.clone(), slot, value, output)?
                            .is_break()
                        {
                            return Ok(ControlFlow::Break(()));
                        }
                    }
                    Ok(ControlFlow::Continue(()))
                } else {
                    self.filter(id, candidate, locals, output)
                }
            }
            Expr::Apply {
                op: Builtin::Unchanged,
                args,
            } if !self.initial => {
                let mut candidate = candidate;
                let mut pending = args.clone();
                while let Some(arg) = pending.pop() {
                    match &self.model.program.nodes[arg].expr {
                        Expr::Variable {
                            slot,
                            primed: false,
                        } => {
                            let value = self.current.get(*slot).and_then(Clone::clone).ok_or_else(
                                || Error("UNCHANGED has unbound current variable".into()),
                            )?;
                            if let Some(previous) = &candidate[*slot]
                                && !previous.semantic_eq(&value)?
                            {
                                return Ok(ControlFlow::Continue(()));
                            }
                            candidate[*slot] = Some(value);
                        }
                        Expr::Apply {
                            op: Builtin::Tuple,
                            args,
                        } => pending.extend(args),
                        Expr::Call { operator, args } if args.is_empty() => {
                            pending.push(self.model.program.operators[*operator].body)
                        }
                        _ => return Err(Error("UNCHANGED of a non-variable".into())),
                    }
                }
                output(candidate)
            }
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => self.solve(
                if self.if_guard(id, self.evaluate(*condition, &candidate, locals)?)? {
                    *then_expr
                } else {
                    *else_expr
                },
                candidate,
                locals,
                depth + 1,
                output,
            ),
            Expr::Quantify {
                kind: kind @ (Quantifier::Exists | Quantifier::ForAll),
                slot,
                domain,
                body,
            } => {
                let domain =
                    domain.ok_or_else(|| Error("unbounded quantifier in an action".into()))?;
                let values = with_evaluator(
                    self.model,
                    if self.initial {
                        &candidate
                    } else {
                        self.current
                    },
                    &candidate,
                    self.level,
                    |evaluator| evaluator.enumerate_with_locals(domain, locals),
                )?;
                if *kind == Quantifier::ForAll {
                    return self.all_bindings(
                        (*slot, *body, depth + 1),
                        &values,
                        candidate,
                        locals,
                        output,
                    );
                }
                for value in &values {
                    let mut nested = locals.to_vec();
                    bind(&mut nested, *slot, value.clone());
                    if self
                        .solve(*body, candidate.clone(), &nested, depth + 1, output)?
                        .is_break()
                    {
                        return Ok(ControlFlow::Break(()));
                    }
                }
                Ok(ControlFlow::Continue(()))
            }
            Expr::Call { operator, args } => {
                let operator = self
                    .model
                    .program
                    .operators
                    .get(*operator)
                    .ok_or_else(|| Error("invalid action operator".into()))?;
                if operator.parameters.len() != args.len() {
                    return Err(Error("action operator arity mismatch".into()));
                }
                let mut nested = locals.to_vec();
                let mut expanded = None;
                let mut body = operator.body;
                for (&slot, arg) in operator.parameters.iter().zip(args) {
                    if self
                        .model
                        .program
                        .assignment_variable(*arg, self.initial)
                        .is_some_and(|slot| candidate[slot].is_none())
                    {
                        let model = expanded.get_or_insert_with(|| self.model.clone());
                        body = model.program.replace_local(body, slot, *arg);
                    } else {
                        bind(&mut nested, slot, self.evaluate(*arg, &candidate, locals)?);
                    }
                }
                Generator {
                    model: expanded.as_ref().unwrap_or(self.model),
                    ..*self
                }
                .solve(body, candidate, &nested, depth + 1, output)
            }
            _ => self.filter(id, candidate, locals, output),
        }
    }
    fn apply_function(
        &self,
        function: ExprId,
        argument: Value,
        candidate: Partial,
        locals: &[Option<Value>],
        depth: usize,
        output: &mut Output<'_>,
    ) -> Flow {
        if depth > 128 {
            return Err(Error("action recursion limit exceeded".into()));
        }
        let current = if self.initial {
            &candidate
        } else {
            self.current
        };
        match &self.model.program.nodes[function].expr {
            Expr::Function { slot, domain, body }
            | Expr::RecursiveFunction { slot, domain, body } => {
                if let Some(domain) = domain
                    && !with_evaluator(self.model, current, &candidate, self.level, |evaluator| {
                        evaluator.contains_with_locals(*domain, argument.clone(), locals)
                    })?
                {
                    return Err(Error("function argument outside domain".into()));
                }
                let mut nested = locals.to_vec();
                bind(&mut nested, *slot, argument);
                self.solve(*body, candidate, &nested, depth + 1, output)
            }
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => {
                let function = if self.model.boolean(
                    function,
                    self.evaluate(*condition, &candidate, locals)?,
                    tla_eval::BooleanContext::If,
                )? {
                    *then_expr
                } else {
                    *else_expr
                };
                self.apply_function(function, argument, candidate, locals, depth + 1, output)
            }
            Expr::Call { operator, args } => {
                let operator = &self.model.program.operators[*operator];
                if operator.parameters.len() != args.len() {
                    return Err(Error("operator arity mismatch".into()));
                }
                let mut nested = locals.to_vec();
                for (&slot, arg) in operator.parameters.iter().zip(args) {
                    bind(&mut nested, slot, self.evaluate(*arg, &candidate, locals)?);
                }
                self.apply_function(
                    operator.body,
                    argument,
                    candidate,
                    &nested,
                    depth + 1,
                    output,
                )
            }
            _ => {
                let value =
                    with_evaluator(self.model, current, &candidate, self.level, |evaluator| {
                        evaluator.apply_with_locals(function, argument, locals)
                    })?;
                if value.boolean()? {
                    output(candidate)
                } else {
                    Ok(ControlFlow::Continue(()))
                }
            }
        }
    }
    fn filter(
        &self,
        id: ExprId,
        candidate: Partial,
        locals: &[Option<Value>],
        output: &mut Output<'_>,
    ) -> Flow {
        if self.evaluate(id, &candidate, locals)?.boolean()? {
            output(candidate)
        } else {
            Ok(ControlFlow::Continue(()))
        }
    }
}
