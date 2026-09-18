use tla_values::{Error, Value};
#[derive(Clone, Copy, Debug)]
pub enum BagOp {
    Add,
    Difference,
    Subset,
    Union,
}
pub fn bag(op: BagOp, args: &[Value]) -> Result<Value, Error> {
    if args.len() != if matches!(op, BagOp::Union) { 1 } else { 2 } {
        return Err(Error::Type("bag operator arity"));
    }
    if matches!(op, BagOp::Union) {
        let values = args[0].elements()?;
        let Some(first) = values.first() else {
            return Value::function(vec![]);
        };
        if values.len() == 1 {
            return Ok(first.clone());
        }
        let mut entries = first.entries()?.to_vec();
        for value in &values[1..] {
            merge(&mut entries, value.entries()?)?;
        }
        return Value::function(entries);
    }
    let a = args[0].entries()?;
    let b = args[1].entries()?;
    if matches!(op, BagOp::Add) {
        if a.iter()
            .chain(b)
            .any(|(_, v)| !matches!(v,Value::Int(n) if *n>0))
        {
            return Err(Error::Type("bag"));
        }
        let mut entries = a.to_vec();
        merge(&mut entries, b)?;
        return Value::function(entries);
    }
    let mut result = Vec::new();
    for (key, value) in a {
        let mut n = value.integer()?;
        for (other, count) in b {
            if key.semantic_eq(other)? {
                n = n.wrapping_sub(count.integer()?);
                break;
            }
        }
        if n > 0 {
            if matches!(op, BagOp::Subset) {
                return Ok(Value::Bool(false));
            }
            result.push((key.clone(), Value::Int(n)));
        }
    }
    if matches!(op, BagOp::Subset) {
        Ok(Value::Bool(true))
    } else {
        Value::function(result)
    }
}
fn merge(entries: &mut Vec<(Value, Value)>, other: &[(Value, Value)]) -> Result<(), Error> {
    for (key, value) in other {
        let mut found = false;
        for (old_key, old_value) in entries.iter_mut() {
            if key.semantic_eq(old_key)? {
                *old_value = Value::Int(old_value.integer()?.wrapping_add(value.integer()?));
                found = true;
                break;
            }
        }
        if !found {
            entries.push((key.clone(), value.clone()));
        }
    }
    Ok(())
}
