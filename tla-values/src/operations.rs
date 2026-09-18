use crate::{Error, Value};

impl Value {
    pub fn union(&self, other: &Self) -> Result<Self, Error> {
        Self::set(
            self.elements()?
                .iter()
                .chain(other.elements()?)
                .cloned()
                .collect(),
        )
    }
    pub fn intersection(&self, other: &Self) -> Result<Self, Error> {
        self.filter(|v| other.contains(v))
    }
    pub fn difference(&self, other: &Self) -> Result<Self, Error> {
        self.filter(|v| Ok(!other.contains(v)?))
    }
    pub fn filter(&self, predicate: impl Fn(&Value) -> Result<bool, Error>) -> Result<Self, Error> {
        let mut values = Vec::new();
        for value in self.elements()? {
            if predicate(value)? {
                values.push(value.clone());
            }
        }
        Ok(Self::Set(values.into()))
    }
    pub fn is_subset(&self, other: &Self) -> Result<bool, Error> {
        for value in self.elements()? {
            if !other.contains(value)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn union_all(&self) -> Result<Self, Error> {
        Self::set(
            self.elements()?
                .iter()
                .map(Self::elements)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .cloned()
                .collect(),
        )
    }
    pub fn except(&self, path: &[Value], replacement: Self) -> Result<Self, Error> {
        let Some((head, tail)) = path.split_first() else {
            return Ok(replacement);
        };
        let mut entries = self.entries()?.to_vec();
        for (key, value) in &mut entries {
            if key.semantic_eq(head)? {
                *value = value.except(tail, replacement)?;
                return Ok(Self::Function(entries.into()));
            }
        }
        Ok(self.clone())
    }
    pub fn sequence(&self) -> Result<Vec<Value>, Error> {
        self.entries()?
            .iter()
            .enumerate()
            .map(|(i, (key, value))| {
                if *key == Self::Int(i32::try_from(i + 1).map_err(|_| Error::Overflow)?) {
                    Ok(value.clone())
                } else {
                    Err(Error::Type("sequence"))
                }
            })
            .collect()
    }
}

impl Value {
    pub fn fingerprint(&self, polynomial: u64) -> i32 {
        self.extend_fingerprint(polynomial, polynomial) as i32
    }
    pub fn extend_fingerprint(&self, initial: u64, polynomial: u64) -> u64 {
        fn bytes(mut fp: u64, input: impl IntoIterator<Item = u8>, polynomial: u64) -> u64 {
            for byte in input {
                fp ^= u64::from(byte);
                for _ in 0..8 {
                    fp = (fp >> 1) ^ if fp & 1 == 0 { 0 } else { polynomial };
                }
            }
            fp
        }
        fn extend(value: &Value, mut fp: u64, polynomial: u64) -> u64 {
            let tag = match value {
                Value::Bool(_) => 0,
                Value::Int(_) => 1,
                Value::String(_) => 3,
                Value::Set(_) => 5,
                Value::Function(_) => 9,
                Value::Model(_) => 21,
            };
            fp = bytes(fp, [tag], polynomial);
            match value {
                Value::Bool(v) => bytes(fp, [if *v { b't' } else { b'f' }], polynomial),
                Value::Int(v) => bytes(fp, v.to_le_bytes(), polynomial),
                Value::String(atom) => {
                    let units = atom.text.encode_utf16();
                    fp = bytes(fp, (units.clone().count() as i32).to_le_bytes(), polynomial);
                    bytes(fp, units.map(|unit| unit as u8), polynomial)
                }
                Value::Model(atom) => bytes(fp, atom.ordinal.to_le_bytes(), polynomial),
                Value::Set(values) => {
                    fp = bytes(fp, (values.len() as i32).to_le_bytes(), polynomial);
                    values.iter().fold(fp, |fp, v| extend(v, fp, polynomial))
                }
                Value::Function(entries) => {
                    fp = bytes(fp, (entries.len() as i32).to_le_bytes(), polynomial);
                    entries.iter().fold(fp, |fp, (k, v)| {
                        extend(v, extend(k, fp, polynomial), polynomial)
                    })
                }
            }
        }
        extend(self, initial, polynomial)
    }
}
