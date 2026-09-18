use tla_bridge::values::intern_value;
use tla_value_expr::{Expr as Runtime, FunctionKind, InfiniteSet, SetOp};
use tla_values::{Atom, Error, Value};
#[derive(Debug)]
pub enum Expr {
    Atom(String),
    String(String),
    List(Vec<Expr>),
}
#[derive(Debug)]
pub enum Failure {
    Unsupported(String),
    Value(Error),
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Value(error)
    }
}
impl Expr {
    pub fn atom(&self) -> &str {
        if let Self::Atom(s) = self {
            s
        } else {
            panic!("expected token")
        }
    }
    fn ordinal(&self) -> Atom {
        let Value::String(atom) = intern_value(self.atom(), false).unwrap() else {
            unreachable!()
        };
        atom
    }
    pub fn eval(&self) -> Result<Runtime, Failure> {
        let expression = match self {
            Self::Atom(a) => {
                return Ok(Runtime::atom(match a.as_str() {
                    "TRUE" => Value::Bool(true),
                    "FALSE" => Value::Bool(false),
                    _ if a.starts_with('@') => intern_value(&a[1..], true).unwrap(),
                    _ => Value::Int(a.parse().map_err(|_| Failure::Unsupported(a.clone()))?),
                }));
            }
            Self::String(v) => return Ok(Runtime::atom(intern_value(v, false).unwrap())),
            Self::List(list) => list,
        };
        let op = expression[0].atom();
        let args = &expression[1..];
        let eval = |i: usize| args[i].eval();
        let values = || args.iter().map(Expr::eval).collect::<Result<Vec<_>, _>>();
        Ok(match op {
            "set" | "nset" => Runtime::Set(values()?),
            "iv" => Runtime::Interval(
                args[0].atom().parse().unwrap(),
                args[1].atom().parse().unwrap(),
            ),
            "tup" => Runtime::tuple(values()?),
            "fcn" => Runtime::Function {
                kind: FunctionKind::Explicit,
                entries: values()?
                    .chunks_exact(2)
                    .map(|p| Ok((p[0].normalize(100_000)?, p[1].clone())))
                    .collect::<Result<_, Error>>()?,
            },
            "fcni" => {
                let low: i32 = args[0].atom().parse().unwrap();
                Runtime::Function {
                    kind: FunctionKind::Interval(low),
                    entries: args[1..]
                        .iter()
                        .enumerate()
                        .map(|(i, e)| Ok((Value::Int(low + i as i32), e.eval()?)))
                        .collect::<Result<_, Failure>>()?,
                }
            }
            "rcd" => Runtime::Function {
                kind: FunctionKind::Record,
                entries: args
                    .chunks_exact(2)
                    .map(|p| Ok((Value::String(p[0].ordinal()), p[1].eval()?)))
                    .collect::<Result<_, Failure>>()?,
            },
            "rcds" => {
                let fields = args
                    .chunks_exact(2)
                    .map(|p| Ok((p[0].ordinal(), p[1].eval()?)))
                    .collect::<Result<Vec<_>, Failure>>()?;
                if fields
                    .iter()
                    .enumerate()
                    .any(|(i, (n, _))| fields[..i].iter().any(|(m, _)| n == m))
                {
                    return Err(Error::DuplicateDomain.into());
                }
                Runtime::Records(fields)
            }
            "cup" | "cap" | "diff" => Runtime::Binary(
                match op {
                    "cup" => SetOp::Union,
                    "cap" => SetOp::Intersection,
                    _ => SetOp::Difference,
                },
                Box::new(eval(0)?),
                Box::new(eval(1)?),
            ),
            "rcup" | "rcap" | "rdiff" => reduce(op, eval(0)?, eval(1)?)?,
            "subset" => Runtime::Subsets {
                base: Box::new(eval(0)?),
                size: None,
            },
            "ksubset" => Runtime::Subsets {
                base: Box::new(eval(1)?),
                size: Some(args[0].atom().parse().unwrap()),
            },
            "union" => Runtime::Union(Box::new(eval(0)?)),
            "unionf" => flatten_union(eval(0)?)?,
            "fcns" => Runtime::Functions {
                domain: Box::new(eval(0)?),
                range: Box::new(eval(1)?),
            },
            "prod" => Runtime::Product(values()?),
            "except" => {
                let v = values()?;
                v[0].except(
                    &v[2..]
                        .iter()
                        .map(|v| v.normalize(100_000))
                        .collect::<Result<Vec<_>, _>>()?,
                    v[1].clone(),
                )?
            }
            "apply" => eval(0)?.apply(&eval(1)?)?,
            "dom" => eval(0)?.domain()?,
            "tofcn" => Runtime::Function {
                kind: FunctionKind::Explicit,
                entries: eval(0)?.entries()?,
            },
            "totup" => Runtime::tuple(eval(0)?.sequence()?),
            "tosetenum" => Runtime::Set(eval(0)?.enumerate(100_000)?),
            "torcd" => to_record(eval(0)?)?,
            "setpred" | "setpredt" => Runtime::Filter {
                base: Box::new(eval(0)?),
                excluded: Box::new(eval(1)?),
                tuple: op == "setpredt",
            },
            "lambda" => {
                let domain = eval(0)?;
                intern_value("x", false).unwrap();
                Runtime::Lambda(Box::new(domain))
            }
            "lambda2" | "lambdaxy" | "lambdat" => {
                let domains = if op == "lambdaxy" {
                    let d = eval(0)?;
                    vec![d.clone(), d]
                } else {
                    values()?
                };
                for name in if op == "lambdat" {
                    vec!["x", "y", "z"]
                } else {
                    vec!["x", "y"]
                } {
                    intern_value(name, false).unwrap();
                }
                Runtime::LambdaMulti {
                    domains,
                    tuple_first: op == "lambdat",
                }
            }
            "nat" => Runtime::Infinite(InfiniteSet::Naturals),
            "int" => Runtime::Infinite(InfiniteSet::Integers),
            "string" => Runtime::Infinite(InfiniteSet::Strings),
            "seq" => Runtime::Sequences(Box::new(eval(0)?)),
            _ => return Err(Failure::Unsupported(op.into())),
        })
    }
}
/// `rcup`, `rcap` and `rdiff` reduce a set operation to an enumerated set.
fn reduce(op: &str, a: Runtime, b: Runtime) -> Result<Runtime, Failure> {
    if !matches!(a, Runtime::Set(_) | Runtime::Interval(..)) {
        return Err(Error::Type("reducible set").into());
    }
    if op == "rcup" {
        let mut values = match &a {
            Runtime::Set(v) => v.clone(),
            _ => a.enumerate(100_000)?,
        };
        for v in b.enumerate(100_000)? {
            if !a.contains(&v)? {
                values.push(v);
            }
        }
        return Ok(Runtime::Set(values));
    }
    let operation = Runtime::Binary(
        match op {
            "rcup" => SetOp::Union,
            "rcap" => SetOp::Intersection,
            _ => SetOp::Difference,
        },
        Box::new(a),
        Box::new(b),
    );
    Ok(Runtime::Set(operation.enumerate(100_000)?))
}

