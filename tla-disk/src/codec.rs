use crate::Codec;
use std::fmt;
use tla_values::Value;

/// Why bytes could not be encoded or decoded.
#[derive(Debug)]
pub enum CodecError {
    Truncated,
    TrailingBytes,
    InvalidLength,
    LengthOverflow,
    NestingLimit,
    UnknownTag(u8),
    InvalidHeader,
    ZeroDepth,
    Utf8(std::str::Utf8Error),
    Value(tla_values::Error),
    /// A reason from a codec outside this crate.
    Other(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated encoded state"),
            Self::TrailingBytes => f.write_str("trailing bytes in state"),
            Self::InvalidLength => f.write_str("invalid encoded collection length"),
            Self::LengthOverflow => f.write_str("encoded length exceeds u32"),
            Self::NestingLimit => f.write_str("value nesting exceeds codec limit"),
            Self::UnknownTag(tag) => write!(f, "unknown encoded value tag {tag}"),
            Self::InvalidHeader => f.write_str("invalid state header"),
            Self::ZeroDepth => f.write_str("stored state depth must be positive"),
            Self::Utf8(e) => e.fmt(f),
            Self::Value(e) => e.fmt(f),
            Self::Other(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<tla_values::Error> for CodecError {
    fn from(e: tla_values::Error) -> Self {
        Self::Value(e)
    }
}

pub struct ValueCodec;
impl Codec<Vec<Value>, Vec<Value>> for ValueCodec {
    fn state(&self, state: &Vec<Value>) -> Result<Vec<u8>, CodecError> {
        encode(state)
    }
    fn key(&self, key: &Vec<Value>) -> Result<Vec<u8>, CodecError> {
        encode(key)
    }
    fn decode(&self, bytes: &[u8]) -> Result<Vec<Value>, CodecError> {
        let mut input = Input(bytes);
        let count = input.count()?;
        let values = (0..count)
            .map(|_| input.value(0))
            .collect::<Result<_, _>>()?;
        if !input.0.is_empty() {
            return Err(CodecError::TrailingBytes);
        }
        Ok(values)
    }
}

fn count(out: &mut Vec<u8>, n: usize) -> Result<(), CodecError> {
    out.extend(
        u32::try_from(n)
            .map_err(|_| CodecError::LengthOverflow)?
            .to_le_bytes(),
    );
    Ok(())
}
fn encode(values: &[Value]) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    count(&mut out, values.len())?;
    for value in values {
        write(value, &mut out, 0)?;
    }
    Ok(out)
}
fn write(value: &Value, out: &mut Vec<u8>, depth: usize) -> Result<(), CodecError> {
    if depth > 128 {
        return Err(CodecError::NestingLimit);
    }
    match value {
        Value::Bool(value) => out.push(u8::from(*value)),
        Value::Int(value) => {
            out.push(2);
            out.extend(value.to_le_bytes());
        }
        Value::String(atom) | Value::Model(atom) => {
            out.push(if matches!(value, Value::String(_)) {
                3
            } else {
                4
            });
            out.extend(atom.ordinal.to_le_bytes());
            count(out, atom.text.len())?;
            out.extend(atom.text.as_bytes());
        }
        Value::Set(values) => {
            out.push(5);
            count(out, values.len())?;
            for value in values.iter() {
                write(value, out, depth + 1)?;
            }
        }
        Value::Function(entries) => {
            out.push(6);
            count(out, entries.len())?;
            for (key, value) in entries.iter() {
                write(key, out, depth + 1)?;
                write(value, out, depth + 1)?;
            }
        }
    }
    Ok(())
}
struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        if n > self.0.len() {
            return Err(CodecError::Truncated);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn word(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, CodecError> {
        let count = self.word()? as usize;
        if count > self.0.len() {
            return Err(CodecError::InvalidLength);
        }
        Ok(count)
    }
    fn value(&mut self, depth: usize) -> Result<Value, CodecError> {
        if depth > 128 {
            return Err(CodecError::NestingLimit);
        }
        Ok(match self.take(1)?[0] {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            2 => Value::Int(self.word()? as i32),
            tag @ (3 | 4) => {
                let ordinal = self.word()?;
                let n = self.count()?;
                let text = std::str::from_utf8(self.take(n)?).map_err(CodecError::Utf8)?;
                if tag == 3 {
                    Value::string(text, ordinal)
                } else {
                    Value::model(text, ordinal)
                }
            }
            5 => {
                let n = self.count()?;
                Value::set(
                    (0..n)
                        .map(|_| self.value(depth + 1))
                        .collect::<Result<_, _>>()?,
                )?
            }
            6 => {
                let n = self.count()?;
                Value::function(
                    (0..n)
                        .map(|_| Ok((self.value(depth + 1)?, self.value(depth + 1)?)))
                        .collect::<Result<_, CodecError>>()?,
                )?
            }
            tag => return Err(CodecError::UnknownTag(tag)),
        })
    }
}

pub struct StateCodec;
impl Codec<tla_engine::Leveled<Vec<Value>>, Vec<Value>> for StateCodec {
    fn state(&self, state: &tla_engine::Leveled<Vec<Value>>) -> Result<Vec<u8>, CodecError> {
        if state.depth == 0 {
            return Err(CodecError::ZeroDepth);
        }
        let mut bytes = b"TLS1".to_vec();
        bytes.extend(
            u64::try_from(state.depth)
                .map_err(|_| CodecError::LengthOverflow)?
                .to_le_bytes(),
        );
        bytes.extend(ValueCodec.state(&state.values)?);
        Ok(bytes)
    }
    fn key(&self, key: &Vec<Value>) -> Result<Vec<u8>, CodecError> {
        ValueCodec.key(key)
    }
    fn decode(&self, bytes: &[u8]) -> Result<tla_engine::Leveled<Vec<Value>>, CodecError> {
        if bytes.len() < 12 || &bytes[..4] != b"TLS1" {
            return Err(CodecError::InvalidHeader);
        }
        let depth = usize::try_from(u64::from_le_bytes(bytes[4..12].try_into().unwrap()))
            .map_err(|_| CodecError::LengthOverflow)?;
        if depth == 0 {
            return Err(CodecError::ZeroDepth);
        }
        Ok(tla_engine::Leveled {
            values: ValueCodec.decode(&bytes[12..])?,
            depth,
        })
    }
}
