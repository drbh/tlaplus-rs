//! Finite TLA+ values: booleans, integers, interned strings and model values,
//! sets and functions, with TLC's ordering and equality. [`Value`] is shared
//! by every layer above; [`SetExpr`] describes sets too large to enumerate.
mod operations;
mod sets;
pub use sets::{SetExpr, SetIter};
use std::{cmp::Ordering, fmt, sync::Arc};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Atom {
    pub text: Arc<str>,
    pub ordinal: u32,
}

impl Atom {
    pub fn new(text: impl Into<Arc<str>>, ordinal: u32) -> Self {
        Self {
            text: text.into(),
            ordinal,
        }
    }
    fn model_type(&self) -> Option<u16> {
        let mut units = self.text.encode_utf16();
        let first = units.next()?;
        (units.next()? == u16::from(b'_') && units.next().is_some()).then_some(first)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    String(Atom),
    Model(Atom),
    Set(Arc<[Value]>),
    Function(Arc<[(Value, Value)]>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Type(&'static str),
    Incomparable,
    BooleanContext(String),
    UnionEnumeration {
        left: String,
        right: String,
    },
    IntegerEquality {
        integer: i32,
        value: String,
    },
    DuplicateDomain,
    Undefined,
    Overflow,
    Limit,
    InfiniteCardinality(&'static str),
    FunctionEnumeration {
        position: &'static str,
        value: String,
    },
    RecordEnumeration {
        field: String,
        value: String,
    },
    ProductEnumeration {
        index: usize,
        value: String,
    },
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BooleanContext(message) => f.write_str(message),
            Self::UnionEnumeration { left, right } => write!(
                f,
                "Attempted to enumerate S \\cup T when S:\n{left}\nand T:\n{right}\nare not both enumerable"
            ),
            Self::IntegerEquality { integer, value } => write!(
                f,
                "Attempted to check equality of integer {integer} with non-integer:\n{value}"
            ),
            Self::FunctionEnumeration { position, value } => write!(
                f,
                "Attempted to enumerate a set of the form [D -> R],but the {position}:\n{value}\ncannot be enumerated."
            ),
            Self::RecordEnumeration { field, value } => write!(
                f,
                "Attempted to enumerate a set of the form [l1 : v1, ..., ln : vn],\nbut can't enumerate the value of the `{field}' field:\n{value}"
            ),
            Self::ProductEnumeration { index, value } => write!(
                f,
                "Attempted to enumerate a set of the form s1 \\X s2 ... \\X sn,\nbut can't enumerate s{index}:\n{value}"
            ),
            _ => write!(f, "{self:?}"),
        }
    }
}
impl std::error::Error for Error {}

impl Value {
    pub fn string(text: impl Into<Arc<str>>, ordinal: u32) -> Self {
        Self::String(Atom::new(text, ordinal))
    }
    pub fn model(text: impl Into<Arc<str>>, ordinal: u32) -> Self {
        Self::Model(Atom::new(text, ordinal))
    }
    pub fn boolean(&self) -> Result<bool, Error> {
        match self {
            Self::Bool(v) => Ok(*v),
            _ => Err(Error::Type("boolean")),
        }
    }
    pub fn integer(&self) -> Result<i32, Error> {
        match self {
            Self::Int(v) => Ok(*v),
            _ => Err(Error::Type("integer")),
        }
    }
    pub fn elements(&self) -> Result<&[Value], Error> {
        match self {
            Self::Set(v) => Ok(v),
            _ => Err(Error::Type("finite set")),
        }
    }
    pub fn entries(&self) -> Result<&[(Value, Value)], Error> {
        match self {
            Self::Function(v) => Ok(v),
            _ => Err(Error::Type("function")),
        }
    }
    pub fn set(mut values: Vec<Value>) -> Result<Self, Error> {
        sort(&mut values, |v| v)?;
        values.dedup();
        Ok(Self::Set(values.into()))
    }
    pub fn function(mut entries: Vec<(Value, Value)>) -> Result<Self, Error> {
        sort(&mut entries, |v| &v.0)?;
        if entries.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(Error::DuplicateDomain);
        }
        Ok(Self::Function(entries.into()))
    }
    pub fn tuple(values: Vec<Value>) -> Self {
        Self::Function(
            values
                .into_iter()
                .enumerate()
                .map(|(i, v)| (Self::position(i), v))
                .collect(),
        )
    }
    pub fn record(entries: Vec<(Atom, Value)>) -> Result<Self, Error> {
        Self::function(
            entries
                .into_iter()
                .map(|(a, v)| (Self::String(a), v))
                .collect(),
        )
    }
    pub fn cardinality(&self) -> Result<usize, Error> {
        Ok(self.elements()?.len())
    }
    pub fn contains(&self, value: &Self) -> Result<bool, Error> {
        for member in self.elements()? {
            if member.semantic_eq(value)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn apply(&self, argument: &Self) -> Result<Self, Error> {
        for (key, value) in self.entries()? {
            if key.semantic_eq(argument)? {
                return Ok(value.clone());
            }
        }
        Err(Error::Undefined)
    }
    pub fn domain(&self) -> Result<Self, Error> {
        Self::set(self.entries()?.iter().map(|(k, _)| k.clone()).collect())
    }
    pub fn choose(&self, predicate: impl Fn(&Self) -> Result<bool, Error>) -> Result<Self, Error> {
        for value in self.elements()? {
            if predicate(value)? {
                return Ok(value.clone());
            }
        }
        Err(Error::Undefined)
    }
    pub fn semantic_eq(&self, other: &Self) -> Result<bool, Error> {
        match (self, other) {
            (Self::Model(a), Self::Model(b)) => {
                if a.model_type().is_some()
                    && b.model_type().is_some()
                    && a.model_type() != b.model_type()
                {
                    return Err(Error::Incomparable);
                }
                Ok(a == b)
            }
            (Self::Model(a), _) | (_, Self::Model(a)) => {
                if a.model_type().is_some() {
                    Err(Error::Incomparable)
                } else {
                    Ok(false)
                }
            }
            (Self::Bool(a), Self::Bool(b)) => Ok(a == b),
            (Self::Int(a), Self::Int(b)) => Ok(a == b),
            (Self::String(a), Self::String(b)) => Ok(a == b),
            (Self::Set(a), Self::Set(b)) => equal_slices(a, b),
            (Self::Function(a), Self::Function(b)) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                for ((ak, av), (bk, bv)) in a.iter().zip(b.iter()) {
                    if !ak.semantic_eq(bk)? || !av.semantic_eq(bv)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Err(Error::Incomparable),
        }
    }
    pub fn compare(&self, other: &Self) -> Result<Ordering, Error> {
        match (self, other) {
            (Self::Model(a), Self::Model(b)) => {
                self.semantic_eq(other)?;
                Ok(a.ordinal.cmp(&b.ordinal))
            }
            (Self::Model(a), _) | (_, Self::Model(a)) => {
                if a.model_type().is_some() {
                    return Err(Error::Incomparable);
                }
                Ok(if matches!(self, Self::Model(_)) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                })
            }
            (Self::Bool(a), Self::Bool(b)) => Ok(a.cmp(b)),
            (Self::Int(a), Self::Int(b)) => Ok(a.cmp(b)),
            (Self::String(a), Self::String(b)) => Ok(a.ordinal.cmp(&b.ordinal)),
            (Self::Set(a), Self::Set(b)) => compare_slices(a, b),
            (Self::Function(a), Self::Function(b)) => {
                let size = a.len().cmp(&b.len());
                if size != Ordering::Equal {
                    return Ok(size);
                }
                for ((ak, _), (bk, _)) in a.iter().zip(b.iter()) {
                    let order = ak.compare(bk)?;
                    if order != Ordering::Equal {
                        return Ok(order);
                    }
                }
                for ((_, av), (_, bv)) in a.iter().zip(b.iter()) {
                    let order = av.compare(bv)?;
                    if order != Ordering::Equal {
                        return Ok(order);
                    }
                }
                Ok(Ordering::Equal)
            }
            _ => Err(Error::Incomparable),
        }
    }
}
fn equal_slices(a: &[Value], b: &[Value]) -> Result<bool, Error> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.iter().zip(b) {
        if !a.semantic_eq(b)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn compare_slices(a: &[Value], b: &[Value]) -> Result<Ordering, Error> {
    let size = a.len().cmp(&b.len());
    if size != Ordering::Equal {
        return Ok(size);
    }
    for (a, b) in a.iter().zip(b) {
        let order = a.compare(b)?;
        if order != Ordering::Equal {
            return Ok(order);
        }
    }
    Ok(Ordering::Equal)
}
fn sort<T>(values: &mut [T], key: impl Fn(&T) -> &Value) -> Result<(), Error> {
    let mut error = None;
    values.sort_by(|a, b| {
        key(a).compare(key(b)).unwrap_or_else(|e| {
            error = Some(e);
            Ordering::Equal
        })
    });
    error.map_or(Ok(()), Err)
}

impl Value {
    /// The 1-based position of `index` in a tuple, as TLA+ numbers its elements.
    pub fn position(index: usize) -> Self {
        Self::Int(i32::try_from(index + 1).expect("tuple position fits an i32"))
    }
}
