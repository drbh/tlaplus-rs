use crate::{Error, Formula, Graph, Lasso, formula::expand};
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

/// Searches the product of `graph` and the negated `property` for a lasso the
/// fairness `assumptions` allow. Fairness is checked on strongly connected
/// components (Streett acceptance), as TLC does, rather than in the tableau,
/// which grows exponentially with it.
pub fn check(
    graph: &Graph,
    assumptions: &[Formula],
    property: &Formula,
    limit: NonZeroUsize,
) -> Result<Check, Error> {
    graph.validate()?;
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
    let search = match Search::build(
        graph,
        &formula,
        untils.into_iter().collect(),
        fairness,
        limit,
    ) {
        Ok(search) => search,
        Err(product_states) => return Ok(Check::StateLimit { product_states }),
    };
    let product_states = search.product.node_count();
    match search.violation()? {
        Some(lasso) => {
            if !lasso.satisfies(graph, &complete)? {
                return Err(Error::UnverifiedLasso);
            }
            Ok(Check::Violated {
                lasso,
                product_states,
            })
        }
        None => Ok(Check::Holds { product_states }),
    }
}

/// The product of a behavior graph and a formula's tableau, with the
/// breadth-first tree that reaches each product state.
struct Search<'g> {
    graph: &'g Graph,
    product: Product,
    parents: Vec<Parent>,
    untils: Vec<Formula>,
    fairness: Vec<Fair>,
}

/// What a strongly connected component offers as a counterexample.
enum Witness {
    /// Edges that together satisfy every acceptance condition.
    Accepting(Vec<EdgeIndex>),
    /// The component is not fair; some edges may be removed to try again.
    Unfair(HashSet<EdgeIndex>),
}

impl<'g> Search<'g> {
    /// Explores the product breadth-first from the initial states. `Err` carries
    /// the number of product states when `limit` was reached.
    fn build(
        graph: &'g Graph,
        formula: &Formula,
        untils: Vec<Formula>,
        fairness: Vec<Fair>,
        limit: NonZeroUsize,
    ) -> Result<Self, usize> {
        let mut search = Self {
            graph,
            product: Product::new(),
            parents: Vec::new(),
            untils,
            fairness,
        };
        let mut index = HashMap::new();
        let mut queue = VecDeque::new();
        let mut outgoing = vec![Vec::new(); graph.nodes];
        for (id, edge) in graph.edges.iter().enumerate() {
            outgoing[edge.from].push(id);
        }
        for &initial in &graph.initial {
            let key = (initial, BTreeSet::from([formula.clone()]));
            if !index.contains_key(&key) {
                queue.push_back(search.add(key, &mut index, limit)?);
            }
        }
        while let Some(from) = queue.pop_front() {
            let (state, obligations) = search.product[from].clone();
            for &id in &outgoing[state] {
                let edge = &graph.edges[id];
                for branch in expand(&obligations, &edge.atoms) {
                    let accepts = search
                        .untils
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
                    let (to, fresh) = match index.get(&key) {
                        Some(&node) => (node, false),
                        None => {
                            let node = search.add(key, &mut index, limit)?;
                            queue.push_back(node);
                            (node, true)
                        }
                    };
                    let link = search.product.add_edge(
                        from,
                        to,
                        ProductEdge {
                            original: id,
                            accepts,
                        },
                    );
                    if fresh {
                        search.parents[to.index()] = Some((from, link));
                    }
                }
            }
        }
        Ok(search)
    }

    fn add(
        &mut self,
        key: (usize, BTreeSet<Formula>),
        index: &mut HashMap<(usize, BTreeSet<Formula>), NodeIndex>,
        limit: NonZeroUsize,
    ) -> Result<NodeIndex, usize> {
        if self.product.node_count() >= limit.get() {
            return Err(self.product.node_count());
        }
        let node = self.product.add_node(key.clone());
        index.insert(key, node);
        self.parents.push(None);
        Ok(node)
    }

    fn labels(&self, edge: EdgeIndex) -> &BTreeSet<usize> {
        &self.graph.edges[self.product[edge].original].atoms
    }

