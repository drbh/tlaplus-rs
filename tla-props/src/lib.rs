mod formula;
mod search;

pub use formula::Formula;
pub use search::{Check, check};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub atoms: BTreeSet<usize>,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub nodes: usize,
    pub initial: Vec<usize>,
    /// Atoms are evaluated on the source state and its outgoing action.
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn validate(&self) -> Result<(), String> {
        if self.initial.iter().any(|&i| i >= self.nodes)
            || self
                .edges
                .iter()
                .any(|e| e.from >= self.nodes || e.to >= self.nodes)
        {
            return Err("liveness graph references a missing state".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lasso {
    pub initial: usize,
    pub prefix: Vec<usize>,
    pub cycle: Vec<usize>,
}

impl Lasso {
    pub fn satisfies(&self, graph: &Graph, formula: &Formula) -> Result<bool, String> {
        graph.validate()?;
        if self.cycle.is_empty() || !graph.initial.contains(&self.initial) {
            return Err("lasso needs an initial state and a nonempty cycle".into());
        }
        let mut state = self.initial;
        let mut labels = Vec::new();
        for &id in &self.prefix {
            let edge = graph.edges.get(id).ok_or("missing prefix edge")?;
            if edge.from != state {
                return Err("disconnected prefix".into());
            }
            state = edge.to;
            labels.push(&edge.atoms);
        }
        let cycle_start = state;
        for &id in &self.cycle {
            let edge = graph.edges.get(id).ok_or("missing cycle edge")?;
            if edge.from != state {
                return Err("disconnected cycle".into());
            }
            state = edge.to;
            labels.push(&edge.atoms);
        }
        if state != cycle_start {
            return Err("cycle does not close".into());
        }
        Ok(evaluate(formula, &labels, self.prefix.len())[0])
    }
}

fn evaluate(formula: &Formula, labels: &[&BTreeSet<usize>], loop_at: usize) -> Vec<bool> {
    let n = labels.len();
    let next = |i| if i + 1 == n { loop_at } else { i + 1 };
    match formula {
        Formula::True => vec![true; n],
        Formula::False => vec![false; n],
        Formula::Atom(id, positive) => labels.iter().map(|l| l.contains(id) == *positive).collect(),
        Formula::Next(a) => {
            let a = evaluate(a, labels, loop_at);
            (0..n).map(|i| a[next(i)]).collect()
        }
        Formula::And(a, b) | Formula::Or(a, b) => {
            let a = evaluate(a, labels, loop_at);
            let b = evaluate(b, labels, loop_at);
            a.into_iter()
                .zip(b)
                .map(|(a, b)| {
                    if matches!(formula, Formula::And(..)) {
                        a && b
                    } else {
                        a || b
                    }
                })
                .collect()
        }
        Formula::Until(a, b) | Formula::Release(a, b) => {
            let a = evaluate(a, labels, loop_at);
            let b = evaluate(b, labels, loop_at);
            let release = matches!(formula, Formula::Release(..));
            let mut value = vec![release; n];
            loop {
                let updated: Vec<_> = (0..n)
                    .map(|i| {
                        if release {
                            b[i] && (a[i] || value[next(i)])
                        } else {
                            b[i] || (a[i] && value[next(i)])
                        }
                    })
                    .collect();
                if updated == value {
                    return value;
                }
                value = updated;
            }
        }
    }
}
