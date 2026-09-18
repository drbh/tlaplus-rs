use crate::{Error, Liveness};
use std::num::NonZeroUsize;
use tla_engine::Report;
use tla_model::{BoundModel, State};
use tla_props::Check;
use tla_temporal::Instantiated;

/// Checks each temporal property in order, stopping at the first violation.
pub fn check(
    model: &BoundModel,
    report: &Report<State>,
    live: &Instantiated,
) -> Result<Vec<(String, Liveness)>, Error> {
    let graph = tla_temporal::behavior::build(model, report, &live.atoms)?;
    let mut verdicts = Vec::new();
    for (name, property) in &live.properties {
        match tla_props::check(
            &graph,
            &live.assumptions,
            property,
            NonZeroUsize::new(1_000_000).unwrap(),
        )
        .map_err(tla_temporal::Error::Search)?
        {
            Check::Holds { .. } => verdicts.push((name.clone(), Liveness::Holds)),
            Check::Violated { lasso, .. } => {
                if !tla_temporal::behavior::validate(
                    model,
                    report,
                    &graph,
                    &lasso,
                    &live.atoms,
                    &live.assumptions,
                    property,
                )? {
                    return Err(tla_temporal::Error::InconsistentReport(
                        "invalid liveness counterexample",
                    )
                    .into());
                }
                let mut steps = vec![report.nodes[lasso.initial].state.clone()];
                let edges = lasso
                    .prefix
                    .iter()
                    .chain(&lasso.cycle[..lasso.cycle.len() - 1]);
                for edge in edges {
                    steps.push(report.nodes[graph.edges[*edge].to].state.clone());
                }
                let stutter = lasso
                    .cycle
                    .iter()
                    .all(|edge| graph.edges[*edge].from == graph.edges[*edge].to);
                let loop_back = (lasso.prefix.len(), stutter);
                verdicts.push((name.clone(), Liveness::Violated { steps, loop_back }));
                return Ok(verdicts);
            }
            Check::StateLimit { .. } => {
                return Err(tla_temporal::Error::Search(
                    "liveness product limit reached; result is incomplete".into(),
                )
                .into());
            }
        }
    }
    Ok(verdicts)
}

/// How many explored states (state predicates) or recorded steps (action
/// predicates) satisfy each `_POSSIBLE` predicate, as TLC's `_Possible` counts.
pub fn possible_counts(
    model: &BoundModel,
    report: &Report<State>,
    predicates: &[(String, tla_ir::ExprId)],
) -> Result<Vec<(String, usize)>, Error> {
    let holds = |from: &State, to: &State, id| -> Result<bool, Error> {
        Ok(model
            .evaluate_at(id, from, to, from.depth)
            .and_then(|value| value.boolean().map_err(Into::into))?)
    };
    let mut counts = Vec::new();
    for (name, id) in predicates {
        let mut count = 0;
        // TLC counts every generated state or step, including revisits.
        let state = model.program.nodes[*id].level <= tla_ir::Level::State;
        for node in report.nodes.iter().filter(|n| n.parent.is_none()) {
            if state {
                count += usize::from(holds(&node.state, &node.state, *id)?);
            }
        }
        for edge in &report.edges {
            let (from, to) = (&report.nodes[edge.from].state, &report.nodes[edge.to].state);
            count += usize::from(if state {
                holds(to, to, *id)?
            } else {
                holds(from, to, *id)?
            });
        }
        counts.push((name.clone(), count));
    }
    Ok(counts)
}

/// The first `_POSSIBLE` predicate that no explored state or recorded step satisfies.
pub fn unwitnessed(
    model: &BoundModel,
    report: &Report<State>,
    predicates: &[(String, tla_ir::ExprId)],
) -> Result<Option<String>, Error> {
    let counts = possible_counts(model, report, predicates)?;
    Ok(counts
        .into_iter()
        .find(|(_, n)| *n == 0)
        .map(|(name, _)| name))
}
