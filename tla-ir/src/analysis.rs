use crate::{Builtin, Expr, ExprId, Level, Program};
use std::collections::{BTreeMap, HashMap};

impl Program {
    pub fn uses_random(&self, roots: impl IntoIterator<Item = ExprId>) -> bool {
        self.uses_builtins(roots, &[Builtin::RandomElement, Builtin::RandomSubset])
    }
    pub fn uses_builtins(
        &self,
        roots: impl IntoIterator<Item = ExprId>,
        builtins: &[Builtin],
    ) -> bool {
        let mut pending: Vec<_> = roots.into_iter().collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            match &self.nodes[id].expr {
                Expr::Apply { op, .. } | Expr::BuiltinOperator(op) if builtins.contains(op) => {
                    return true;
                }
                Expr::Apply { args, .. } => pending.extend(args),
                Expr::Call { operator, args } => {
                    pending.extend(args);
                    pending.push(self.operators[*operator].body);
                }
                Expr::Invoke { function, args, .. } => {
                    pending.push(*function);
                    pending.extend(args);
                }
                Expr::Scoped { current, next } => pending.extend([*current, *next]),
                Expr::Lambda { body, .. } | Expr::AssertError { body, .. } => pending.push(*body),
                Expr::If {
                    condition,
                    then_expr,
                    else_expr,
                } => pending.extend([*condition, *then_expr, *else_expr]),
                Expr::Quantify { domain, body, .. }
                | Expr::Function { domain, body, .. }
                | Expr::RecursiveFunction { domain, body, .. }
                | Expr::Choose { domain, body, .. } => {
                    pending.extend(domain);
                    pending.push(*body);
                }
                Expr::Except {
                    function,
                    key,
                    value,
                } => pending.extend([*function, *key, *value]),
                Expr::Temporal { args, bindings, .. } => {
                    pending.extend(args);
                    pending.extend(bindings.iter().map(|(_, id)| *id));
                }
                _ => {}
            }
        }
        false
    }

    pub fn specialize_calls(&mut self) -> Result<(), crate::Error> {
        let mut specializations = HashMap::new();
        let mut id = 0;
        while id < self.nodes.len() {
            let level = self.nodes[id].level;
            match self.nodes[id].expr.clone() {
                Expr::Invoke {
                    function,
                    args,
                    literal_message,
                } if matches!(
                    self.nodes[function].expr,
                    Expr::Lambda { .. } | Expr::BuiltinOperator(_)
                ) =>
                {
                    let result = self.invoke(function, args, literal_message, level)?;
                    self.nodes[id] = self.nodes[result].clone();
                    self.inherit(result, id);
                }
                Expr::Call { operator, args } => {
                    let bound: Vec<_> = args
                        .iter()
                        .map(|id| {
                            matches!(
                                self.nodes[*id].expr,
                                Expr::Lambda { .. } | Expr::BuiltinOperator(_)
                            )
                            .then_some(*id)
                        })
                        .collect();
                    if bound.iter().any(Option::is_some) {
                        let key = (operator, bound.clone());
                        let specialized = if let Some(&index) = specializations.get(&key) {
                            index
                        } else {
                            if specializations.len() >= 1024 {
                                return Err(crate::Error(
                                    "recursive operator specialization limit exceeded".into(),
                                ));
                            }
                            let original = self.operators[operator].clone();
                            if original.parameters.len() != args.len() {
                                return Err(crate::Error(
                                    "operator argument arity mismatch".into(),
                                ));
                            }
                            let index = self.operators.len();
                            specializations.insert(key, index);
                            let parameters = original
                                .parameters
                                .iter()
                                .zip(&bound)
                                .filter_map(|(slot, value)| value.is_none().then_some(*slot))
                                .collect();
                            self.operators.push(crate::Operator {
                                name: original.name,
                                parameters,
                                body: original.body,
                            });
                            let mut body = original.body;
                            for (slot, value) in original.parameters.iter().zip(&bound) {
                                if let Some(value) = value {
                                    body = self.replace_local(body, *slot, *value);
                                }
                            }
                            self.operators[index].body = body;
                            index
                        };
                        self.nodes[id].expr = Expr::Call {
                            operator: specialized,
                            args: args
                                .into_iter()
                                .zip(bound)
                                .filter_map(|(id, value)| value.is_none().then_some(id))
                                .collect(),
                        };
                    }
                }
                _ => {}
            }
            id += 1;
        }
        Ok(())
    }

    pub fn table_operator(&mut self, table: crate::Literal, parameters: Vec<usize>) -> ExprId {
        let args = parameters
            .iter()
            .map(|slot| self.push(Expr::Local(*slot), Level::Constant))
            .collect();
        let argument = self.push(
            Expr::Apply {
                op: Builtin::Tuple,
                args,
            },
            Level::Constant,
        );
        let function = self.push(Expr::Literal(table), Level::Constant);
        let body = self.push(
            Expr::Apply {
                op: Builtin::Apply,
                args: vec![function, argument],
            },
            Level::Constant,
        );
        self.push(Expr::Lambda { parameters, body }, Level::Constant)
    }

    pub fn invoke(
        &mut self,
        function: ExprId,
        args: Vec<ExprId>,
        literal_message: bool,
        level: Level,
    ) -> Result<ExprId, crate::Error> {
        match self.nodes[function].expr.clone() {
            Expr::BuiltinOperator(Builtin::Prime | Builtin::Unchanged) => {
                let [current] = args.as_slice() else {
                    return Err(crate::Error("state operator expects one argument".into()));
                };
                let next = self.prime(*current)?;
                Ok(
                    if self.nodes[function].expr == Expr::BuiltinOperator(Builtin::Prime) {
                        next
                    } else {
                        self.unchanged_between(*current, next)
                    },
                )
            }
            Expr::BuiltinOperator(op) => {
                let id = self.checked_apply(op, args, literal_message, level)?;
                Ok(self.inherit(function, id))
            }
            Expr::Lambda {
                parameters,
                mut body,
            } => {
                if parameters.len() != args.len() {
                    return Err(crate::Error("operator argument arity mismatch".into()));
                }
                for (slot, argument) in parameters.into_iter().zip(args) {
                    body = self.replace_local(body, slot, argument);
                }
                Ok(self.at_least(body, level))
            }
            _ => Ok(self.push(
                Expr::Invoke {
                    function,
                    args,
                    literal_message,
                },
                level,
            )),
        }
    }

    pub fn enabled_scope<K: Copy + Ord + std::hash::Hash>(
        &mut self,
        bindings: impl IntoIterator<Item = (K, ExprId)>,
        variables: &mut HashMap<K, ExprId>,
    ) -> HashMap<K, (ExprId, ExprId)> {
        bindings
            .into_iter()
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .map(|(key, original)| {
                let variable = *variables.entry(key).or_insert_with(|| {
                    let slot = self.variables.len() + self.enabled_variables;
                    self.enabled_variables += 1;
                    self.push(
                        Expr::Variable {
                            slot,
                            primed: false,
                        },
                        Level::State,
                    )
                });
                (key, (original, variable))
            })
            .collect()
    }
    pub fn scoped_binding<K: Eq + std::hash::Hash>(
        &mut self,
        key: &K,
        substitutions: &HashMap<K, ExprId>,
        scope: &HashMap<K, (ExprId, ExprId)>,
    ) -> Option<ExprId> {
        let current = *substitutions.get(key)?;
        let Some((_, variable)) = scope.get(key).filter(|(original, _)| *original == current)
        else {
            return Some(current);
        };
        let Expr::Variable { slot, .. } = self.nodes[*variable].expr else {
            unreachable!("scope slots are variables");
        };
        let next = self.push(Expr::Variable { slot, primed: true }, Level::Action);
        Some(self.push(Expr::Scoped { current, next }, self.nodes[current].level))
    }
    pub fn copy_expr(&mut self, id: ExprId) -> ExprId {
        self.derived(id, self.nodes[id].expr.clone(), self.nodes[id].level)
    }
    pub fn unchanged_between(&mut self, current: ExprId, next: ExprId) -> ExprId {
        let mut pending = vec![(current, next)];
        let mut args = Vec::new();
        while let Some((current, next)) = pending.pop() {
            if let (
                Expr::Apply {
                    op: Builtin::Tuple,
                    args: a,
                },
                Expr::Apply {
                    op: Builtin::Tuple,
                    args: b,
                },
            ) = (&self.nodes[current].expr, &self.nodes[next].expr)
                && a.len() == b.len()
            {
                pending.extend(a.iter().zip(b).rev().map(|(a, b)| (*a, *b)));
            } else {
                args.push(self.push(
                    Expr::Apply {
                        op: Builtin::Eq,
                        args: vec![next, current],
                    },
                    Level::Action,
                ));
            }
        }
        self.push(
            Expr::Apply {
                op: Builtin::And,
                args,
            },
            Level::Action,
        )
    }
    pub fn assignment_variable(&self, id: ExprId, initial: bool) -> Option<usize> {
        match self.nodes.get(id)?.expr {
            Expr::Scoped { current, .. } => self.assignment_variable(current, initial),
            Expr::Variable { slot, primed } if primed != initial => Some(slot),
            _ => None,
        }
    }
    pub fn order_assignments(
        &self,
        args: &[ExprId],
        mut assigned: Vec<bool>,
        initial: bool,
    ) -> Vec<ExprId> {
        let mut pending = args.to_vec();
        let mut ordered = Vec::with_capacity(pending.len());
        while !pending.is_empty() {
            let index = pending
                .iter()
                .position(|id| {
                    !self.reads_unassigned(
                        *id,
                        &assigned,
                        self.assignment_target(*id, initial),
                        initial,
                    )
                })
                .unwrap_or(0);
            let id = pending.remove(index);
            if let Some(slot) = self.assignment_target(id, initial)
                && let Some(flag) = assigned.get_mut(slot)
            {
                *flag = true;
            }
            ordered.push(id);
        }
        ordered
    }
    fn assignment_target(&self, id: ExprId, initial: bool) -> Option<usize> {
        let Expr::Apply { op, args } = &self.nodes.get(id)?.expr else {
            return None;
        };
        if !matches!(op, Builtin::Eq | Builtin::In | Builtin::SubsetEq) || args.len() != 2 {
            return None;
        }
        self.assignment_variable(args[0], initial)
            .or_else(|| self.assignment_variable(args[1], initial))
    }
    fn reads_unassigned(
        &self,
        id: ExprId,
        assigned: &[bool],
        ignore: Option<usize>,
        initial: bool,
    ) -> bool {
        let mut stack = vec![id];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if ignore.is_none()
                && let Some(target) = self.assignment_target(id, initial)
            {
                if self.reads_unassigned(id, assigned, Some(target), initial) {
                    return true;
                }
                continue;
            }
            match &node.expr {
                Expr::Scoped { current, .. } => stack.push(*current),
                Expr::Apply {
                    op: Builtin::Enabled,
                    ..
                } if !initial => {}
                Expr::Variable { slot, primed } if *primed != initial && Some(*slot) != ignore => {
                    if assigned.get(*slot).copied() != Some(true) {
                        return true;
                    }
                }
                Expr::Apply {
                    op: Builtin::Not,
                    args,
                } if !initial && self.negated_unchanged(args).is_some() => {
                    // `~UNCHANGED e` compares e' with e, so it reads e's variables primed.
                    let mut variables = self.negated_unchanged(args).unwrap_or_default();
                    let mut visited = std::collections::HashSet::new();
                    while let Some(id) = variables.pop() {
                        if !visited.insert(id) {
                            continue;
                        }
                        match &self.nodes[id].expr {
                            Expr::Variable { slot, .. } => {
                                if assigned.get(*slot).copied() != Some(true) {
                                    return true;
                                }
                            }
                            Expr::Apply { args, .. } => variables.extend(args),
                            Expr::Call { operator, args } => {
                                variables.extend(args);
                                variables.push(self.operators[*operator].body);
                            }
                            _ => stack.push(id),
                        }
                    }
                }
                Expr::Apply {
                    op: Builtin::Unchanged,
                    args,
                } if !initial => {
                    let mut variables = args.clone();
                    let mut visited = std::collections::HashSet::new();
                    while let Some(id) = variables.pop() {
                        if !visited.insert(id) {
                            continue;
                        }
                        match &self.nodes[id].expr {
                            Expr::Variable { primed: false, .. } => {}
                            Expr::Apply { args, .. } => variables.extend(args),
                            Expr::Call { operator, args } => {
                                variables.extend(args);
                                variables.push(self.operators[*operator].body);
                            }
                            _ => stack.push(id),
                        }
                    }
                }
                Expr::Apply { args, .. } => stack.extend(args),
                Expr::Call { operator, args } => {
                    stack.extend(args);
                    if let Some(op) = self.operators.get(*operator) {
                        stack.push(op.body);
                    }
                }
                Expr::Invoke { function, args, .. } => {
                    stack.push(*function);
                    stack.extend(args);
                }
                Expr::AssertError { body, .. } => stack.push(*body),
                Expr::If {
                    condition,
                    then_expr,
                    else_expr,
                } => stack.extend([*condition, *then_expr, *else_expr]),
                Expr::Quantify { domain, body, .. }
                | Expr::Function { domain, body, .. }
                | Expr::RecursiveFunction { domain, body, .. }
                | Expr::Choose { domain, body, .. } => {
                    stack.extend(domain.iter().copied());
                    stack.push(*body);
                }
                Expr::Except {
                    function,
                    key,
                    value,
                } => stack.extend([*function, *key, *value]),
                _ => {}
            }
        }
        false
    }
}

impl Program {
    fn negated_unchanged(&self, args: &[ExprId]) -> Option<Vec<ExprId>> {
        match args {
            [arg] => match &self.nodes.get(*arg)?.expr {
                Expr::Apply {
                    op: Builtin::Unchanged,
                    args,
                } => Some(args.clone()),
                _ => None,
            },
            _ => None,
        }
    }
    /// Instantiation can raise a template expression to the application's level.
    pub fn at_least(&mut self, id: ExprId, level: Level) -> ExprId {
        if self.nodes[id].level >= level {
            return id;
        }
        let expr = self.nodes[id].expr.clone();
        self.derived(id, expr, level)
    }
}
