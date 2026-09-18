use crate::Error;
use std::collections::{BTreeSet, HashMap};
use tla_engine::{Completion, Report, TransitionSystem};
use tla_ir::ExprId;
use tla_model::{BoundModel, State};
use tla_props::{Edge, Formula, Graph, Lasso};

#[derive(Clone, Copy)]
pub struct Atom {
    pub predicate: ExprId,
    pub enabled: bool,
}

/// Under VIEW or SYMMETRY the graph is TLC's quotient graph: nodes are
/// representatives, and each edge is labeled by an actual transition into the
/// target's class.
fn supported(report: &Report<State>) -> Result<(), Error> {
    if !matches!(report.completion, Completion::Complete) {
        return Err(Error::IncompleteExploration);
    }
    Ok(())
}

fn key(model: &BoundModel, state: &State) -> Result<<BoundModel as TransitionSystem>::Key, Error> {
    model.key(state).map_err(Error::from)
}

fn enabled(model: &BoundModel, state: &State, atoms: &[Atom]) -> Result<BTreeSet<usize>, Error> {
    let mut result = BTreeSet::new();
    for (i, atom) in atoms.iter().enumerate().filter(|(_, a)| a.enabled) {
        let mut action = model.clone();
        action.checks.actions = vec![("enabled".into(), atom.predicate)];
        if !action.successors(state).map_err(Error::from)?.is_empty() {
            result.insert(i);
        }
    }
    Ok(result)
}

fn labels(
    model: &BoundModel,
    from: &State,
    to: &State,
    atoms: &[Atom],
    enabled: &BTreeSet<usize>,
) -> Result<BTreeSet<usize>, Error> {
    let mut result = enabled.clone();
    for (i, atom) in atoms.iter().enumerate().filter(|(_, a)| !a.enabled) {
        if model
            .evaluate_at(atom.predicate, from, to, from.depth)
            .map_err(Error::from)?
            .boolean()
            .map_err(Error::from)?
        {
            result.insert(i);
        }
    }
    Ok(result)
}

pub fn build(model: &BoundModel, report: &Report<State>, atoms: &[Atom]) -> Result<Graph, Error> {
    supported(report)?;
    let enabled = report
        .nodes
        .iter()
        .map(|n| enabled(model, &n.state, atoms))
        .collect::<Result<Vec<_>, _>>()?;
    let mut graph = Graph {
        nodes: report.nodes.len(),
        initial: Vec::new(),
        edges: Vec::new(),
    };
    let indices = report
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| Ok((key(model, &n.state)?, i)))
        .collect::<Result<HashMap<_, _>, Error>>()?;
    let mut initial = BTreeSet::new();
    for state in model.initial_states().map_err(Error::from)? {
        if model.state_allowed(&state).map_err(Error::from)? {
            initial.insert(
                *indices
                    .get(&key(model, &state)?)
                    .ok_or(Error::InconsistentReport(
                        "behavior report omits an initial state",
                    ))?,
            );
        }
    }
    graph.initial = initial.into_iter().collect();
    for (from, source) in report.nodes.iter().enumerate() {
        let stutter = labels(model, &source.state, &source.state, atoms, &enabled[from])?;
        let mut edges = BTreeSet::from([(from, stutter)]);
        for transition in model.successors(&source.state).map_err(Error::from)? {
            if model
                .state_allowed(&transition.state)
                .map_err(Error::from)?
                && model
                    .step_allowed(&source.state, &transition.state)
                    .map_err(Error::from)?
            {
                let to = *indices.get(&key(model, &transition.state)?).ok_or(
                    Error::InconsistentReport("behavior report omits a reachable successor"),
                )?;
                let atoms = labels(
                    model,
                    &source.state,
                    &transition.state,
                    atoms,
                    &enabled[from],
                )?;
                edges.insert((to, atoms));
            }
        }
        for (to, atoms) in edges {
            graph.edges.push(Edge { from, to, atoms });
        }
    }
    Ok(graph)
}

pub fn validate(
    model: &BoundModel,
    report: &Report<State>,
    graph: &Graph,
    lasso: &Lasso,
    atoms: &[Atom],
    assumptions: &[Formula],
    property: &Formula,
) -> Result<bool, Error> {
    supported(report)?;
    if graph.nodes != report.nodes.len()
        || lasso.satisfies(graph, property).map_err(Error::Search)?
    {
        return Ok(false);
    }
    for assumption in assumptions {
        if !lasso.satisfies(graph, assumption).map_err(Error::Search)? {
            return Ok(false);
        }
    }
    let initial = &report
        .nodes
        .get(lasso.initial)
        .ok_or(Error::InconsistentReport("missing initial state"))?
        .state;
    let initial_key = key(model, initial)?;
    let mut initial_found = false;
    for state in model.initial_states().map_err(Error::from)? {
        initial_found |= key(model, &state)? == initial_key;
    }
    if !initial_found || !model.state_allowed(initial).map_err(Error::from)? {
        return Ok(false);
    }
    for &id in lasso.prefix.iter().chain(&lasso.cycle) {
        let edge = &graph.edges[id];
        let from = &report.nodes[edge.from].state;
        let to = &report.nodes[edge.to].state;
        if !model.state_allowed(from).map_err(Error::from)?
            || !model.state_allowed(to).map_err(Error::from)?
        {
            return Ok(false);
        }
        // Some actual step into the target's class carries the edge's labels.
        let enabled = enabled(model, from, atoms)?;
        let target = key(model, to)?;
        let mut witnessed =
            edge.from == edge.to && labels(model, from, from, atoms, &enabled)? == edge.atoms;
        for transition in model.successors(from).map_err(Error::from)? {
            if witnessed {
                break;
            }
            witnessed = key(model, &transition.state)? == target
                && model
                    .step_allowed(from, &transition.state)
                    .map_err(Error::from)?
                && labels(model, from, &transition.state, atoms, &enabled)? == edge.atoms;
        }
        if !witnessed {
            return Ok(false);
        }
    }
    Ok(true)
}
