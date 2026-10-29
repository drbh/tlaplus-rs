//! Human-readable counterexamples: TLA+ values and the variables each step
//! changes.

use std::io::IsTerminal;
use tlaplus_checker::{BoundModel, State, Value, Violation, ViolationKind as K};

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

/// A value in TLA+ notation.
pub fn value(value: &Value, style: Style) -> String {
    let text = tlaplus_value_expr::stringify(&tlaplus_value_expr::Expr::atom(value.clone()))
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
    variables: &[String],
    steps: &[(&State, Option<String>)],
    style: Style,
    loop_back: Option<(usize, bool)>,
) {
    let width = variables.iter().map(|v| v.len()).max().unwrap_or(0);
    let mut previous: Option<&State> = None;
    for (index, (state, label)) in steps.iter().enumerate() {
        let label = label.as_deref().unwrap_or("Initial predicate");
        let heading = format!("State {}: {label}", index + 1);
        println!("\n{}", style.paint("1", &heading));
        for (slot, name) in variables.iter().enumerate() {
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

/// Prints a safety counterexample.
pub fn violation(model: &BoundModel, violation: &Violation<State>, style: Style) {
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
                .map(|a| model.checks.actions[trace.actions[a]].0.clone());
            (state, action)
        })
        .collect();
    states(&model.program.variables, &steps, style, None);
}
