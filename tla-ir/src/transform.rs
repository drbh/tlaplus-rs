//! Expression rewrites over a [`Program`]: priming, slot substitution and
//! tuple destructuring.

use crate::{Error, Expr, ExprId, Level, Literal, Program};

impl Program {
    /// `e'`: every state variable read in `e` becomes a primed read. The
    /// result is action level. Calls retain their parameters and use primed
    /// operator templates so recursive arguments advance on each invocation.
    /// Shared and recursive references use a placeholder redirected after the
    /// referent is primed, so cycles do not need the Rust stack.
    pub fn prime(&mut self, root: ExprId) -> Result<ExprId, Error> {
        let mut memo = std::collections::HashMap::new();
        let mut forward = std::collections::HashMap::new();
        let mut operators = std::collections::HashMap::new();
        let mut stack = vec![root];
        let mut build = vec![false];
        while let Some(&id) = stack.last() {
            if memo.contains_key(&id) {
                stack.pop();
                build.pop();
                continue;
            }
            if !build.last().copied().unwrap_or(true) {
                *build.last_mut().unwrap() = true;
                forward.insert(
                    id,
                    self.push(Expr::Literal(Literal::Bool(false)), Level::Action),
                );
                for child in self.prime_deps(id)?.into_iter().rev() {
                    if memo.contains_key(&child) || forward.contains_key(&child) {
                        continue;
                    }
                    stack.push(child);
                    build.push(false);
                }
                continue;
            }
            stack.pop();
            build.pop();
            let primed = self.prime_node(id, &memo, &forward, &mut operators)?;
            let placeholder = forward.remove(&id).expect("prime placeholder");
            self.redirect(placeholder, primed);
            memo.insert(id, primed);
        }
        memo.get(&root)
            .copied()
            .ok_or_else(|| Error("priming failed".into()))
    }

    fn redirect(&mut self, from: ExprId, to: ExprId) {
        if from == to {
            return;
        }
        let rewrite = |id: &mut ExprId| {
            if *id == from {
                *id = to;
            }
        };
        for node in &mut self.nodes {
            match &mut node.expr {
                Expr::Scoped { current, next } => {
                    rewrite(current);
                    rewrite(next);
                }
                Expr::Lambda { body, .. } | Expr::AssertError { body, .. } => rewrite(body),
                Expr::Invoke { function, args, .. } => {
                    rewrite(function);
                    args.iter_mut().for_each(&rewrite);
                }
                Expr::Apply { args, .. } | Expr::Call { args, .. } => {
                    args.iter_mut().for_each(&rewrite)
                }
                Expr::If {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    rewrite(condition);
                    rewrite(then_expr);
                    rewrite(else_expr);
                }
                Expr::Quantify { domain, body, .. }
                | Expr::Function { domain, body, .. }
                | Expr::RecursiveFunction { domain, body, .. }
                | Expr::Choose { domain, body, .. } => {
                    domain.as_mut().map(&rewrite);
                    rewrite(body);
                }
                Expr::Except {
                    function,
                    key,
                    value,
                } => {
                    rewrite(function);
                    rewrite(key);
                    rewrite(value);
                }
                Expr::Temporal { args, bindings, .. } => {
                    args.iter_mut().for_each(&rewrite);
                    bindings.iter_mut().for_each(|(_, id)| rewrite(id));
                }
                _ => {}
            }
        }
        for operator in &mut self.operators {
            rewrite(&mut operator.body);
        }
    }

    fn prime_deps(&self, id: ExprId) -> Result<Vec<ExprId>, Error> {
        Ok(match &self.nodes[id].expr {
            Expr::Scoped { .. }
            | Expr::BuiltinOperator(_)
            | Expr::Literal(_)
            | Expr::Constant(_)
            | Expr::Local(_)
            | Expr::At
            | Expr::Variable { .. } => Vec::new(),
            Expr::Apply { args, .. } => args.clone(),
            Expr::Invoke { function, args, .. } => {
                let mut deps = vec![*function];
                deps.extend(args);
                deps
            }
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => vec![*condition, *then_expr, *else_expr],
            Expr::Quantify { domain, body, .. }
            | Expr::Function { domain, body, .. }
            | Expr::RecursiveFunction { domain, body, .. }
            | Expr::Choose { domain, body, .. } => {
                let mut deps: Vec<_> = domain.iter().copied().collect();
                deps.push(*body);
                deps
            }
            Expr::Except {
                function,
                key,
                value,
            } => vec![*function, *key, *value],
            Expr::Lambda { body, .. } | Expr::AssertError { body, .. } => vec![*body],
            Expr::Call { operator, args } => {
                let mut deps = args.clone();
                deps.push(
                    self.operators
                        .get(*operator)
                        .ok_or_else(|| Error("priming an unknown operator".into()))?
                        .body,
                );
                deps
            }
            Expr::Temporal { .. } => {
                return Err(Error("priming a temporal expression is unsupported".into()));
            }
        })
    }