/// `unionf` flattens a set of enumerated sets without duplicates, and falls
/// back to the lazy union otherwise.
fn flatten_union(base: Runtime) -> Result<Runtime, Failure> {
    if let Runtime::Set(sets) = &base
        && sets.iter().all(|s| matches!(s, Runtime::Set(_)))
    {
        let mut values = Vec::<Runtime>::new();
        for set in sets {
            let Runtime::Set(members) = set else {
                unreachable!()
            };
            for v in members {
                let mut found = false;
                for previous in &values {
                    if previous.semantic_eq(v)? {
                        found = true;
                        break;
                    }
                }
                if !found {
                    values.push(v.clone());
                }
            }
        }
        return Ok(Runtime::Set(values));
    }
    Ok(Runtime::Union(Box::new(base)))
}

/// `torcd` views a function with string keys as a record.
fn to_record(source: Runtime) -> Result<Runtime, Failure> {
    if let Runtime::Function {
        kind: FunctionKind::Record,
        ..
    } = source
    {
        return Ok(source);
    }
    if matches!(
        source,
        Runtime::Function {
            kind: FunctionKind::Interval(_),
            ..
        }
    ) {
        return Err(Error::Type("record").into());
    }
    let entries = source.entries()?;
    if entries.iter().any(|(k, _)| !matches!(k, Value::String(_))) {
        return Err(Error::Type("record").into());
    }
    Ok(Runtime::Function {
        kind: FunctionKind::Record,
        entries,
    })
}

pub fn parse(text: &str) -> Vec<Expr> {
    fn node(bytes: &[u8], cursor: &mut usize) -> Expr {
        while bytes[*cursor].is_ascii_whitespace() {
            *cursor += 1;
        }
        if bytes[*cursor] == b'(' {
            *cursor += 1;
            let mut children = Vec::new();
            loop {
                while bytes[*cursor].is_ascii_whitespace() {
                    *cursor += 1;
                }
                if bytes[*cursor] == b')' {
                    *cursor += 1;
                    break;
                }
                children.push(node(bytes, cursor));
            }
            return Expr::List(children);
        }
        let start = *cursor;
        if bytes[start] == b'"' {
            *cursor += 1;
            loop {
                match bytes[*cursor] {
                    b'\\' => *cursor += 2,
                    b'"' => {
                        *cursor += 1;
                        break;
                    }
                    _ => *cursor += 1,
                }
            }
            let text: String = serde_json::from_slice(&bytes[start..*cursor]).unwrap();
            return Expr::String(text);
        }
        while *cursor < bytes.len()
            && !bytes[*cursor].is_ascii_whitespace()
            && bytes[*cursor] != b')'
        {
            *cursor += 1;
        }
        let atom = String::from_utf8(bytes[start..*cursor].to_vec()).unwrap();
        Expr::Atom(atom)
    }
    let mut cursor = 0;
    let mut nodes = Vec::new();
    while cursor < text.len() {
        while cursor < text.len() && text.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor < text.len() {
            nodes.push(node(text.as_bytes(), &mut cursor));
        }
    }
    nodes
}
