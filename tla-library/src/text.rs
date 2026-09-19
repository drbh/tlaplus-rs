pub use tla_value_expr::stringify;
use tla_value_expr::{Expr, FunctionKind, quoted};
use tla_values::{Error, Value};

#[derive(Clone, Copy)]
pub enum BooleanContext {
    Not,
    If,
    InitialIf,
    NextIf,
    EnabledIf,
    Predicate,
}

pub fn boolean(
    value: &Value,
    context: BooleanContext,
    source: Option<&str>,
) -> Result<bool, Error> {
    if let Value::Bool(value) = value {
        return Ok(*value);
    }
    let kind = match value {
        Value::Int(_) => Some("an integer"),
        Value::String(_) => Some("a string"),
        Value::Model(_) => Some("a model value"),
        _ => None,
    };
    let (mut message, separator) = match (context, kind) {
        (BooleanContext::Not, Some(kind)) => (
            format!("Attempted to apply the operator ~ to a non-boolean\n({kind})"),
            "\n",
        ),
        (BooleanContext::If, Some(kind)) => (
            format!("A non-boolean expression ({kind}) was used as the condition of an IF."),
            "\n",
        ),
        (BooleanContext::InitialIf, Some(kind)) => (
            format!(
                "In computing initial states, a non-boolean expression ({kind}) was used as the condition of an IF."
            ),
            "\n",
        ),
        (BooleanContext::NextIf, Some(kind)) => (
            format!(
                "In computing next states, a non-boolean expression ({kind}) was used as the condition of an IF."
            ),
            "",
        ),
        (BooleanContext::EnabledIf, Some(kind)) => (
            format!(
                "In computing ENABLED, a non-boolean expression({kind}) was used as the guard condition of an IF."
            ),
            "\n",
        ),
        (BooleanContext::Predicate, _) => (
            "TLC expected a boolean value, but did not find one.".to_owned(),
            " ",
        ),
        _ => return Err(Error::Type("boolean")),
    };
    if let Some(source) = source {
        message.push_str(separator);
        message.push_str(source);
    }
    Err(Error::BooleanContext(message))
}

pub fn contextual_error(program: &tla_ir::Program, id: usize, error: String) -> String {
    if matches!(error.as_str(), "Incomparable" | "recursion limit exceeded") {
        return format!("{error} at expression {id}: {:?}", program.nodes[id].expr);
    }
    if error.starts_with("Attempted to apply the operator ~ to a non-boolean\n")
        && !error.contains("\nline ")
        && let Some(source) = program.source(id)
    {
        return format!("{error}\n{source}");
    }
    error
}

