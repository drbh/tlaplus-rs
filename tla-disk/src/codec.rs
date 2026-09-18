use crate::Codec;
use tla_values::Value;

pub struct ValueCodec;
impl Codec<Vec<Value>, Vec<Value>> for ValueCodec {
    fn state(&self, state: &Vec<Value>) -> Result<Vec<u8>, String> {
        encode(state)
    }
    fn key(&self, key: &Vec<Value>) -> Result<Vec<u8>, String> {
        encode(key)
    }
    fn decode(&self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        let mut input = Input(bytes);
        let count = input.count()?;
        let values = (0..count)
            .map(|_| input.value(0))
            .collect::<Result<_, _>>()?;
        if !input.0.is_empty() {
            return Err("trailing bytes in state".into());
        }
        Ok(values)
    }
}

fn count(out: &mut Vec<u8>, n: usize) -> Result<(), String> {
    out.extend(
        u32::try_from(n)
            .map_err(|_| "encoded length exceeds u32")?
            .to_le_bytes(),
    );
    Ok(())
}
fn encode(values: &[Value]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    count(&mut out, values.len())?;
    for value in values {
        write(value, &mut out, 0)?;
    }
    Ok(out)
}
fn write(value: &Value, out: &mut Vec<u8>, depth: usize) -> Result<(), String> {
    if depth > 128 {
        return Err("value nesting exceeds codec limit".into());
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
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("truncated encoded state".into());
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn word(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        let count = self.word()? as usize;
        if count > self.0.len() {
            return Err("invalid encoded collection length".into());
        }
        Ok(count)
    }
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return Err("value nesting exceeds codec limit".into());
        }
        Ok(match self.take(1)?[0] {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            2 => Value::Int(self.word()? as i32),
            tag @ (3 | 4) => {
                let ordinal = self.word()?;
                let n = self.count()?;
                let text = std::str::from_utf8(self.take(n)?).map_err(|e| e.to_string())?;
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
                )
                .map_err(|e| e.to_string())?
            }
            6 => {
                let n = self.count()?;
                Value::function(
                    (0..n)
                        .map(|_| Ok((self.value(depth + 1)?, self.value(depth + 1)?)))
                        .collect::<Result<_, String>>()?,
                )
                .map_err(|e| e.to_string())?
            }
            _ => return Err("unknown encoded value tag".into()),
        })
    }
}

pub struct StateCodec;
impl Codec<tla_engine::Leveled<Vec<Value>>, Vec<Value>> for StateCodec {
    fn state(&self, state: &tla_engine::Leveled<Vec<Value>>) -> Result<Vec<u8>, String> {
        if state.depth == 0 {
            return Err("stored state depth must be positive".into());
        }
        let mut bytes = b"TLS1".to_vec();
        bytes.extend(
            u64::try_from(state.depth)
                .map_err(|e| e.to_string())?
                .to_le_bytes(),
        );
        bytes.extend(ValueCodec.state(&state.values)?);
        Ok(bytes)
    }
    fn key(&self, key: &Vec<Value>) -> Result<Vec<u8>, String> {
        ValueCodec.key(key)
    }
    fn decode(&self, bytes: &[u8]) -> Result<tla_engine::Leveled<Vec<Value>>, String> {
        if bytes.len() < 12 || &bytes[..4] != b"TLS1" {
            return Err("invalid state header".into());
        }
        let depth = usize::try_from(u64::from_le_bytes(bytes[4..12].try_into().unwrap()))
            .map_err(|e| e.to_string())?;
        if depth == 0 {
            return Err("stored state depth must be positive".into());
        }
        Ok(tla_engine::Leveled {
            values: ValueCodec.decode(&bytes[12..])?,
            depth,
        })
    }
}
