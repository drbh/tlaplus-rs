use crate::{Error, Expr, FunctionKind, InfiniteSet, Value};
use std::cmp::Ordering;
impl Expr {
    pub fn entries(&self) -> Result<Vec<(Value, Self)>, Error> {
        let mut entries = match self {
            Self::Function { entries, .. } => entries.clone(),
            Self::Lambda(domain) => domain
                .sorted(100_000)?
                .into_iter()
                .map(|v| Ok((v.normalize(100_000)?, Self::tuple(vec![v]))))
                .collect::<Result<_, Error>>()?,
            Self::LambdaMulti {
                domains,
                tuple_first,
            } => {
                self.size()?;
                Expr::Product(domains.clone())
                    .enumerate(100_000)?
                    .into_iter()
                    .map(|key| {
                        let arguments = key.sequence()?;
                        let mut bindings = if *tuple_first {
                            let mut tuple = arguments[0].sequence()?;
                            if tuple.len() != 2 {
                                return Err(Error::Type("pair"));
                            }
                            tuple.extend_from_slice(&arguments[1..]);
                            tuple
                        } else {
                            arguments
                        };
                        bindings.reverse();
                        Ok((key.normalize(100_000)?, Self::tuple(bindings)))
                    })
                    .collect::<Result<_, Error>>()?
            }
            Self::Canonical(Value::Function(entries)) => entries
                .iter()
                .map(|(k, v)| (k.clone(), Self::atom(v.clone())))
                .collect(),
            _ => return Err(Error::Type("function")),
        };
        let mut error = None;
        entries.sort_by(|(a, _), (b, _)| {
            a.compare(b).unwrap_or_else(|e| {
                error = Some(e);
                Ordering::Equal
            })
        });
        if let Some(e) = error {
            return Err(e);
        }
        if entries.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(Error::DuplicateDomain);
        }
        Ok(entries)
    }
    pub(crate) fn check_record(&self, other: &Self) -> Result<(), Error> {
        if matches!(
            self,
            Self::Function {
                kind: FunctionKind::Record,
                ..
            }
        ) && (matches!(
            other,
            Self::Function {
                kind: FunctionKind::Interval(_),
                ..
            }
        ) || !other
            .entries()?
            .iter()
            .all(|(k, _)| matches!(k, Value::String(_))))
        {
            return Err(Error::Type("record"));
        }
        Ok(())
    }
    pub fn apply(&self, argument: &Self) -> Result<Self, Error> {
        let argument = Self::atom(argument.normalize(100_000)?);
        for (key, value) in self.entries()? {
            if Self::atom(key).semantic_eq(&argument)? {
                return Ok(value);
            }
        }
        Err(Error::Undefined)
    }
    pub fn domain(&self) -> Result<Self, Error> {
        Ok(Self::Set(
            self.entries()?
                .into_iter()
                .map(|(key, _)| Self::atom(key))
                .collect(),
        ))
    }
    pub fn sequence(&self) -> Result<Vec<Self>, Error> {
        if let Self::Function {
            kind: FunctionKind::Interval(low),
            ..
        } = self
            && *low != 1
        {
            return Err(Error::Type("sequence"));
        }
        self.entries()?
            .into_iter()
            .enumerate()
            .map(|(i, (key, value))| {
                if key == Value::Int(i as i32 + 1) {
                    Ok(value)
                } else {
                    Err(Error::Type("sequence"))
                }
            })
            .collect()
    }
    pub fn except(&self, path: &[Value], replacement: Self) -> Result<Self, Error> {
        let Some((head, tail)) = path.split_first() else {
            return Ok(replacement);
        };
        let (kind, mut entries) = match self {
            Self::Function { kind, entries } => (*kind, entries.clone()),
            _ => return Err(Error::Type("function")),
        };
        if kind == FunctionKind::Tuple && !matches!(head, Value::Int(_)) {
            return Ok(replacement);
        }
        if kind == FunctionKind::Record && !matches!(head, Value::String(_)) {
            return Ok(replacement);
        }
        if matches!(kind, FunctionKind::Interval(_)) && !matches!(head, Value::Int(_)) {
            return Ok(self.clone());
        }
        for (key, value) in &mut entries {
            if key.semantic_eq(head)? {
                *value = value.except(tail, replacement)?;
                return Ok(Self::Function { kind, entries });
            }
        }
        if kind == FunctionKind::Tuple {
            for (_, value) in &mut entries {
                *value = Self::Undefined;
            }
            return Ok(Self::Function { kind, entries });
        }
        Ok(self.clone())
    }
}

pub fn quoted(text: &str) -> String {
    serde_json::to_string(text)
        .expect("string JSON serialization")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
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
pub fn stringify(expr: &Expr) -> Result<String, Error> {
    match expr {
        Expr::Canonical(Value::Bool(v)) => Ok(if *v { "TRUE" } else { "FALSE" }.into()),
        Expr::Canonical(Value::Int(v)) => Ok(v.to_string()),
        Expr::Canonical(Value::String(a)) => Ok(quoted(&a.text)),
        Expr::Canonical(Value::Model(a)) => Ok(a.text.to_string()),
        Expr::Canonical(v) => stringify(&Expr::atom(v.clone())),
        Expr::Interval(low, high) => Ok(format!("{low}..{high}")),
        Expr::Infinite(kind) => Ok(match kind {
            InfiniteSet::Any => "ANY",
            InfiniteSet::Naturals => "Nat",
            InfiniteSet::Integers => "Int",
            InfiniteSet::Strings => "STRING",
        }
        .into()),
        Expr::Sequences(base) => Ok(format!("Seq({})", stringify(base)?)),
        Expr::Set(_) => joined(expr.sorted(100_000)?.iter().map(stringify), "{", ", ", "}"),
        Expr::Function { kind, .. } => {
            if *kind == FunctionKind::Record {
                return joined(
                    expr.entries()?.iter().map(|(k, v)| {
                        let Value::String(a) = k else {
                            return Err(Error::Type("record key"));
                        };
                        Ok(format!("{} |-> {}", a.text, stringify(v)?))
                    }),
                    "[",
                    ", ",
                    "]",
                );
            }
            if let Ok(sequence) = expr.sequence() {
                return joined(sequence.iter().map(stringify), "<<", ", ", ">>");
            }
            let entries = expr.entries()?;
            if entries
                .iter()
                .all(|(k, _)| matches!(k,Value::String(a) if identifier(&a.text)))
            {
                return stringify(&Expr::Function {
                    kind: FunctionKind::Record,
                    entries,
                });
            }
            joined(
                entries.iter().map(|(k, v)| {
                    Ok(format!(
                        "{} :> {}",
                        stringify(&Expr::atom(k.clone()))?,
                        stringify(v)?
                    ))
                }),
                "(",
                " @@ ",
                ")",
            )
        }
        Expr::Lambda(_) | Expr::LambdaMulti { .. } => stringify(&Expr::Function {
            kind: FunctionKind::Explicit,
            entries: expr.entries()?,
        }),
        _ => stringify(&Expr::Set(expr.sorted(100_000)?)),
    }
}
fn identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().any(|c| c.is_ascii_alphabetic())
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