pub fn validate_method_args(method: Option<&str>, args: &[Value]) -> Result<(), crate::Error> {
    if args.iter().all(|value| matches!(value, Value::Int(_))) {
        return Ok(());
    }
    let Some(method) = method else { return Ok(()) };
    let Some((_, parameters)) = method.rsplit_once('(') else {
        return Ok(());
    };
    for (parameter, value) in parameters.trim_end_matches(')').split(',').zip(args) {
        if parameter != "tlc2.value.impl.IntValue" || matches!(value, Value::Int(_)) {
            continue;
        }
        let class = match value {
            Value::Bool(_) => "BoolValue",
            Value::String(_) => "StringValue",
            Value::Model(_) => "ModelValue",
            _ => return Ok(()),
        };
        return Err(crate::Error(format!(
            "Attempted to apply the operator overridden by the Java method\n{method},\nbut it produced the following error:\nCannot cast tlc2.value.impl.{class} to {parameter}"
        )));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonMode {
    Value,
    Array,
    Object,
}
pub fn integer_comparison(
    op: tla_ir::Builtin,
    left: &Expr,
    right: &Expr,
) -> Result<bool, crate::Error> {
    use tla_ir::Builtin as B;
    let name = match op {
        B::Lt => "<",
        B::Le => "<=",
        B::Gt | B::NaturalsGe => ">",
        B::Ge => ">=",
        _ => return Err(crate::Error("invalid integer comparison".into())),
    };
    let integer = |value: &Expr, position: &str| match value {
        Expr::Canonical(Value::Int(n)) => Ok(*n),
        _ => Err(crate::Error(format!(
            "The {position} argument of {name} should be an integer, but instead it is:\n{}",
            stringify(value)?
        ))),
    };
    let a = integer(left, "first")?;
    let b = integer(right, "second")?;
    Ok(match op {
        B::Lt => a < b,
        B::Le => a <= b,
        B::Gt => a > b,
        _ => a >= b,
    })
}
pub fn contains(set: &Expr, value: &Expr) -> Result<bool, crate::Error> {
    match (set, set.contains(value)) {
        (Expr::Canonical(Value::String(_)), Err(_)) => Err(crate::Error(format!(
            "Attempted to check if the value:\n{}\nis an element of the string {}",
            stringify(value)?,
            stringify(set)?
        ))),
        (Expr::Infinite(_), Err(Error::Type("infinite set member"))) => Err(crate::Error(format!(
            "Attempted to check if the value:\n{}\nis an element of {}.",
            stringify(value)?,
            stringify(set)?
        ))),
        (_, result) => result.map_err(Into::into),
    }
}
pub fn apply(function: &Expr, argument: &Expr) -> Result<Expr, crate::Error> {
    let error = match function.apply(argument) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let detail = match (function, &error) {
        (
            Expr::Function {
                kind: FunctionKind::Tuple,
                ..
            },
            Error::Undefined,
        ) => {
            format!(
                "Attempted to access index {} of tuple\n{}\nwhich is out of bounds.",
                stringify(argument)?,
                stringify(function)?
            )
        }
        (
            Expr::Function {
                kind: FunctionKind::Record,
                ..
            },
            Error::Undefined,
        ) => {
            format!(
                "Attempted to select nonexistent field {} from the record\n{}",
                stringify(argument)?,
                stringify(function)?
            )
        }
        (Expr::Interval(..), Error::Type("function")) => {
            "A non-function (a set of the form i..j) was applied as a function.".into()
        }
        _ => return Err(error.into()),
    };
    Err(crate::Error(detail))
}
pub fn sequence_operand(value: &Value, operation: &str) -> Result<Vec<Value>, crate::Error> {
    if let Ok(sequence) = value.sequence() {
        return Ok(sequence);
    }
    let prefix = match operation {
        "Append" => {
            "Evaluating an expression of the form Append(s, v) when s is not a sequence:".into()
        }
        "SubSeq" => "The first argument of SubSeq should be a sequence, but instead it is:".into(),
        name => format!("The argument of {name} should be a sequence, but instead it is:"),
    };
    Err(crate::Error(format!(
        "{prefix}\n{}",
        stringify(&Expr::atom(value.clone()))?
    )))
}

pub fn anti_function(value: &Value) -> Result<Value, crate::Error> {
    let Value::Function(entries) = value else {
        return Err(crate::Error("AntiFunction expects a function".into()));
    };
    let mut inverse: Vec<(Value, Value)> = Vec::with_capacity(entries.len());
    for (key, result) in entries.iter() {
        if inverse.iter().any(|(_, value)| value == result) {
            return Err(crate::Error(
                "AntiFunction requires distinct values in the function range".into(),
            ));
        }
        inverse.push((result.clone(), key.clone()));
    }
    Ok(Value::function(inverse)?)
}
pub fn function_domain_error(function: &Expr, argument: &Expr) -> Result<String, Error> {
    Ok(format!(
        "In applying the function\n{},\nthe first argument is:\n{}\nwhich is not in its domain.\n",
        stringify(function)?,
        stringify(argument)?
    ))
}
pub fn subsequence_range(
    value: &Value,
    from: &Value,
    to: &Value,
    len: usize,
) -> Result<std::ops::Range<usize>, crate::Error> {
    let integer = |value: &Value, position| match value {
        Value::Int(n) => Ok(*n),
        _ => Err(crate::Error(format!(
            "The {position} argument of SubSeq should be a natural number, but instead it is:\n{}",
            stringify(&Expr::atom(value.clone()))?
        ))),
    };
    let (from, to) = (integer(from, "second")?, integer(to, "third")?);
    if from > to {
        return Ok(0..0);
    }
    for (bound, position) in [(from, "second"), (to, "third")] {
        if bound < 1 || bound as usize > len {
            return Err(crate::Error(format!(
                "The {position} argument of SubSeq must be in the domain of its first argument:\n{}\n, but instead it is\n{bound}",
                stringify(&Expr::atom(value.clone()))?
            )));
        }
    }
    Ok((from - 1) as usize..to as usize)
}
fn joined(
    values: impl IntoIterator<Item = Result<String, Error>>,
    left: &str,
    separator: &str,
    right: &str,
) -> Result<String, Error> {
    Ok(format!(
        "{left}{}{right}",
        values
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .join(separator)
    ))
}
pub fn json(expr: &Expr, mode: JsonMode) -> Result<String, Error> {
    match expr {
        Expr::Canonical(Value::Int(v)) if mode == JsonMode::Value => Ok(v.to_string()),
        Expr::Canonical(Value::Bool(v)) if mode == JsonMode::Value => Ok(v.to_string()),
        Expr::Canonical(Value::String(a) | Value::Model(a)) if mode == JsonMode::Value => {
            Ok(quoted(&a.text))
        }
        Expr::Function { kind, entries } => {
            if *kind == FunctionKind::Record {
                if mode == JsonMode::Array {
                    return Err(Error::Type("JSON array"));
                }
                return object(
                    entries
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                );
            }
            if *kind == FunctionKind::Tuple && mode == JsonMode::Object {
                return object(
                    entries
                        .iter()
                        .enumerate()
                        .map(|(i, (_, v))| (Value::string(i.to_string(), i as u32), v.clone()))
                        .collect(),
                );
            }
            if let Ok(sequence) = expr.sequence() {
                return joined(
                    sequence.iter().map(|v| json(v, JsonMode::Value)),
                    "[",
                    ",",
                    "]",
                );
            }
            object(expr.entries()?)
        }
        Expr::Lambda(_) | Expr::LambdaMulti { .. } => json(
            &Expr::Function {
                kind: FunctionKind::Explicit,
                entries: expr.entries()?,
            },
            mode,
        ),
        Expr::Canonical(Value::Function(_) | Value::Set(_)) => {
            json(&Expr::expanded(expr.normalize(100_000)?), mode)
        }
        _ if mode != JsonMode::Object => joined(
            expr.sorted(100_000)?
                .iter()
                .map(|v| json(v, JsonMode::Value)),
            "[",
            ",",
            "]",
        ),
        _ => Err(Error::Type("JSON object")),
    }
}
fn object(entries: Vec<(Value, Expr)>) -> Result<String, Error> {
    let mut fields = Vec::<(String, String)>::new();
    for (key, value) in entries {
        let key = if let Value::String(a) = key {
            a.text.to_string()
        } else {
            stringify(&Expr::atom(key))?
        };
        let value = json(&value, JsonMode::Value)?;
        if let Some((_, old)) = fields.iter_mut().find(|(k, _)| k == &key) {
            *old = value;
        } else {
            fields.push((key, value));
        }
    }
    joined(
        fields
            .into_iter()
            .map(|(key, value)| Ok(format!("{}:{value}", quoted(&key)))),
        "{",
        ",",
        "}",
    )
}
pub fn faithful_error_text(error: &str) -> Option<String> {
    if let Some(payload) = error.strip_prefix("assertion failed: ") {
        return Some(format!(
            "The first argument of Assert evaluated to FALSE; the second argument was:\n{payload}"
        ));
    }
    if let Some(value) = error
        .strip_prefix("non-positive modulus: ")
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value <= 0)
    {
        return Some(format!(
            "The second argument of % should be a positive number, but instead it is:\n{value}"
        ));
    }
    Some(
        match error {
            "division by zero" => "The second argument of \\div is 0.",
            "Head of empty sequence" => "Attempted to apply Head to the empty sequence.",
            "Tail of empty sequence" => "Attempted to apply Tail to the empty sequence.",
            _ => return None,
        }
        .into(),
    )
}
