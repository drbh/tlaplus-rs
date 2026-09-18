use crate::{Formula, Graph, Lasso, formula::expand};
use petgraph::{
    algo::kosaraju_scc,
    graph::{DiGraph, EdgeIndex, NodeIndex},
    visit::{EdgeFiltered, EdgeRef},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    num::NonZeroUsize,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Check {
    Holds { product_states: usize },
    Violated { lasso: Lasso, product_states: usize },
    StateLimit { product_states: usize },
}

#[derive(Clone)]
struct ProductEdge {
    original: usize,
    accepts: BTreeSet<usize>,
}
type Product = DiGraph<(usize, BTreeSet<Formula>), ProductEdge>;
type Parent = Option<(NodeIndex, EdgeIndex)>;

pub fn check(
    graph: &Graph,
    assumptions: &[Formula],
    property: &Formula,
    limit: NonZeroUsize,
) -> Result<Check, String> {
    graph.validate()?;
    // Fairness is checked on strongly connected components (Streett acceptance),
    // as TLC does, rather than in the tableau, which grows exponentially with it.
    let mut fairness = Vec::new();
    let mut tableau = Vec::new();
    for assumption in assumptions {
        conjuncts(assumption, &mut fairness, &mut tableau);
    }
    let complete = assumptions
        .iter()
        .cloned()
        .fold(property.negate(), Formula::and);
    let formula = tableau.into_iter().fold(property.negate(), Formula::and);
    let mut untils = BTreeSet::new();
    formula.untils(&mut untils);
    let untils: Vec<_> = untils.into_iter().collect();
    let mut product = Product::new();
    let mut index = HashMap::new();
    let mut queue = VecDeque::new();
    let mut parents: Vec<Parent> = Vec::new();
    let mut outgoing = vec![Vec::new(); graph.nodes];
    for (id, edge) in graph.edges.iter().enumerate() {
        outgoing[edge.from].push(id);
    }
    for &initial in &graph.initial {
        let key = (initial, BTreeSet::from([formula.clone()]));
        if index.contains_key(&key) {
            continue;
        }
        if product.node_count() >= limit.get() {
            return Ok(Check::StateLimit {
                product_states: product.node_count(),
            });
        }
        let node = product.add_node(key.clone());
        index.insert(key, node);
        queue.push_back(node);
        parents.push(None);
    }
    while let Some(from) = queue.pop_front() {
        let (state, obligations) = product[from].clone();
        for &id in &outgoing[state] {
            let edge = &graph.edges[id];
            for branch in expand(&obligations, &edge.atoms) {
                let accepts = untils
                    .iter()
                    .enumerate()
                    .filter_map(|(i, until)| {
                        let Formula::Until(_, right) = until else {
                            unreachable!()
                        };
                        (!branch.old.contains(until) || branch.old.contains(right.as_ref()))
                            .then_some(i)
                    })
                    .collect();
                let key = (edge.to, branch.next);
                let (to, fresh) = if let Some(&node) = index.get(&key) {
                    (node, false)
                } else {
                    if product.node_count() >= limit.get() {
                        return Ok(Check::StateLimit {
                            product_states: product.node_count(),
                        });
                    }
                    let node = product.add_node(key.clone());
                    index.insert(key, node);
                    queue.push_back(node);
                    parents.push(None);
                    (node, true)
                };
                let link = product.add_edge(
                    from,
                    to,
                    ProductEdge {
                        original: id,
                        accepts,
                    },
                );
                if fresh {
                    parents[to.index()] = Some((from, link));
                }
            }
        }
    }
    let labels = |edge: EdgeIndex| &graph.edges[product[edge].original].atoms;
    let mut pending: Vec<(Vec<NodeIndex>, HashSet<EdgeIndex>)> = kosaraju_scc(&product)
        .into_iter()
        .map(|component| (component, HashSet::new()))
        .collect();
    while let Some((component, removed)) = pending.pop() {
        let members: HashSet<_> = component.iter().copied().collect();
        let internal: Vec<_> = component
            .iter()
            .flat_map(|&n| product.edges(n))
            .filter(|e| members.contains(&e.target()) && !removed.contains(&e.id()))
            .map(|e| e.id())
            .collect();
        if internal.is_empty() {
            continue;
        }
        let mut witness = Vec::new();
        for acceptance in 0..untils.len() {
            if let Some(&edge) = internal
                .iter()
                .find(|&&e| product[e].accepts.contains(&acceptance))
            {
                witness.push(edge);
            } else {
                break;
            }
        }
        if witness.len() != untils.len() {
            continue;
        }
        let mut fair = true;
        let mut restricted = removed.clone();
        for &Fair {
            strong,
            enabled,
            taken,
        } in &fairness
        {
            let taken_edge = internal.iter().find(|&&e| labels(e).contains(&taken));
            let idle = internal.iter().find(|&&e| !labels(e).contains(&enabled));
            match (strong, taken_edge, idle) {
                (_, Some(&edge), _) => witness.push(edge),
                (false, None, Some(&edge)) => witness.push(edge),
                (false, None, None) => fair = false,
                // Strong fairness can only hold where the action is never enabled.
                (true, None, _) => {
                    let enabled_edges: Vec<_> = internal
                        .iter()
                        .copied()
                        .filter(|&e| labels(e).contains(&enabled))
                        .collect();
                    if !enabled_edges.is_empty() {
                        restricted.extend(enabled_edges);
                        fair = false;
                    }
                }
            }
        }
        if !fair {
            if restricted.len() > removed.len() {
                let kept = EdgeFiltered::from_fn(&product, |e| {
                    members.contains(&e.source())
                        && members.contains(&e.target())
                        && !restricted.contains(&e.id())
                });
                pending.extend(
                    kosaraju_scc(&kept)
                        .into_iter()
                        .filter(|c| c.iter().all(|n| members.contains(n)))
                        .map(|c| (c, restricted.clone())),
                );
            }
            continue;
        }
        if witness.is_empty() {
            witness.push(internal[0]);
        }
        let members: HashSet<_> = members;
        let allowed = |edge: EdgeIndex| !removed.contains(&edge);
        let base = product.edge_endpoints(witness[0]).unwrap().0;
        let mut current = base;
        let mut cycle = Vec::new();
        for edge in witness {
            let (from, to) = product.edge_endpoints(edge).unwrap();
            cycle.extend(path(&product, current, from, &members, &allowed));
            cycle.push(edge);
            current = to;
        }
        cycle.extend(path(&product, current, base, &members, &allowed));
        let mut prefix = Vec::new();
        current = base;
        while let Some((parent, edge)) = parents[current.index()] {
            prefix.push(edge);
            current = parent;
        }
        prefix.reverse();
        let lasso = Lasso {
            initial: product[current].0,
            prefix: prefix.into_iter().map(|e| product[e].original).collect(),
            cycle: cycle.into_iter().map(|e| product[e].original).collect(),
        };
        if !lasso.satisfies(graph, &complete)? {
            return Err("constructed lasso failed independent temporal validation".into());
        }
        return Ok(Check::Violated {
            lasso,
            product_states: product.node_count(),
        });
    }
    Ok(Check::Holds {
        product_states: product.node_count(),
    })
}

fn path(
    graph: &Product,
    start: NodeIndex,
    target: NodeIndex,
    members: &HashSet<NodeIndex>,
    allowed: &impl Fn(EdgeIndex) -> bool,
) -> Vec<EdgeIndex> {
    let mut queue = VecDeque::from([start]);
    let mut parents = HashMap::from([(start, None)]);
    while let Some(node) = queue.pop_front() {
        if node == target {
            break;
        }
        for edge in graph.edges(node) {
            if members.contains(&edge.target())
                && allowed(edge.id())
                && !parents.contains_key(&edge.target())
            {
                parents.insert(edge.target(), Some((node, edge.id())));
                queue.push_back(edge.target());
            }
        }
    }
    let mut result = Vec::new();
    let mut node = target;
    while let Some((parent, edge)) = parents[&node] {
        result.push(edge);
        node = parent;
    }
    result.reverse();
    result
}

#[derive(Clone, Copy)]
struct Fair {
    strong: bool,
    enabled: usize,
    taken: usize,
}

/// Splits an assumption into fairness constraints and formulas for the tableau.
fn conjuncts(formula: &Formula, fairness: &mut Vec<Fair>, tableau: &mut Vec<Formula>) {
    use Formula::And;
    if let And(a, b) = formula {
        conjuncts(a, fairness, tableau);
        conjuncts(b, fairness, tableau);
        return;
    }
    let mut atoms = BTreeSet::new();
    formula.atoms(&mut atoms);
    if let [enabled, taken] = atoms.into_iter().collect::<Vec<_>>()[..] {
        for (enabled, taken) in [(enabled, taken), (taken, enabled)] {
            for strong in [false, true] {
                let expected = if strong {
                    Formula::strong_fairness(enabled, taken)
                } else {
                    Formula::weak_fairness(enabled, taken)
                };
                if expected == *formula {
                    fairness.push(Fair {
                        strong,
                        enabled,
                        taken,
                    });
                    return;
                }
            }
        }
    }
    tableau.push(formula.clone());
}
