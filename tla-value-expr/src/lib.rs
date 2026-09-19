//! Deferred value expressions: sets and functions kept symbolic until a
//! consumer needs their elements, plus TLC-compatible rendering of values.
//! [`Expr`] wraps a [`Value`] or a lazy set or function over values.
mod functions;
pub use functions::{quoted, stringify};
mod sets;
use std::cmp::Ordering;
pub use tla_values::{Atom, Error, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FunctionKind {
    Explicit,
    Interval(i32),
    Tuple,
    Record,
    Lambda,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetOp {
    Union,
    Intersection,
    Difference,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfiniteSet {
    Any,
    Integers,
    Naturals,
    Strings,
}
#[derive(Clone, Debug)]
pub enum Expr {
    Canonical(Value),
    Set(Vec<Expr>),
    Interval(i32, i32),
    Function {
        kind: FunctionKind,
        entries: Vec<(Value, Expr)>,
    },
    Binary(SetOp, Box<Expr>, Box<Expr>),
    Subsets {
        base: Box<Expr>,
        size: Option<i32>,
    },
    Functions {
        domain: Box<Expr>,
        range: Box<Expr>,
    },
    Product(Vec<Expr>),
    Records(Vec<(Atom, Expr)>),
    Union(Box<Expr>),
    Filter {
        base: Box<Expr>,
        excluded: Box<Expr>,
        tuple: bool,
    },
    Infinite(InfiniteSet),
    Sequences(Box<Expr>),
    Lambda(Box<Expr>),
    LambdaMulti {
        domains: Vec<Expr>,
        tuple_first: bool,
    },
    Undefined,
}
impl Expr {
    /// A finite value as an expression. The value stays canonical and
    /// shared; it is expanded into a set or function form only where an
    /// operation needs that shape, so passing values through the deferred
    /// layer costs nothing.
    pub fn atom(value: Value) -> Self {
        Self::Canonical(value)
    }
    /// A value with its outermost set or function expanded into the
    /// corresponding expression form, for operations and messages that
    /// work on that shape.
    pub fn expanded(value: Value) -> Self {
        match value {
            Value::Set(values) => Self::Set(values.iter().cloned().map(Self::atom).collect()),
            Value::Function(entries) => Self::Function {
                kind: FunctionKind::Explicit,
                entries: entries
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::atom(v.clone())))
                    .collect(),
            },
            value => Self::Canonical(value),
        }
    }
    /// What a model value is compared with when this is the other operand.
    /// A model value equals nothing but itself, and the value layer decides
    /// whether comparing it with a non-model value is `false` or an error
    /// for typed model values; so a canonical operand is used as is, and any
    /// deferred form is represented by a placeholder non-model value.
    pub(crate) fn against_model(&self) -> Value {
        match self {
            Self::Canonical(value) => value.clone(),
            _ => Value::Int(0),
        }
    }
    /// Prepares a deferred left operand for comparison with a model value
    /// on the right. An infinite set answers for itself and returns `true`:
    /// it is unequal to, and ordered after, any model value, typed or not,
    /// which mirrors TLC where the set's own comparison runs first. Any
    /// other operand returns `false` and is compared by the model value, a
    /// finite set being enumerated first so that enumeration errors
    /// surface, as in TLC.
    fn answers_against_model(&self) -> Result<bool, Error> {
        if matches!(self, Self::Infinite(_) | Self::Sequences(_)) {
            return Ok(true);
        }
        if self.is_set() {
            self.sorted(100_000)?;
        }
        Ok(false)
    }
    /// Whether a model value is a member of a set defined by the shape of
    /// its members: never, though a typed model value makes the test an
    /// error.
    pub(crate) fn model_membership(model: &Value) -> Result<bool, Error> {
        model.semantic_eq(&Value::Int(0))
    }
    /// Whether membership of this set is decided by the shape of the value:
    /// an integer in a range, a function, tuple or record over given sets,
    /// or a member of a built-in infinite set.
    fn members_have_shape(&self) -> bool {
        matches!(
            self,
            Self::Interval(..)
                | Self::Functions { .. }
                | Self::Product(_)
                | Self::Records(_)
                | Self::Infinite(_)
                | Self::Sequences(_)
        )
    }
    /// Whether this is a function in any form, canonical included.
    pub(crate) fn is_function(&self) -> bool {
        matches!(
            self,
            Self::Function { .. }
                | Self::Lambda(_)
                | Self::LambdaMulti { .. }
                | Self::Canonical(Value::Function(_))
        )
    }
    pub fn tuple(values: Vec<Self>) -> Self {
        Self::Function {
            kind: FunctionKind::Tuple,
            entries: values
                .into_iter()
                .enumerate()
                .map(|(i, v)| (Value::position(i), v))
                .collect(),
        }
    }
    pub fn normalize(&self, limit: usize) -> Result<Value, Error> {
        match self {
            Self::Canonical(v) => Ok(v.clone()),
            Self::Function { .. } | Self::Lambda(_) | Self::LambdaMulti { .. } => Value::function(
                self.entries()?
                    .into_iter()
                    .map(|(k, v)| Ok((k, v.normalize(limit)?)))
                    .collect::<Result<_, Error>>()?,
            ),
            _ => Value::set(
                self.enumerate(limit)?
                    .iter()
                    .map(|v| v.normalize(limit))
                    .collect::<Result<_, _>>()?,
            ),
        }
    }
    pub fn semantic_eq(&self, other: &Self) -> Result<bool, Error> {
        if matches!(self, Self::Binary(SetOp::Union, ..)) {
            return Self::Set(self.sorted(100_000)?).semantic_eq(other);
        }
        if let (
            Self::Functions {
                domain: a,
                range: b,
            },
            Self::Functions {
                domain: c,
                range: d,
            },
        ) = (self, other)
        {
            let (a_empty, c_empty) = (a.empty()?, c.empty()?);
            if a_empty || c_empty {
                return Ok(a_empty == c_empty);
            }
            let (b_empty, d_empty) = (b.empty()?, d.empty()?);
            if b_empty || d_empty {
                return Ok(b_empty == d_empty);
            }
            return Ok(a.semantic_eq(c)? && b.semantic_eq(d)?);
        }
        if let Self::Canonical(model @ Value::Model(_)) = self {
            return model.semantic_eq(&other.against_model());
        }
        if let Self::Canonical(Value::Model(_)) = other {
            if self.answers_against_model()? {
                return Ok(false);
            }
            return other.semantic_eq(self);
        }
        if let (Self::Canonical(a), Self::Canonical(b)) = (self, other)
            && (!matches!(a, Value::Int(_)) || matches!(b, Value::Int(_)))
        {
            // Incomparable canonical values are compared again in expanded
            // form, which names the offending parts the way TLC does. An
            // integer against a non-integer is reported below.
            return match a.semantic_eq(b) {
                Err(Error::Incomparable) if a.is_compound() || b.is_compound() => {
                    Self::expanded(a.clone()).semantic_eq(&Self::expanded(b.clone()))
                }
                result => result,
            };
        }
        if self.is_function() {
            self.check_record(other)?;
            let (a, b) = (self.entries()?, other.entries()?);
            if a.len() != b.len() {
                return Ok(false);
            }
            for ((ak, _), (bk, _)) in a.iter().zip(&b) {
                if !ak.semantic_eq(bk)? {
                    return Ok(false);
                }
            }
            for ((_, av), (_, bv)) in a.iter().zip(&b) {
                if !av.semantic_eq(bv)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if matches!((self, other), (Self::Infinite(_), Self::Infinite(_))) {
            return Ok(self.compare(other)? == Ordering::Equal);
        }
        if let (Self::Sequences(a), Self::Sequences(b)) = (self, other) {
            return Ok(a.compare(b)? == Ordering::Equal);
        }
        match (self, other) {
            (Self::Canonical(Value::Int(integer)), other)
                if !matches!(other, Self::Canonical(Value::Int(_))) =>
            {
                Err(Error::IntegerEquality {
                    integer: *integer,
                    value: stringify(other)?,
                })
            }
            _ if self.is_set() && other.is_set() => {
                let (a, b) = (self.sorted(100_000)?, other.sorted(100_000)?);
                if a.len() != b.len() {
                    return Ok(false);
                }
                for (a, b) in a.iter().zip(&b) {
                    if !a.semantic_eq(b)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Err(Error::Incomparable),
        }
    }
    pub fn compare(&self, other: &Self) -> Result<Ordering, Error> {
        if let Self::Canonical(model @ Value::Model(_)) = self {
            return model.compare(&other.against_model());
        }
        if let Self::Canonical(Value::Model(_)) = other {
            if self.answers_against_model()? {
                return Ok(Ordering::Greater);
            }
            return Ok(other.compare(self)?.reverse());
        }
        if let (Self::Canonical(a), Self::Canonical(b)) = (self, other) {
            return match a.compare(b) {
                Err(Error::Incomparable) if a.is_compound() || b.is_compound() => {
                    Self::expanded(a.clone()).compare(&Self::expanded(b.clone()))
                }
                result => result,
            };
        }
        if self.is_function() {
            self.check_record(other)?;
            let (a, b) = (self.entries()?, other.entries()?);
            let size = a.len().cmp(&b.len());
            if size != Ordering::Equal {
                return Ok(size);
            }
            for ((ak, _), (bk, _)) in a.iter().zip(&b) {
                if (matches!(
                    self,
                    Self::Function {
                        kind: FunctionKind::Interval(_) | FunctionKind::Tuple,
                        ..
                    }
                ) && !matches!(bk, Value::Int(_)))
                    || (matches!(
                        other,
                        Self::Function {
                            kind: FunctionKind::Interval(_) | FunctionKind::Tuple,
                            ..
                        }
                    ) && !matches!(ak, Value::Int(_)))
                {
                    return Err(Error::Incomparable);
                }
                let c = ak.compare(bk)?;
                if c != Ordering::Equal {
                    return Ok(c);
                }
            }
            for ((_, av), (_, bv)) in a.iter().zip(&b) {
                let c = av.compare(bv)?;
                if c != Ordering::Equal {
                    return Ok(c);
                }
            }
            return Ok(Ordering::Equal);
        }
        if let (Self::Sequences(a), Self::Sequences(b)) = (self, other) {
            return a.compare(b);
        }
        if let (Self::Infinite(a), Self::Infinite(b)) = (self, other) {
            return match (a, b) {
                (InfiniteSet::Any, _) | (_, InfiniteSet::Any) => Err(Error::Incomparable),
                _ if a == b => Ok(Ordering::Equal),
                (InfiniteSet::Naturals, InfiniteSet::Integers) => Ok(Ordering::Less),
                (InfiniteSet::Integers, InfiniteSet::Naturals) => Ok(Ordering::Greater),
                _ => Err(Error::Incomparable),
            };
        }
        match (self, other) {
            _ if self.is_set() && other.is_set() => {
                let (a, b) = (self.sorted(100_000)?, other.sorted(100_000)?);
                let size = a.len().cmp(&b.len());
                if size != Ordering::Equal {
                    return Ok(size);
                }
                for (a, b) in a.iter().zip(&b) {
                    let c = a.compare(b)?;
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                Ok(Ordering::Equal)
            }
            _ => Err(Error::Incomparable),
        }
    }
    pub fn sorted(&self, limit: usize) -> Result<Vec<Self>, Error> {
        Self::sort_values(self.enumerate(limit)?)
    }
    pub(crate) fn sort_values(mut values: Vec<Self>) -> Result<Vec<Self>, Error> {
        let mut error = None;
        values.sort_by(|a, b| {
            a.compare(b).unwrap_or_else(|e| {
                error = Some(e);
                Ordering::Equal
            })
        });
        if let Some(error) = error {
            return Err(error);
        }
        let mut result = Vec::<Self>::new();
        for value in values {
            if result.last().map(|last| last.compare(&value)).transpose()? != Some(Ordering::Equal)
            {
                result.push(value);
            }
        }
        Ok(result)
    }
    pub(crate) fn is_set(&self) -> bool {
        !matches!(
            self,
            Self::Function { .. }
                | Self::Lambda(_)
                | Self::LambdaMulti { .. }
                | Self::Canonical(
                    Value::Bool(_)
                        | Value::Int(_)
                        | Value::String(_)
                        | Value::Model(_)
                        | Value::Function(_)
                )
        )
    }
    pub fn cardinality(&self) -> Result<usize, Error> {
        if !self.is_set() || matches!(self, Self::Sequences(_) | Self::Undefined) {
            return Err(Error::Type("enumerable set"));
        }
        self.size()
    }
    pub fn size(&self) -> Result<usize, Error> {
        match self {
            Self::Infinite(InfiniteSet::Naturals) => Err(Error::InfiniteCardinality("Nat")),
            Self::Infinite(InfiniteSet::Integers) => Err(Error::InfiniteCardinality("Int")),
            Self::Infinite(InfiniteSet::Strings) => Err(Error::InfiniteCardinality("STRING")),
            Self::Function {
                kind: FunctionKind::Explicit,
                ..
            } => Ok(self.entries()?.len()),
            Self::Function { entries, .. } => Ok(entries.len()),
            Self::Lambda(domain) => domain.size(),
            Self::LambdaMulti { domains, .. } => domains.iter().try_fold(1usize, |n, d| {
                n.checked_mul(d.size()?).ok_or(Error::Overflow)
            }),
            Self::Canonical(Value::Function(v)) => Ok(v.len()),
            Self::Canonical(Value::Set(v)) => Ok(v.len()),
            Self::Interval(low, high) => {
                Ok((i64::from(*high) - i64::from(*low) + 1).max(0) as usize)
            }
            _ => Ok(self.sorted(100_000)?.len()),
        }
    }
    pub fn finite(&self) -> Result<bool, Error> {
        match self {
            Self::Canonical(Value::Set(_) | Value::Function(_))
            | Self::Set(_)
            | Self::Interval(..)
            | Self::Function {
                kind:
                    FunctionKind::Explicit
                    | FunctionKind::Interval(_)
                    | FunctionKind::Tuple
                    | FunctionKind::Record,
                ..
            } => Ok(true),
            Self::Infinite(_) => Ok(false),
            Self::Sequences(base) => Ok(base.finite()? && base.empty()?),
            Self::Binary(SetOp::Union, a, b) => Ok(a.finite()? && b.finite()?),
            Self::Binary(SetOp::Intersection, a, b) => {
                if a.finite()? || b.finite()? {
                    Ok(true)
                } else {
                    Err(Error::Type("finite set"))
                }
            }
            Self::Binary(SetOp::Difference, a, b) => {
                if a.finite()? {
                    Ok(true)
                } else if b.finite()? {
                    Ok(false)
                } else {
                    Err(Error::Type("finite set"))
                }
            }
            Self::Subsets { base, size } => Ok(size.is_some_and(|k| k <= 0) || base.finite()?),
            Self::Filter { base, .. } => base.finite(),
            Self::Union(base) => {
                for set in base.enumerate(100_000)? {
                    if !set.finite()? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Functions { domain, range } => Ok(domain.finite()? && range.finite()?),
            Self::Product(sets) => sets.iter().try_fold(true, |acc, s| Ok(acc && s.finite()?)),
            Self::Records(fields) => fields
                .iter()
                .try_fold(true, |acc, (_, s)| Ok(acc && s.finite()?)),
            _ => Err(Error::Type("finite set")),
        }
    }
}
