//! Human-readable counterexamples: named actions, TLA+ values, and the
//! variables each step changes.

use std::io::IsTerminal;
use tla_ir::{Builtin as B, Expr, ExprId, Level, Program, Quantifier};
use tla_model::{State, Value};

/// How traces are drawn: TLA+ ASCII or mathematical symbols, and whether to
/// color changed values (on a terminal, unless `NO_COLOR` is set).
#[derive(Clone, Copy)]
pub struct Style {
    pub unicode: bool,
    pub color: bool,
}

impl Style {
    pub fn new(unicode: bool) -> Self {
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Self { unicode, color }
    }
    fn paint(self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.into()
        }
    }
}

/// Splits the next-state relation into its disjuncts, as TLC does, so every
/// step can be labeled with the action that took it. The union of the parts
/// is the original relation, so the reachable states do not change.
pub fn actions(program: &mut Program, next: &str, body: ExprId) -> Vec<(String, ExprId)> {
    let mut parts = Vec::new();
    split(program, body, &mut parts, 0);
    if parts.len() > 64 {
        return vec![(next.into(), body)];
    }
    parts
        .into_iter()
        .map(|id| (name(program, id).unwrap_or_else(|| next.into()), id))
        .collect()
}

fn split(program: &mut Program, id: ExprId, parts: &mut Vec<ExprId>, depth: usize) {
    if depth > 16 {
        parts.push(id);
        return;
    }
    match program.nodes[id].expr.clone() {
        Expr::Call { operator, args } if args.is_empty() => {
            let body = program.operators[operator].body;
            let before = parts.len();
            split(program, body, parts, depth + 1);
            if parts.len() == before + 1 && parts[before] == body {
                parts[before] = id;
            }
        }
        Expr::Apply { op: B::Or, args } => {
            for arg in args {
                split(program, arg, parts, depth + 1);
            }
        }
        // `(A \/ B) /\ C` is `(A /\ C) \/ (B /\ C)`, evaluated in the same order.
        Expr::Apply { op: B::And, args } => {
            for (index, &arg) in args.iter().enumerate() {
                let mut alternatives = Vec::new();
                split(program, arg, &mut alternatives, depth + 1);
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

/// A value in TLA+ notation.
pub fn value(value: &Value, style: Style) -> String {
    let text = tla_value_expr::stringify(&tla_value_expr::Expr::atom(value.clone()))
        .unwrap_or_else(|_| format!("{value:?}"));
    if !style.unicode {
        return text;
    }
    // Replace operators outside string literals.
    let mut result = String::new();
    let mut quoted = false;
    let mut rest = text.as_str();
    while let Some(c) = rest.chars().next() {
        let symbol = [("<<", "⟨"), (">>", "⟩"), ("|->", "↦"), (":>", "↦")]
            .into_iter()
            .find(|(ascii, _)| !quoted && rest.starts_with(ascii));
        if let Some((ascii, unicode)) = symbol {
            result.push_str(unicode);
            rest = &rest[ascii.len()..];
            continue;
        }
        if c == '"' && !result.ends_with('\\') {
            quoted = !quoted;
        }
        result.push(c);
        rest = &rest[c.len_utf8()..];
    }
    result
}

/// Prints states with their variables, marking those each step changed.
pub fn states(
    program: &Program,
    steps: &[(&State, Option<String>)],
    style: Style,
    loop_back: Option<(usize, bool)>,
) {
    let width = program.variables.iter().map(|v| v.len()).max().unwrap_or(0);
    let mut previous: Option<&State> = None;
    for (index, (state, label)) in steps.iter().enumerate() {
        let label = label.as_deref().unwrap_or("Initial predicate");
        let heading = format!("State {}: {label}", index + 1);
        println!("\n{}", style.paint("1", &heading));
        for (slot, name) in program.variables.iter().enumerate() {
            let Some(current) = state.values.get(slot) else {
                continue;
            };
            let changed = previous.is_some_and(|p| p.values.get(slot) != Some(current));
            let line = format!("{name:width$} = {}", value(current, style));
            if changed {
                println!("{} {}", style.paint("33", "*"), style.paint("33;1", &line));
            } else if previous.is_some() {
                println!("  {}", style.paint("2", &line));
            } else {
                println!("  {line}");
            }
        }
        previous = Some(state);
    }
    if let Some((target, stutter)) = loop_back {
        let text = if stutter {
            format!("\nStuttering: state {} repeats forever.", target + 1)
        } else {
            format!("\nBack to state {}: the cycle repeats forever.", target + 1)
        };
        println!("{}", style.paint("1", &text));
    }
}

/// Validates a safety counterexample and prints it.
pub fn violation(
    model: &tla_model::BoundModel,
    violation: &tla_engine::Violation<tla_model::State>,
    style: Style,
) -> Result<(), String> {
    use tla_engine::ViolationKind as K;
    if !tla_engine::validate_trace(model, violation).map_err(|e| e.to_string())? {
        return Err("invalid safety counterexample".into());
    }
    println!(
        "\n{}",
        match &violation.kind {
            K::Invariant(name) => format!("Invariant {name} is violated."),
            K::InitialProperty(name) => format!("Initial property {name} is violated."),
            K::ActionProperty(name) => format!("Action property {name} is violated."),
            K::Deadlock => "Deadlock reached.".into(),
        }
    );
    let trace = &violation.trace;
    let steps: Vec<_> = trace
        .states
        .iter()
        .enumerate()
        .map(|(i, state)| {
            let action = i
                .checked_sub(1)
                .map(|a| model.actions[trace.actions[a]].0.clone());
            (state, action)
        })
        .collect();
    states(&model.program, &steps, style, None);
    Ok(())
}
