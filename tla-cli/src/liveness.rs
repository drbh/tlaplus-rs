use std::num::NonZeroUsize;
use tla_engine::Report;
use tla_model::{BoundModel, State};
use tla_props::Check;
use tla_temporal::Instantiated;

pub fn check(
    model: &BoundModel,
    report: &Report<State>,
    live: &Instantiated,
    style: crate::trace::Style,
) -> Result<bool, String> {
    let graph = tla_behavior::build(model, report, &live.atoms)?;
    for (name, property) in &live.properties {
        match tla_props::check(
            &graph,
            &live.assumptions,
            property,
            NonZeroUsize::new(1_000_000).unwrap(),
        )? {
            Check::Holds { .. } => println!("Temporal property {name} passed"),
            Check::Violated { lasso, .. } => {
                if !tla_behavior::validate(
                    model,
                    report,
                    &graph,
                    &lasso,
                    &live.atoms,
                    &live.assumptions,
                    property,
                )? {
                    return Err("invalid liveness counterexample".into());
                }
                println!("\nTemporal property {name} violated");
                let mut steps = vec![(&report.nodes[lasso.initial].state, None)];
                let edges = lasso
                    .prefix
                    .iter()
                    .chain(&lasso.cycle[..lasso.cycle.len() - 1]);
                for edge in edges {
                    let state = &report.nodes[graph.edges[*edge].to].state;
                    steps.push((state, Some("Next step".to_owned())));
                }
                let stutter = lasso
                    .cycle
                    .iter()
                    .all(|edge| graph.edges[*edge].from == graph.edges[*edge].to);
                let back = Some((lasso.prefix.len(), stutter));
                crate::trace::states(&model.program, &steps, style, back);
                return Ok(false);
            }
            Check::StateLimit { .. } => {
                return Err("liveness product limit reached; result is incomplete".into());
            }
        }
    }
    Ok(true)
}

/// How many explored states (state predicates) or recorded steps (action
/// predicates) satisfy each `_POSSIBLE` predicate, as TLC's `_Possible` counts.
pub fn possible_counts(
    model: &BoundModel,
    report: &Report<State>,
    predicates: &[(String, tla_ir::ExprId)],
) -> Result<Vec<(String, usize)>, String> {
    let holds = |from: &State, to: &State, id| -> Result<bool, String> {
        model
            .evaluate_at(id, from, to, from.depth)
            .and_then(|value| value.boolean().map_err(Into::into))
            .map_err(|e| e.to_string())
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
) -> Result<Option<String>, String> {
    let counts = possible_counts(model, report, predicates)?;
    Ok(counts
        .into_iter()
        .find(|(_, n)| *n == 0)
        .map(|(name, _)| name))
}
