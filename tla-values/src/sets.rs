use crate::{Error, Value};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum SetExpr {
    Finite(Arc<[Value]>),
    Interval(i32, i32),
    Subsets(Arc<[Value]>),
    Functions {
        domain: Arc<[Value]>,
        range: Arc<[Value]>,
    },
    Product(Arc<[Arc<[Value]>]>),
}
impl SetExpr {
    pub fn finite(value: &Value) -> Result<Self, Error> {
        Ok(Self::Finite(value.elements()?.into()))
    }
    pub fn subsets(value: &Value) -> Result<Self, Error> {
        Ok(Self::Subsets(value.elements()?.into()))
    }
    pub fn functions(domain: &Value, range: &Value) -> Result<Self, Error> {
        Ok(Self::Functions {
            domain: domain.elements()?.into(),
            range: range.elements()?.into(),
        })
    }
    pub fn product(sets: &[Value]) -> Result<Self, Error> {
        Ok(Self::Product(
            sets.iter()
                .map(|s| Ok(s.elements()?.into()))
                .collect::<Result<_, Error>>()?,
        ))
    }
    pub fn cardinality(&self) -> Result<usize, Error> {
        match self {
            Self::Finite(v) => Ok(v.len()),
            Self::Interval(low, high) => {
                usize::try_from((i64::from(*high) - i64::from(*low) + 1).max(0))
                    .map_err(|_| Error::Overflow)
            }
            Self::Subsets(v) => 1usize
                .checked_shl(u32::try_from(v.len()).map_err(|_| Error::Overflow)?)
                .ok_or(Error::Overflow),
            Self::Functions { domain, range } => range
                .len()
                .checked_pow(u32::try_from(domain.len()).map_err(|_| Error::Overflow)?)
                .ok_or(Error::Overflow),
            Self::Product(sets) => sets
                .iter()
                .try_fold(1usize, |n, s| n.checked_mul(s.len()).ok_or(Error::Overflow)),
        }
    }
    pub fn contains(&self, value: &Value) -> Result<bool, Error> {
        match self {
            Self::Finite(values) => Value::Set(values.clone()).contains(value),
            Self::Interval(low, high) => match value {
                Value::Int(n) => Ok(low <= n && n <= high),
                Value::Model(_) => value.semantic_eq(&Value::Int(*low)),
                _ => Err(Error::Type("integer")),
            },
            Self::Subsets(base) => {
                for member in value.elements()? {
                    if !Value::Set(base.clone()).contains(member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Functions { domain, range } => {
                let entries = value.entries()?;
                if entries.len() != domain.len() {
                    return Ok(false);
                }
                for ((key, member), expected) in entries.iter().zip(domain.iter()) {
                    if !key.semantic_eq(expected)? || !Value::Set(range.clone()).contains(member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Product(sets) => {
                let entries = value.entries()?;
                if entries.len() != sets.len() {
                    return Ok(false);
                }
                for (i, ((key, member), set)) in entries.iter().zip(sets.iter()).enumerate() {
                    if *key != Value::Int(i32::try_from(i + 1).map_err(|_| Error::Overflow)?)
                        || !Value::Set(set.clone()).contains(member)?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
    pub fn iter(&self) -> Result<SetIter, Error> {
        Ok(match self {
            Self::Finite(values) => SetIter::Finite {
                values: values.clone(),
                index: 0,
            },
            Self::Interval(low, high) => SetIter::Interval {
                next: i64::from(*low),
                high: i64::from(*high),
            },
            Self::Subsets(values) => SetIter::Subsets {
                values: values.clone(),
                indices: Vec::new(),
                size: 0,
                finished: false,
            },
            Self::Functions { domain, range } => SetIter::Functions {
                domain: domain.clone(),
                range: range.clone(),
                digits: vec![0; domain.len()],
                remaining: self.cardinality()?,
            },
            Self::Product(sets) => SetIter::Product {
                sets: sets.clone(),
                digits: vec![0; sets.len()],
                remaining: self.cardinality()?,
            },
        })
    }
    pub fn materialize(&self, limit: usize) -> Result<Value, Error> {
        if self.cardinality()? > limit {
            return Err(Error::Limit);
        }
        Value::set(self.iter()?.collect())
    }
    pub fn choose(
        &self,
        predicate: impl Fn(&Value) -> Result<bool, Error>,
    ) -> Result<Value, Error> {
        for value in self.iter()? {
            if predicate(&value)? {
                return Ok(value);
            }
        }
        Err(Error::Undefined)
    }
}

pub enum SetIter {
    Finite {
        values: Arc<[Value]>,
        index: usize,
    },
    Interval {
        next: i64,
        high: i64,
    },
    Subsets {
        values: Arc<[Value]>,
        indices: Vec<usize>,
        size: usize,
        finished: bool,
    },
    Functions {
        domain: Arc<[Value]>,
        range: Arc<[Value]>,
        digits: Vec<usize>,
        remaining: usize,
    },
    Product {
        sets: Arc<[Arc<[Value]>]>,
        digits: Vec<usize>,
        remaining: usize,
    },
}
impl Iterator for SetIter {
    type Item = Value;
    fn next(&mut self) -> Option<Value> {
        match self {
            Self::Finite { values, index } => {
                let value = values.get(*index)?.clone();
                *index += 1;
                Some(value)
            }
            Self::Interval { next, high } => {
                if *next > *high {
                    return None;
                }
                let value = Value::Int(*next as i32);
                *next += 1;
                Some(value)
            }
            Self::Subsets {
                values,
                indices,
                size,
                finished,
            } => {
                if *finished {
                    return None;
                }
                let result = Value::Set(indices.iter().map(|&i| values[i].clone()).collect());
                let position = (0..*size)
                    .rev()
                    .find(|&i| indices[i] < values.len() - *size + i);
                if let Some(i) = position {
                    indices[i] += 1;
                    for j in i + 1..*size {
                        indices[j] = indices[j - 1] + 1;
                    }
                } else if *size < values.len() {
                    *size += 1;
                    *indices = (0..*size).collect();
                } else {
                    *finished = true;
                }
                Some(result)
            }
            Self::Functions {
                domain,
                range,
                digits,
                remaining,
            } => {
                if *remaining == 0 {
                    return None;
                }
                let result = Value::Function(
                    domain
                        .iter()
                        .zip(digits.iter())
                        .map(|(k, &d)| (k.clone(), range[d].clone()))
                        .collect(),
                );
                *remaining -= 1;
                increment(digits, |_| range.len());
                Some(result)
            }
            Self::Product {
                sets,
                digits,
                remaining,
            } => {
                if *remaining == 0 {
                    return None;
                }
                let result = Value::tuple(
                    sets.iter()
                        .zip(digits.iter())
                        .map(|(s, &d)| s[d].clone())
                        .collect(),
                );
                *remaining -= 1;
                increment(digits, |i| sets[i].len());
                Some(result)
            }
        }
    }
}
fn increment(digits: &mut [usize], radix: impl Fn(usize) -> usize) {
    for i in (0..digits.len()).rev() {
        digits[i] += 1;
        if digits[i] < radix(i) {
            return;
        }
        digits[i] = 0;
    }
}
