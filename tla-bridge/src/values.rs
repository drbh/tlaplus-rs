//! Conversion between faithful and native values, and canonical keys over
//! faithful values.

use crate::model::{self, ModelError};
pub use tla_model::literal;
pub use tla_oracle::ValueKey;
use tla_tlc_value::{TlcError, Value};

pub fn from_faithful(value: &Value) -> Result<tla_values::Value, TlcError> {
    use tla_values::Value as New;
    let map_error = |e: tla_values::Error| TlcError::runtime(e.to_string());
    value.deep_normalize()?;
    Ok(match value {
        Value::Bool(v) => New::Bool(*v),
        Value::Int(v) => New::Int(*v),
        Value::Str(v) => New::string(v.val.as_str(), v.val.tok() as u32),
        Value::Model(v) => New::model(v.val.as_str(), v.val.tok() as u32),
        _ => {
            if let Some(f) = value.to_fcn_rcd()? {
                New::function(
                    f.domain_as_values()?
                        .iter()
                        .zip(f.values().iter())
                        .map(|(k, v)| Ok((from_faithful(k)?, from_faithful(v)?)))
                        .collect::<Result<_, TlcError>>()?,
                )
                .map_err(map_error)?
            } else if let Some(s) = value.to_set_enum()? {
                New::set(
                    s.elems()
                        .iter()
                        .map(from_faithful)
                        .collect::<Result<_, _>>()?,
                )
                .map_err(map_error)?
            } else {
                return Err(TlcError::runtime("value is not finite"));
            }
        }
    })
}

pub fn to_faithful(value: &tla_values::Value) -> Result<Value, TlcError> {
    use tla_values::Value as New;
    Ok(match value {
        New::Bool(v) => Value::Bool(*v),
        New::Int(v) => Value::Int(*v),
        New::String(a) | New::Model(a) => {
            let token = tla_tlc_value::UniqueString::of(&a.text);
            if token.tok() as u32 != a.ordinal {
                return Err(TlcError::runtime(
                    "atom ordinal belongs to another intern session",
                ));
            }
            if matches!(value, New::String(_)) {
                Value::string(&a.text)
            } else {
                tla_tlc_value::value::mv_make(&a.text)
            }
        }
        New::Set(v) => Value::set_enum(v.iter().map(to_faithful).collect::<Result<_, _>>()?, true),
        New::Function(v) => Value::fcn_rcd(
            v.iter()
                .map(|(k, _)| to_faithful(k))
                .collect::<Result<_, _>>()?,
            v.iter()
                .map(|(_, v)| to_faithful(v))
                .collect::<Result<_, _>>()?,
            true,
        ),
    })
}

pub fn intern_value(text: &str, model: bool) -> Result<tla_values::Value, TlcError> {
    from_faithful(&if model {
        tla_tlc_value::value::mv_make(text)
    } else {
        Value::string(text)
    })
}

pub fn with_value_session<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tla_tlc_eval::in_fresh_jvm(move || {
        tla_tlc_value::value::mv_init();
        f()
    })
}

pub(crate) fn value_key(value: &Value) -> Result<ValueKey, TlcError> {
    value.deep_normalize()?;
    Ok(match value {
        Value::Bool(value) => ValueKey::Bool(*value),
        Value::Int(value) => ValueKey::Int(*value),
        Value::Str(value) => ValueKey::String(value.val.as_str().into()),
        Value::Model(value) => ValueKey::Model(value.val.as_str().into()),
        _ => {
            if let Some(function) = value.to_fcn_rcd()? {
                let mut entries = function
                    .domain_as_values()?
                    .iter()
                    .zip(function.values().iter())
                    .map(|(key, value)| Ok((value_key(key)?, value_key(value)?)))
                    .collect::<Result<Vec<_>, TlcError>>()?;
                entries.sort();
                ValueKey::Function(entries)
            } else if let Some(set) = value.to_set_enum()? {
                let mut members = set
                    .elems()
                    .iter()
                    .map(value_key)
                    .collect::<Result<Vec<_>, _>>()?;
                members.sort();
                members.dedup();
                ValueKey::Set(members)
            } else {
                return Err(TlcError::runtime(format!(
                    "cannot canonicalize {}",
                    value.kind_string()
                )));
            }
        }
    })
}

pub(crate) fn operator_table(
    table: &tla_tlc_value::value::OpRcdValue,
) -> Result<tla_ir::Literal, ModelError> {
    let convert = |value: &tla_tlc_value::Value| {
        from_faithful(value)
            .map(|value| literal(&value))
            .map_err(|e| ModelError::message(model::Stage::Load, e))
    };
    let entries = table
        .domain()
        .into_iter()
        .zip(table.values())
        .map(|(args, value)| {
            let args = args.iter().map(convert).collect::<Result<_, _>>()?;
            Ok((tla_ir::Literal::Tuple(args), convert(&value)?))
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    let mut unique = Vec::new();
    for entry in entries {
        if !unique.iter().any(|(key, _)| key == &entry.0) {
            unique.push(entry);
        }
    }
    Ok(tla_ir::Literal::Function(unique))
}