    fn prime_node(
        &mut self,
        id: ExprId,
        memo: &std::collections::HashMap<ExprId, ExprId>,
        forward: &std::collections::HashMap<ExprId, ExprId>,
        operators: &mut std::collections::HashMap<usize, usize>,
    ) -> Result<ExprId, Error> {
        let child = |id: ExprId| memo.get(&id).or_else(|| forward.get(&id)).copied().unwrap();
        let expr = match self.nodes[id].expr.clone() {
            Expr::Scoped { next, .. } => return Ok(next),
            Expr::Variable {
                slot,
                primed: false,
            } => Expr::Variable { slot, primed: true },
            Expr::BuiltinOperator(_)
            | Expr::Literal(_)
            | Expr::Constant(_)
            | Expr::Local(_)
            | Expr::At
            | Expr::Variable { .. } => return Ok(id),
            Expr::Apply { op, args } => Expr::Apply {
                op,
                args: args.into_iter().map(child).collect(),
            },
            Expr::Invoke {
                function,
                args,
                literal_message,
            } => Expr::Invoke {
                function: child(function),
                args: args.into_iter().map(child).collect(),
                literal_message,
            },
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => Expr::If {
                condition: child(condition),
                then_expr: child(then_expr),
                else_expr: child(else_expr),
            },
            Expr::Quantify {
                kind,
                slot,
                domain,
                body,
            } => Expr::Quantify {
                kind,
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::Function { slot, domain, body } => Expr::Function {
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::RecursiveFunction { slot, domain, body } => Expr::RecursiveFunction {
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::Choose { slot, domain, body } => Expr::Choose {
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::Except {
                function,
                key,
                value,
            } => Expr::Except {
                function: child(function),
                key: child(key),
                value: child(value),
            },
            Expr::AssertError { message, body } => Expr::AssertError {
                message,
                body: child(body),
            },
            Expr::Lambda { parameters, body } => Expr::Lambda {
                parameters,
                body: child(body),
            },
            Expr::Call { operator, args } => {
                let operator = *operators.entry(operator).or_insert_with(|| {
                    let mut definition = self.operators[operator].clone();
                    definition.body = child(definition.body);
                    let index = self.operators.len();
                    self.operators.push(definition);
                    index
                });
                Expr::Call {
                    operator,
                    args: args.into_iter().map(child).collect(),
                }
            }
            Expr::Temporal { .. } => {
                return Err(Error("priming a temporal expression is unsupported".into()));
            }
        };
        Ok(self.derived(id, expr, Level::Action))
    }

    /// Substitute `replacement` for every `Local(slot)` occurrence. Slots are
    /// globally unique; captured slots require cloning operator templates too.
    pub fn replace_local(&mut self, root: ExprId, slot: usize, replacement: ExprId) -> ExprId {
        let mut parents = std::collections::HashMap::<_, Vec<_>>::new();
        let mut visited = std::collections::HashSet::new();
        let mut affected = std::collections::HashSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if matches!(self.nodes[id].expr, Expr::Local(local) if local == slot) {
                affected.insert(id);
            }
            for child in self.refs(id, slot) {
                parents.entry(child).or_default().push(id);
                pending.push(child);
            }
        }
        pending.extend(affected.iter().copied());
        while let Some(id) = pending.pop() {
            for &parent in parents.get(&id).into_iter().flatten() {
                if affected.insert(parent) {
                    pending.push(parent);
                }
            }
        }
        let mut memo = std::collections::HashMap::new();
        let mut forward = std::collections::HashMap::new();
        let mut stack = vec![root];
        let mut build = vec![false];
        while let Some(&id) = stack.last() {
            if memo.contains_key(&id) {
                stack.pop();
                build.pop();
                continue;
            }
            if !build.last().copied().unwrap_or(true) {
                *build.last_mut().unwrap() = true;
                if matches!(self.nodes[id].expr, Expr::Local(local) if local == slot) {
                    memo.insert(id, replacement);
                    stack.pop();
                    build.pop();
                    continue;
                }
                if !affected.contains(&id) {
                    memo.insert(id, id);
                    stack.pop();
                    build.pop();
                    continue;
                }
                forward.insert(
                    id,
                    self.push(Expr::Literal(Literal::Bool(false)), self.nodes[id].level),
                );
                for child in self.refs(id, slot).into_iter().rev() {
                    if !memo.contains_key(&child) && !forward.contains_key(&child) {
                        stack.push(child);
                        build.push(false);
                    }
                }
                continue;
            }
            stack.pop();
            build.pop();
            let replaced = self.replace_node(id, slot, &memo, &forward);
            let placeholder = forward.remove(&id).expect("replace placeholder");
            self.redirect(placeholder, replaced);
            memo.insert(id, replaced);
        }
        memo[&root]
    }

    fn refs(&self, id: ExprId, slot: usize) -> Vec<ExprId> {
        match &self.nodes[id].expr {
            Expr::Scoped { current, next } => vec![*current, *next],
            Expr::Apply { args, .. } => args.clone(),
            Expr::Call { operator, args } => {
                let mut refs = args.clone();
                let operator = &self.operators[*operator];
                if !operator.parameters.contains(&slot) {
                    refs.push(operator.body);
                }
                refs
            }
            Expr::Invoke { function, args, .. } => {
                let mut refs = vec![*function];
                refs.extend(args);
                refs
            }
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => vec![*condition, *then_expr, *else_expr],
            Expr::Quantify { domain, body, .. }
            | Expr::Function { domain, body, .. }
            | Expr::RecursiveFunction { domain, body, .. }
            | Expr::Choose { domain, body, .. } => {
                let mut refs: Vec<_> = domain.iter().copied().collect();
                refs.push(*body);
                refs
            }
            Expr::Except {
                function,
                key,
                value,
            } => vec![*function, *key, *value],
            Expr::Lambda { body, .. } | Expr::AssertError { body, .. } => vec![*body],
            Expr::Temporal { args, bindings, .. } => {
                let mut refs = args.clone();
                refs.extend(bindings.iter().map(|(_, id)| *id));
                refs
            }
            _ => Vec::new(),
        }
    }

    fn replace_node(
        &mut self,
        id: ExprId,
        slot: usize,
        memo: &std::collections::HashMap<ExprId, ExprId>,
        forward: &std::collections::HashMap<ExprId, ExprId>,
    ) -> ExprId {
        let child = |id: ExprId| memo.get(&id).or_else(|| forward.get(&id)).copied().unwrap();
        let level = self.nodes[id].level;
        let expr = match self.nodes[id].expr.clone() {
            Expr::Scoped { current, next } => Expr::Scoped {
                current: child(current),
                next: child(next),
            },
            Expr::Apply { op, args } => Expr::Apply {
                op,
                args: args.into_iter().map(child).collect(),
            },
            Expr::Invoke {
                function,
                args,
                literal_message,
            } => Expr::Invoke {
                function: child(function),
                args: args.into_iter().map(child).collect(),
                literal_message,
            },
            Expr::If {
                condition,
                then_expr,
                else_expr,
            } => Expr::If {
                condition: child(condition),
                then_expr: child(then_expr),
                else_expr: child(else_expr),
            },
            Expr::Quantify {
                kind,
                slot,
                domain,
                body,
            } => Expr::Quantify {
                kind,
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::Function { slot, domain, body } => Expr::Function {
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::RecursiveFunction { slot, domain, body } => Expr::RecursiveFunction {
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::Choose { slot, domain, body } => Expr::Choose {
                slot,
                domain: domain.map(child),
                body: child(body),
            },
            Expr::Except {
                function,
                key,
                value,
            } => Expr::Except {
                function: child(function),
                key: child(key),
                value: child(value),
            },
            Expr::AssertError { message, body } => Expr::AssertError {
                message,
                body: child(body),
            },
            Expr::Lambda { parameters, body } => Expr::Lambda {
                parameters,
                body: child(body),
            },
            Expr::Call { operator, args } => {
                let mut definition = self.operators[operator].clone();
                let operator = if definition.parameters.contains(&slot)
                    || child(definition.body) == definition.body
                {
                    operator
                } else {
                    definition.body = child(definition.body);
                    let index = self.operators.len();
                    self.operators.push(definition);
                    index
                };
                Expr::Call {
                    operator,
                    args: args.into_iter().map(child).collect(),
                }
            }
            Expr::Temporal { op, args, bindings } => Expr::Temporal {
                op,
                args: args.into_iter().map(child).collect(),
                bindings: bindings
                    .into_iter()
                    .map(|(bound, id)| (bound, child(id)))
                    .collect(),
            },
            _ => return id,
        };
        self.derived(id, expr, level)
    }
}