    /// The first fair, accepting cycle in any strongly connected component,
    /// as a lasso of the original graph's edges. Unfair components are
    /// re-split without the edges that break fairness.
    fn violation(&self) -> Result<Option<Lasso>, Error> {
        let mut pending: Vec<(Vec<NodeIndex>, HashSet<EdgeIndex>)> = kosaraju_scc(&self.product)
            .into_iter()
            .map(|component| (component, HashSet::new()))
            .collect();
        while let Some((component, removed)) = pending.pop() {
            let members: HashSet<_> = component.iter().copied().collect();
            let internal: Vec<_> = component
                .iter()
                .flat_map(|&n| self.product.edges(n))
                .filter(|e| members.contains(&e.target()) && !removed.contains(&e.id()))
                .map(|e| e.id())
                .collect();
            if internal.is_empty() {
                continue;
            }
            match self.witness(&internal, &removed) {
                None => {}
                Some(Witness::Accepting(witness)) => {
                    return Ok(Some(self.lasso(witness, &members, &removed)));
                }
                Some(Witness::Unfair(restricted)) if restricted.len() > removed.len() => {
                    let kept = EdgeFiltered::from_fn(&self.product, |e| {
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
                Some(Witness::Unfair(_)) => {}
            }
        }
        Ok(None)
    }

    /// Edges of a component that satisfy every `until` acceptance and every
    /// fairness assumption, or why the component cannot.
    fn witness(&self, internal: &[EdgeIndex], removed: &HashSet<EdgeIndex>) -> Option<Witness> {
        let mut witness = Vec::new();
        for acceptance in 0..self.untils.len() {
            let edge = internal
                .iter()
                .find(|&&e| self.product[e].accepts.contains(&acceptance))?;
            witness.push(*edge);
        }
        let mut fair = true;
        let mut restricted = removed.clone();
        for &Fair {
            strong,
            enabled,
            taken,
        } in &self.fairness
        {
            let taken_edge = internal.iter().find(|&&e| self.labels(e).contains(&taken));
            let idle = internal
                .iter()
                .find(|&&e| !self.labels(e).contains(&enabled));
            match (strong, taken_edge, idle) {
                (_, Some(&edge), _) => witness.push(edge),
                (false, None, Some(&edge)) => witness.push(edge),
                (false, None, None) => fair = false,
                // Strong fairness can only hold where the action is never enabled.
                (true, None, _) => {
                    let enabled_edges: Vec<_> = internal
                        .iter()
                        .copied()
                        .filter(|&e| self.labels(e).contains(&enabled))
                        .collect();
                    if !enabled_edges.is_empty() {
                        restricted.extend(enabled_edges);
                        fair = false;
                    }
                }
            }
        }
        if !fair {
            return Some(Witness::Unfair(restricted));
        }
        if witness.is_empty() {
            witness.push(internal[0]);
        }
        Some(Witness::Accepting(witness))
    }

    /// A cycle through every witness edge within `members`, reached from an
    /// initial state along the breadth-first tree.
    fn lasso(
        &self,
        witness: Vec<EdgeIndex>,
        members: &HashSet<NodeIndex>,
        removed: &HashSet<EdgeIndex>,
    ) -> Lasso {
        let allowed = |edge: EdgeIndex| !removed.contains(&edge);
        let base = self.product.edge_endpoints(witness[0]).unwrap().0;
        let mut current = base;
        let mut cycle = Vec::new();
        for edge in witness {
            let (from, to) = self.product.edge_endpoints(edge).unwrap();
            cycle.extend(path(&self.product, current, from, members, &allowed));
            cycle.push(edge);
            current = to;
        }
        cycle.extend(path(&self.product, current, base, members, &allowed));
        let mut prefix = Vec::new();
        current = base;
        while let Some((parent, edge)) = self.parents[current.index()] {
            prefix.push(edge);
            current = parent;
        }
        prefix.reverse();
        Lasso {
            initial: self.product[current].0,
            prefix: prefix
                .into_iter()
                .map(|e| self.product[e].original)
                .collect(),
            cycle: cycle
                .into_iter()
                .map(|e| self.product[e].original)
                .collect(),
        }
    }
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
