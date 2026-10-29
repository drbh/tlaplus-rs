//! Splits the next-state relation into named actions so every step of a
//! counterexample can be labeled with the action that took it.
use tlaplus_ir::{Builtin as B, Expr, ExprId, Level, Program, Quantifier};

/// Splits the next-state relation into its disjuncts, as TLC does, so every
/// step can be labeled with the action that took it. The union of the parts
/// is the original relation, so the reachable states do not change.
pub fn split(program: &mut Program, next: &str, body: ExprId) -> Vec<(String, ExprId)> {
    let mut parts = Vec::new();
    disjuncts(program, body, &mut parts, 0);
    if parts.len() > 64 {
        return vec![(next.into(), body)];
    }
    parts
        .into_iter()
        .map(|id| (name(program, id).unwrap_or_else(|| next.into()), id))
        .collect()
}

fn disjuncts(program: &mut Program, id: ExprId, parts: &mut Vec<ExprId>, depth: usize) {
    if depth > 16 {
        parts.push(id);
        return;
    }
    match program.nodes[id].expr.clone() {
        Expr::Call { operator, args } if args.is_empty() => {
            let body = program.operators[operator].body;
            let before = parts.len();
            disjuncts(program, body, parts, depth + 1);
            if parts.len() == before + 1 && parts[before] == body {
                parts[before] = id;
            }
        }
        Expr::Apply { op: B::Or, args } => {
            for arg in args {
                disjuncts(program, arg, parts, depth + 1);
            }
        }
        // `(A \/ B) /\ C` is `(A /\ C) \/ (B /\ C)`, evaluated in the same order.
        Expr::Apply { op: B::And, args } => {
            for (index, &arg) in args.iter().enumerate() {
                let mut alternatives = Vec::new();
                disjuncts(program, arg, &mut alternatives, depth + 1);
                if alternatives.len() < 2 {
                    continue;
                }
                let level = program.nodes[id].level.max(Level::Action);
                for alternative in alternatives {
                    let mut args = args.clone();
                    args[index] = alternative;
                    let expr = Expr::Apply { op: B::And, args };
                    parts.push(program.push(expr, level));
                }
                return;
            }
            parts.push(id);
        }
        _ => parts.push(id),
    }
}

/// The user operator an action calls, looking through quantifiers and conjunctions.
fn name(program: &Program, id: ExprId) -> Option<String> {
    match &program.nodes[id].expr {
        Expr::Call { operator, .. } => {
            let name = &program.operators[*operator].name;
            let name = name.split('@').next().unwrap_or(name);
            (!name.starts_with("__") && !name.is_empty()).then(|| name.to_owned())
        }
        Expr::Quantify {
            kind: Quantifier::Exists,
            body,
            ..
        } => name(program, *body),
        Expr::Apply { op: B::And, args } => args.iter().find_map(|&arg| name(program, arg)),
        _ => None,
    }
}
