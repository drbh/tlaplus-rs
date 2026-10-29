use tlaplus_eval::{Error, Evaluator, Value};
use tlaplus_ir::{ExprId, Program};

pub fn materialize(program: &Program, bindings: &[(usize, ExprId)]) -> Result<Vec<Value>, Error> {
    let strings = std::sync::Arc::new(std::sync::Mutex::new(program.strings.clone()));
    materialize_with(program, bindings, |values, expression| {
        Evaluator::new(program, values, &[], &[])
            .with_strings(strings.clone())
            .evaluate(expression)
    })
}

pub fn materialize_with(
    program: &Program,
    bindings: &[(usize, ExprId)],
    mut evaluate: impl FnMut(&[Value], ExprId) -> Result<Value, Error>,
) -> Result<Vec<Value>, Error> {
    use std::collections::BTreeSet;
    use tlaplus_ir::Expr;
    let mut pending = std::collections::BTreeMap::new();
    for &(slot, expression) in bindings {
        if slot >= program.constants.len() || pending.insert(slot, expression).is_some() {
            return Err(Error("duplicate or invalid constant binding".into()));
        }
    }
    if pending.len() != program.constants.len() {
        return Err(Error("unbound configuration constant".into()));
    }
    let mut dependencies = std::collections::BTreeMap::new();
    for (&slot, &expression) in &pending {
        let mut visit = vec![expression];
        let mut seen = BTreeSet::new();
        let mut constants = BTreeSet::new();
        while let Some(id) = visit.pop() {
            if !seen.insert(id) {
                continue;
            }
            let node = program
                .nodes
                .get(id)
                .ok_or_else(|| Error("invalid constant expression".into()))?;
            match &node.expr {
                Expr::Scoped { current, .. } => visit.push(*current),
                Expr::Constant(slot) => {
                    constants.insert(*slot);
                }
                Expr::Variable { .. } => {
                    return Err(Error("configuration constant depends on state".into()));
                }
                Expr::Apply { args, .. } => visit.extend(args),
                Expr::Lambda { body, .. } => visit.push(*body),
                Expr::Invoke { function, args, .. } => {
                    visit.push(*function);
                    visit.extend(args);
                }
                Expr::Call { operator, args } => {
                    visit.extend(args);
                    visit.push(
                        program
                            .operators
                            .get(*operator)
                            .ok_or_else(|| Error("invalid constant operator".into()))?
                            .body,
                    );
                }
                Expr::If {
                    condition,
                    then_expr,
                    else_expr,
                } => visit.extend([condition, then_expr, else_expr]),
                Expr::Quantify { domain, body, .. }
                | Expr::Function { domain, body, .. }
                | Expr::RecursiveFunction { domain, body, .. }
                | Expr::Choose { domain, body, .. } => {
                    visit.extend(domain);
                    visit.push(*body);
                }
                Expr::Except {
                    function,
                    key,
                    value,
                } => visit.extend([function, key, value]),
                _ => {}
            }
        }
        dependencies.insert(slot, constants);
    }
    let mut values = vec![Value::Bool(false); program.constants.len()];
    let mut completed = BTreeSet::new();
    while !pending.is_empty() {
        let Some((&slot, &expression)) = pending
            .iter()
            .find(|(slot, _)| dependencies[slot].is_subset(&completed))
        else {
            return Err(Error("cyclic configuration constants".into()));
        };
        values[slot] = evaluate(&values, expression)?;
        completed.insert(slot);
        pending.remove(&slot);
    }
    Ok(values)
}

pub fn configure(
    program: &mut Program,
    bindings: &[(usize, ExprId)],
    overrides: &[(usize, ExprId)],
    mut evaluate: impl FnMut(&Program, &[Value], ExprId) -> Result<Value, Error>,
) -> Result<Vec<Value>, Error> {
    use std::collections::BTreeMap;
    use tlaplus_ir::{Expr, Level, Operator};
    if overrides.is_empty() {
        return materialize_with(program, bindings, |values, expression| {
            evaluate(program, values, expression)
        });
    }
    let original = program.clone();
    let mut replacements = BTreeMap::new();
    let mut bodies = BTreeMap::new();
    for &(slot, expression) in overrides {
        let node = original
            .nodes
            .get(expression)
            .ok_or_else(|| Error("invalid override expression".into()))?;
        let Expr::Call { operator, args } = &node.expr else {
            return Err(Error("constant override requires an operator".into()));
        };
        let definition = original
            .operators
            .get(*operator)
            .ok_or_else(|| Error("invalid override operator".into()))?;
        if slot >= program.constants.len()
            || node.level != Level::Constant
            || !args.is_empty()
            || !definition.parameters.is_empty()
            || !bindings.contains(&(slot, expression))
            || replacements.contains_key(&slot)
        {
            return Err(Error("invalid constant override".into()));
        }
        replacements.insert(slot, program.operators.len());
        program.operators.push(Operator {
            name: format!("constant override {slot}"),
            parameters: Vec::new(),
            body: definition.body,
        });
        bodies.insert(expression, (*operator, definition.body));
    }
    for node in &mut program.nodes {
        if let Expr::Constant(slot) = &node.expr
            && let Some(&operator) = replacements.get(slot)
        {
            node.expr = Expr::Call {
                operator,
                args: Vec::new(),
            };
        }
    }
    materialize_with(&original, bindings, |values, expression| {
        let body = bodies
            .get(&expression)
            .map_or(expression, |(_, body)| *body);
        let value = evaluate(program, values, body)?;
        if let Some(&(operator, _)) = bodies.get(&expression) {
            let body = program.push(Expr::Literal(tlaplus_eval::literal(&value)), Level::Constant);
            program.operators[operator].body = body;
        }
        Ok(value)
    })
}
