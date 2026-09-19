//! Refinements are predicates; finite enumeration domains are separate definitions.
use crate::{Error, Location, ast::*, walk};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn lower(spec: &mut Spec) -> Result<(), Error> {
    let mut finite: BTreeSet<_> = spec.enums.iter().map(|e| e.name.text.clone()).collect();
    let mut types = BTreeMap::new();
    for (name, fields) in &spec.records {
        for (_, ty) in fields {
            check_record_base(ty, &types, &finite, name.at)?;
        }
        let fields = fields
            .iter()
            .map(|(n, ty)| (n.text.clone(), resolve(ty, &types)))
            .collect();
        types.insert(name.text.clone(), Type::Record(name.text.clone(), fields));
    }
    let mut explicit = BTreeMap::new();
    for (name, value) in std::mem::take(&mut spec.domains) {
        if explicit.insert(name.text, (name.at, value)).is_some() {
            return Err(err(name.at, "duplicate exploration domain"));
        }
    }
    for refinement in spec.refinements.clone() {
        let base = resolve(&refinement.base, &types);
        if !value_type(&base) {
            return Err(err(
                refinement.name.at,
                "refinements must describe values, not operators",
            ));
        }
        let name = &refinement.name;
        if types.contains_key(&name.text) || finite.contains(&name.text) {
            return Err(err(name.at, "duplicate identifier for refinement"));
        }
        let ty = Type::Refined(Box::new(base.clone()), name.text.clone());
        let binding = Name {
            text: format!("__tsl_value_{}", name.text),
            at: Location { line: 0, column: 0 },
        };
        let value = named(&binding.text, binding.at);
        let replacements = BTreeMap::from([(refinement.binding.text.clone(), value.clone())]);
        let body = crate::composition::substitute(&refinement.predicate, &replacements, &mut 0);
        let test = Expr {
            at: name.at,
            kind: Kind::TotalTest(base.clone(), Box::new(value.clone())),
        };
        spec.functions.push(Function {
            name: Name {
                text: predicate(&name.text),
                at: Location { line: 0, column: 0 },
            },
            params: vec![(binding.clone(), base.clone())],
            result: Type::Bool,
            body: Expr {
                at: name.at,
                kind: Kind::Binary("&&".into(), Box::new(test), Box::new(body)),
            },
            recursive: false,
            stateful: false,
        });
        let domain = if let Some((_, value)) = explicit.remove(&name.text) {
            Some(value)
        } else {
            domain(&base, &finite, name.at)
        };
        if let Some(domain) = domain {
            spec.constants.push(Binding {
                name: Name {
                    text: name.text.clone(),
                    at: Location { line: 0, column: 0 },
                },
                ty: Type::Set(Box::new(erase(&base))),
                choice: false,
                initialized: true,
                value: Expr {
                    at: name.at,
                    kind: Kind::Binder(
                        "filter".into(),
                        binding,
                        Box::new(domain),
                        Box::new(Expr {
                            at: name.at,
                            kind: Kind::TypeTest(ty.clone(), Box::new(value)),
                        }),
                    ),
                },
            });
            finite.insert(name.text.clone());
        }
        types.insert(name.text.clone(), ty);
    }
    if let Some((_, (at, _))) = explicit.into_iter().next() {
        return Err(err(at, "domain declaration must name a refinement type"));
    }
    for (_, fields) in &mut spec.records {
        for (_, ty) in fields {
            *ty = resolve(ty, &types);
        }
    }
    for b in spec.constants.iter_mut().chain(&mut spec.states) {
        b.ty = resolve(&b.ty, &types);
    }
    for f in &mut spec.functions {
        for (_, ty) in &mut f.params {
            *ty = resolve(ty, &types);
        }
        f.result = resolve(&f.result, &types);
    }
    walk::spec(spec, &mut |e| {
        if let Kind::Binary(op, value, rhs) = &e.kind
            && op == "is"
            && let Kind::Name(name) = &rhs.kind
            && let Some(ty) = types.get(name)
        {
            e.kind = Kind::TypeTest(ty.clone(), value.clone());
        }
        if let Kind::Lambda(_, ty, _) = &mut e.kind {
            *ty = resolve(ty, &types);
        }
        Ok(())
    })
}
pub(crate) fn predicate(name: &str) -> String {
    format!("__tsl_is_{name}")
}
fn err(location: Location, message: &str) -> Error {
    Error {
        location,
        message: message.into(),
    }
}
fn named(name: &str, at: Location) -> Expr {
    Expr {
        at,
        kind: Kind::Name(name.into()),
    }
}
fn domain(ty: &Type, finite: &BTreeSet<String>, at: Location) -> Option<Expr> {
    let kind = match ty {
        Type::Enum(n) | Type::Refined(_, n) if finite.contains(n) => Kind::Name(n.clone()),
        Type::Bool => Kind::Collection(
            true,
            vec![
                Expr {
                    at,
                    kind: Kind::Bool(false),
                },
                Expr {
                    at,
                    kind: Kind::Bool(true),
                },
            ],
        ),
        Type::Set(t) => Kind::Call("powerset".into(), vec![domain(t, finite, at)?]),
        Type::Map(k, v) => Kind::Call(
            "functions".into(),
            vec![domain(k, finite, at)?, domain(v, finite, at)?],
        ),
        _ => return None,
    };
    Some(Expr { at, kind })
}
fn resolve(ty: &Type, names: &BTreeMap<String, Type>) -> Type {
    match ty {
        Type::Enum(n) => names.get(n).cloned().unwrap_or_else(|| ty.clone()),
        Type::Refined(t, n) => Type::Refined(Box::new(resolve(t, names)), n.clone()),
        Type::Record(n, fs) => Type::Record(
            n.clone(),
            fs.iter()
                .map(|(f, t)| (f.clone(), resolve(t, names)))
                .collect(),
        ),
        Type::Set(t) => Type::Set(Box::new(resolve(t, names))),
        Type::Seq(t) => Type::Seq(Box::new(resolve(t, names))),
        Type::Map(k, v) => Type::Map(Box::new(resolve(k, names)), Box::new(resolve(v, names))),
        Type::Function(args, result) => Type::Function(
            args.iter().map(|t| resolve(t, names)).collect(),
            Box::new(resolve(result, names)),
        ),
        Type::Tuple(items) => Type::Tuple(items.iter().map(|t| resolve(t, names)).collect()),
        _ => ty.clone(),
    }
}
pub(crate) fn erase(ty: &Type) -> Type {
    match ty {
        Type::Refined(t, _) => erase(t),
        Type::Record(n, fs) => Type::Record(
            n.clone(),
            fs.iter().map(|(f, t)| (f.clone(), erase(t))).collect(),
        ),
        Type::Set(t) => Type::Set(Box::new(erase(t))),
        Type::Seq(t) => Type::Seq(Box::new(erase(t))),
        Type::Map(k, v) => Type::Map(Box::new(erase(k)), Box::new(erase(v))),
        Type::Function(args, result) => {
            Type::Function(args.iter().map(erase).collect(), Box::new(erase(result)))
        }
        Type::Tuple(items) => Type::Tuple(items.iter().map(erase).collect()),
        _ => ty.clone(),
    }
}

fn value_type(ty: &Type) -> bool {
    match ty {
        Type::Function(..) | Type::Unknown => false,
        Type::Set(t) | Type::Seq(t) | Type::Refined(t, _) => value_type(t),
        Type::Map(k, v) => value_type(k) && value_type(v),
        Type::Record(_, fields) => fields.iter().all(|(_, t)| value_type(t)),
        Type::Tuple(items) => items.iter().all(value_type),
        _ => true,
    }
}

fn check_record_base(
    ty: &Type,
    records: &BTreeMap<String, Type>,
    enums: &BTreeSet<String>,
    at: Location,
) -> Result<(), Error> {
    match ty {
        Type::Enum(n) if !records.contains_key(n) && !enums.contains(n) => {
            return Err(err(
                at,
                "struct fields require base types, enums, or earlier structs; refine the whole struct for value constraints",
            ));
        }
        Type::Set(t) | Type::Seq(t) => check_record_base(t, records, enums, at)?,
        Type::Map(k, v) => {
            check_record_base(k, records, enums, at)?;
            check_record_base(v, records, enums, at)?;
        }
        Type::Function(..) => return Err(err(at, "struct fields must contain values")),
        _ => {}
    }
    Ok(())
}
