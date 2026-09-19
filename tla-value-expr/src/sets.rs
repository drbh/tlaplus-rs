use crate::{Error, Expr, FunctionKind, InfiniteSet, SetOp, Value};
use tla_values::{Atom, SetExpr};

impl Expr {
    pub fn subset_of(&self, other: &Self, limit: usize) -> Result<bool, Error> {
        match (self, other) {
            (
                Self::Subsets {
                    base: a,
                    size: None,
                },
                Self::Subsets {
                    base: b,
                    size: None,
                },
            ) => a.subset_of(b, limit),
            (Self::Interval(a, b), Self::Interval(c, d)) => Ok(a > b || (c <= a && b <= d)),
            _ => {
                for value in self.enumerate(limit)? {
                    if !other.contains(&value)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    pub fn contains(&self, value: &Self) -> Result<bool, Error> {
        if let Self::Canonical(model @ Value::Model(_)) = value
            && self.members_have_shape()
        {
            return Self::model_membership(model);
        }
        match self {
            Self::Canonical(v) => v.contains(&value.normalize(100_000)?),
            Self::Set(values) => {
                for member in values {
                    if member.semantic_eq(value)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Interval(low, high) if high < low => Ok(false),
            Self::Interval(low, high) => match value {
                Self::Canonical(Value::Int(n)) => Ok(low <= n && n <= high),
                _ => Err(Error::Type("integer")),
            },
            Self::Binary(SetOp::Union, a, b) => Ok(a.contains(value)? || b.contains(value)?),
            Self::Binary(SetOp::Intersection, a, b) => Ok(a.contains(value)? && b.contains(value)?),
            Self::Binary(SetOp::Difference, a, b) => Ok(a.contains(value)? && !b.contains(value)?),
            Self::Subsets { base, size } => {
                if let Some(k) = size {
                    if *k < 0 || *k as usize > base.size()? {
                        return Ok(false);
                    }
                    if value.size()? != *k as usize {
                        return Ok(false);
                    }
                }
                for member in value.enumerate(100_000)? {
                    if !base.contains(&member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Functions { domain, range } => {
                let entries = value.entries()?;
                if !domain.semantic_eq(&value.domain()?)? {
                    return Ok(false);
                }
                for (_, member) in entries {
                    if !range.contains(&member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Product(sets) => {
                let values = match value.sequence() {
                    Ok(v) => v,
                    Err(_) => {
                        let entries = value.entries()?;
                        if entries.iter().all(|(key, _)| matches!(key, Value::Int(_))) {
                            return Ok(false);
                        }
                        return Err(Error::Type("tuple"));
                    }
                };
                if sets.len() != values.len() {
                    return Ok(false);
                }
                for (set, member) in sets.iter().zip(values) {
                    if !set.contains(&member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Records(fields) => {
                if matches!(
                    value,
                    Self::Function {
                        kind: FunctionKind::Interval(_),
                        ..
                    }
                ) || matches!(value,Self::Lambda(domain) if matches!(domain.as_ref(),Self::Interval(..)))
                {
                    return Err(Error::Type("record"));
                }
                let entries = value.entries()?;
                if entries.iter().any(|(k, _)| !matches!(k, Value::String(_))) {
                    return Err(Error::Type("record"));
                }
                if entries.len() != fields.len() {
                    return Ok(false);
                }
                for (name, set) in fields {
                    let Some((_, member)) = entries
                        .iter()
                        .find(|(k, _)| k == &Value::String(name.clone()))
                    else {
                        return Ok(false);
                    };
                    if !set.contains(member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Union(base) => {
                for set in base.enumerate(100_000)? {
                    if set.contains(value)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Filter {
                base,
                excluded,
                tuple,
            } => {
                if !base.contains(value)? {
                    return Ok(false);
                }
                let candidate = if *tuple {
                    value.sequence()?.last().cloned().ok_or(Error::Undefined)?
                } else {
                    value.clone()
                };
                Ok(!candidate.semantic_eq(excluded)?)
            }
            Self::Infinite(kind) => match (kind, value) {
                (InfiniteSet::Any, _) => Ok(true),
                (InfiniteSet::Integers, Self::Canonical(Value::Int(_))) => Ok(true),
                (InfiniteSet::Naturals, Self::Canonical(Value::Int(n))) => Ok(*n >= 0),
                (InfiniteSet::Strings, Self::Canonical(Value::String(_))) => Ok(true),
                _ => Err(Error::Type("infinite set member")),
            },
            Self::Sequences(base) => {
                for member in value.sequence()? {
                    if !base.contains(&member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Err(Error::Type("set")),
        }
    }
    /// `SUBSET S`, or the subsets of `S` of one size.
    fn enumerate_subsets(base: &Self, size: Option<i32>, limit: usize) -> Result<Vec<Self>, Error> {
        Ok({
            if size.is_some_and(|k| k < 0) {
                return Ok(vec![]);
            }
            if matches!(base, Self::Infinite(_)) {
                base.size()?;
            }
            let base = base.sorted(limit)?;
            if let Some(k) = size
                && (k < 0 || k as usize > base.len())
            {
                return Ok(vec![]);
            }
            let indices = Value::set((0..base.len()).map(|i| Value::Int(i as i32)).collect())?;
            let subsets = SetExpr::subsets(&indices)?;
            if subsets.cardinality()? > limit {
                return Err(Error::Limit);
            }
            let mut result = Vec::new();
            for subset in subsets.iter()? {
                let indices = subset.elements()?;
                if size.is_none_or(|k| indices.len() == k as usize) {
                    result.push(Self::Set(
                        indices
                            .iter()
                            .map(|i| base[i.integer().unwrap() as usize].clone())
                            .collect(),
                    ));
                }
            }
            result
        })
    }

    /// `[D -> R]`: every function from the domain into the range.
    fn enumerate_functions(domain: &Self, range: &Self, limit: usize) -> Result<Vec<Self>, Error> {
        Ok({
            if !domain.empty()? && range.empty()? {
                return Ok(vec![]);
            }
            domain.require_enumerable(|value| Error::FunctionEnumeration {
                position: "domain D",
                value,
            })?;
            let domain = domain.sorted(limit)?;
            if domain.is_empty() {
                return Ok(vec![Self::Function {
                    kind: FunctionKind::Explicit,
                    entries: vec![],
                }]);
            }
            range.require_enumerable(|value| Error::FunctionEnumeration {
                position: "range R",
                value,
            })?;
            let values = range.sorted(limit)?;
            let domains = domain.iter().map(|_| values.clone()).collect::<Vec<_>>();
            combinations(&domains, limit)?
                .into_iter()
                .map(|values| {
                    Ok(Self::Function {
                        kind: FunctionKind::Explicit,
                        entries: domain
                            .iter()
                            .zip(values)
                            .map(|(k, v)| Ok((k.normalize(limit)?, v)))
                            .collect::<Result<_, Error>>()?,
                    })
                })
                .collect::<Result<_, Error>>()?
        })
    }

    /// `S1 \\X S2 \\X ...`.
    fn enumerate_product(&self, sets: &[Self], limit: usize) -> Result<Vec<Self>, Error> {
        Ok({
            if self.empty()? {
                return Ok(vec![]);
            }
            let mut domains = Vec::new();
            for (index, set) in sets.iter().enumerate() {
                set.require_enumerable(|value| Error::ProductEnumeration { index, value })?;
                let domain = set.sorted(limit)?;
                if domain.is_empty() {
                    return Ok(vec![]);
                }
                domains.push(domain);
            }
            combinations(&domains, limit)?
                .into_iter()
                .map(Self::tuple)
                .collect()
        })
    }

    /// `[f1 : S1, ...]`: every record over the field sets.
    fn enumerate_records(fields: &[(Atom, Self)], limit: usize) -> Result<Vec<Self>, Error> {
        Ok({
            let mut fields: Vec<_> = fields.iter().collect();
            fields.sort_by_key(|(name, _)| name.ordinal);
            if fields
                .iter()
                .enumerate()
                .any(|(i, (name, _))| fields[..i].iter().any(|(n, _)| n == name))
            {
                return Err(Error::DuplicateDomain);
            }
            for (_, set) in &fields {
                if set.empty()? {
                    return Ok(vec![]);
                }
            }
            let mut domains = Vec::new();
            for (name, set) in &fields {
                set.require_enumerable(|value| Error::RecordEnumeration {
                    field: name.text.to_string(),
                    value,
                })?;
                let domain = set.sorted(limit)?;
                if domain.is_empty() {
                    return Ok(vec![]);
                }
                domains.push(domain);
            }
            combinations(&domains, limit)?
                .into_iter()
                .map(|values| Self::Function {
                    kind: FunctionKind::Record,
                    entries: fields
                        .iter()
                        .map(|(n, _)| Value::String(n.clone()))
                        .zip(values)
                        .collect(),
                })
                .collect()
        })
    }

    pub fn enumerate(&self, limit: usize) -> Result<Vec<Self>, Error> {
        let values = match self {
            Self::Canonical(value) => value.elements()?.iter().cloned().map(Self::atom).collect(),
            Self::Set(values) => Self::sort_values(values.clone())?,
            Self::Interval(low, high) => {
                if self.size()? > limit {
                    return Err(Error::Limit);
                }
                (*low..=*high).map(|n| Self::atom(Value::Int(n))).collect()
            }
            Self::Binary(SetOp::Union, a, b) => {
                if [a.as_ref(), b.as_ref()].into_iter().any(|set| {
                    !set.is_set()
                        || matches!(
                            set,
                            Self::Infinite(_) | Self::Sequences(_) | Self::Undefined
                        )
                }) {
                    return Err(Error::UnionEnumeration {
                        left: crate::stringify(a)?,
                        right: crate::stringify(b)?,
                    });
                }
                let mut values = a.enumerate(limit)?;
                values.extend(b.enumerate(limit)?);
                values
            }
            Self::Binary(op, a, b) => {
                let (a, b) = if *op == SetOp::Intersection && !a.is_set() {
                    (b, a)
                } else {
                    (a, b)
                };
                let mut result = Vec::new();
                for value in a.enumerate(limit)? {
                    if b.contains(&value)? == (*op == SetOp::Intersection) {
                        result.push(value);
                    }
                }
                result
            }
            Self::Subsets { base, size } => Self::enumerate_subsets(base, *size, limit)?,
            Self::Functions { domain, range } => Self::enumerate_functions(domain, range, limit)?,
            Self::Product(sets) => self.enumerate_product(sets, limit)?,
            Self::Records(fields) => Self::enumerate_records(fields, limit)?,
            Self::Union(base) => {
                let mut values = Vec::new();
                for set in base.enumerate(limit)? {
                    values.extend(set.enumerate(limit)?);
                }
                values
            }
            Self::Filter {
                base,
                excluded,
                tuple,
            } => {
                let mut values = Vec::new();
                for value in base.enumerate(limit)? {
                    let candidate = if *tuple {
                        let tuple = value.sequence()?;
                        if tuple.len() != 2 {
                            return Err(Error::Type("pair"));
                        }
                        tuple[1].clone()
                    } else {
                        value.clone()
                    };
                    if !candidate.semantic_eq(excluded)? {
                        values.push(value);
                    }
                }
                values
            }
            _ => return Err(Error::Type("enumerable set")),
        };
        if values.len() > limit {
            Err(Error::Limit)
        } else {
            Ok(values)
        }
    }
}
fn combinations(domains: &[Vec<Expr>], limit: usize) -> Result<Vec<Vec<Expr>>, Error> {
    let mut result = vec![vec![]];
    for domain in domains {
        if result
            .len()
            .checked_mul(domain.len())
            .ok_or(Error::Overflow)?
            > limit
        {
            return Err(Error::Limit);
        }
        result = result
            .into_iter()
            .flat_map(|row| {
                domain.iter().map(move |value| {
                    let mut row = row.clone();
                    row.push(value.clone());
                    row
                })
            })
            .collect();
    }
    Ok(result)
}

impl Expr {
    fn require_enumerable(&self, error: impl FnOnce(String) -> Error) -> Result<(), Error> {
        if matches!(self, Self::Infinite(_) | Self::Sequences(_)) {
            return Err(error(crate::stringify(self)?));
        }
        Ok(())
    }
    pub fn empty(&self) -> Result<bool, Error> {
        match self {
            Self::Set(values) => Ok(values.is_empty()),
            Self::Canonical(Value::Set(values)) => Ok(values.is_empty()),
            Self::Interval(low, high) => Ok(high < low),
            Self::Subsets { base, size } => Ok(size.is_some_and(|k| k < 0)
                || size.is_some_and(|k| base.size().is_ok_and(|n| k as usize > n))),
            Self::Functions { domain, range } => Ok(!domain.empty()? && range.empty()?),
            Self::Product(sets) => {
                for set in sets {
                    if set.empty()? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Records(fields) => {
                let mut fields: Vec<_> = fields.iter().collect();
                fields.sort_by_key(|(name, _)| name.ordinal);
                for (_, set) in fields {
                    if set.empty()? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Infinite(_) | Self::Sequences(_) => Ok(false),
            _ => Ok(self.enumerate(100_000)?.is_empty()),
        }
    }
}
