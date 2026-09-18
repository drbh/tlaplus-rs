use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Formula {
    True,
    False,
    Atom(usize, bool),
    And(Arc<Self>, Arc<Self>),
    Or(Arc<Self>, Arc<Self>),
    Next(Arc<Self>),
    Until(Arc<Self>, Arc<Self>),
    Release(Arc<Self>, Arc<Self>),
}

impl Formula {
    pub fn atom(id: usize) -> Self {
        Self::Atom(id, true)
    }
    pub fn and(self, other: Self) -> Self {
        Self::And(self.into(), other.into())
    }
    pub fn or(self, other: Self) -> Self {
        Self::Or(self.into(), other.into())
    }
    pub fn next(self) -> Self {
        Self::Next(self.into())
    }
    pub fn until(self, other: Self) -> Self {
        Self::Until(self.into(), other.into())
    }
    pub fn eventually(self) -> Self {
        Self::True.until(self)
    }
    pub fn always(self) -> Self {
        Self::Release(Self::False.into(), self.into())
    }
    pub fn implies(self, other: Self) -> Self {
        self.negate().or(other)
    }
    pub fn negate(&self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Atom(id, positive) => Self::Atom(*id, !positive),
            Self::And(a, b) => a.negate().or(b.negate()),
            Self::Or(a, b) => a.negate().and(b.negate()),
            Self::Next(a) => a.negate().next(),
            Self::Until(a, b) => Self::Release(a.negate().into(), b.negate().into()),
            Self::Release(a, b) => a.negate().until(b.negate()),
        }
    }
    pub fn weak_fairness(enabled: usize, taken: usize) -> Self {
        Self::atom(enabled)
            .negate()
            .or(Self::atom(taken))
            .eventually()
            .always()
    }
    pub fn strong_fairness(enabled: usize, taken: usize) -> Self {
        Self::atom(enabled)
            .eventually()
            .always()
            .implies(Self::atom(taken).eventually().always())
    }
    pub(crate) fn atoms(&self, out: &mut BTreeSet<usize>) {
        match self {
            Self::Atom(id, _) => {
                out.insert(*id);
            }
            Self::And(a, b) | Self::Or(a, b) | Self::Until(a, b) | Self::Release(a, b) => {
                a.atoms(out);
                b.atoms(out);
            }
            Self::Next(a) => a.atoms(out),
            Self::True | Self::False => {}
        }
    }
    pub(crate) fn untils(&self, out: &mut BTreeSet<Self>) {
        match self {
            Self::Until(a, b) => {
                out.insert(self.clone());
                a.untils(out);
                b.untils(out);
            }
            Self::And(a, b) | Self::Or(a, b) | Self::Release(a, b) => {
                a.untils(out);
                b.untils(out);
            }
            Self::Next(a) => a.untils(out),
            _ => {}
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Branch {
    pub old: BTreeSet<Formula>,
    pub next: BTreeSet<Formula>,
}

pub(crate) fn expand(obligations: &BTreeSet<Formula>, labels: &BTreeSet<usize>) -> Vec<Branch> {
    let mut work = vec![(
        obligations.iter().cloned().collect::<Vec<_>>(),
        Branch::default(),
    )];
    let mut result = Vec::new();
    while let Some((mut todo, mut branch)) = work.pop() {
        let Some(formula) = todo.pop() else {
            result.push(branch);
            continue;
        };
        if !branch.old.insert(formula.clone()) {
            work.push((todo, branch));
            continue;
        }
        match formula {
            Formula::False => continue,
            Formula::Atom(id, positive) if labels.contains(&id) != positive => continue,
            Formula::True | Formula::Atom(..) => {}
            Formula::And(a, b) => {
                todo.push((*a).clone());
                todo.push((*b).clone());
            }
            Formula::Or(a, b) => {
                let mut alternative = todo.clone();
                alternative.push((*b).clone());
                work.push((alternative, branch.clone()));
                todo.push((*a).clone());
            }
            Formula::Next(a) => {
                branch.next.insert((*a).clone());
            }
            Formula::Until(a, b) => {
                let mut later = branch.clone();
                later.next.insert(Formula::Until(a.clone(), b.clone()));
                let mut alternative = todo.clone();
                alternative.push((*a).clone());
                work.push((alternative, later));
                todo.push((*b).clone());
            }
            Formula::Release(a, b) => {
                todo.push((*b).clone());
                let mut later = branch.clone();
                later.next.insert(Formula::Release(a.clone(), b));
                work.push((todo.clone(), later));
                todo.push((*a).clone());
            }
        }
        work.push((todo, branch));
    }
    result
}
