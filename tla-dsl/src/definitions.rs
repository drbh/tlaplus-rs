//! Order reusable definitions by dependency; reject cycles except explicit self recursion.
use crate::{Error, ast::*, refinements, walk};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) fn order(spec: &mut Spec) -> Result<(), Error> {
    let mut nodes = BTreeMap::new();
    for (i, c) in spec.constants.iter().enumerate() {
        nodes.insert(c.name.text.clone(), (false, i));
    }
    for (i, f) in spec.functions.iter().enumerate() {
        nodes.insert(f.name.text.clone(), (true, i));
    }
    let mut active = BTreeSet::new();
    let mut done = BTreeSet::new();
    let mut order = vec![];
    for name in nodes.keys() {
        visit(name, spec, &nodes, &mut active, &mut done, &mut order)?;
    }
    spec.definition_order = order;
    Ok(())
}
fn visit(
    name: &str,
    spec: &Spec,
    nodes: &BTreeMap<String, (bool, usize)>,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
    order: &mut Vec<(bool, usize)>,
) -> Result<(), Error> {
    if done.contains(name) {
        return Ok(());
    }
    let &(function, i) = &nodes[name];
    let (mut expr, at) = if function {
        (spec.functions[i].body.clone(), spec.functions[i].name.at)
    } else {
        (spec.constants[i].value.clone(), spec.constants[i].name.at)
    };
    if active.len() >= 128 {
        return Err(Error {
            location: at,
            message: "definition dependency nesting exceeds 128 levels".into(),
        });
    }
    if !active.insert(name.into()) {
        return Err(Error {
            location: at,
            message: format!("cyclic definition involving {name:?}"),
        });
    }
    let mut refs = BTreeSet::new();
    walk::visit(&mut expr, &mut |e| {
        match &e.kind {
            Kind::Name(n) | Kind::Call(n, _) => {
                refs.insert(n.clone());
            }
            Kind::TypeTest(t, _) | Kind::TotalTest(t, _) => type_refs(t, &mut refs),
            _ => {}
        }
        Ok(())
    })?;
    if function {
        for (_, t) in &spec.functions[i].params {
            type_refs(t, &mut refs);
        }
        type_refs(&spec.functions[i].result, &mut refs);
    } else {
        type_refs(&spec.constants[i].ty, &mut refs);
    }
    for dependency in refs {
        if dependency == name && function && spec.functions[i].recursive {
            continue;
        }
        if let Some(&(is_fn, index)) = nodes.get(&dependency) {
            if is_fn && spec.functions[index].stateful && (!function || !spec.functions[i].stateful)
            {
                return Err(Error {
                    location: at,
                    message: "pure definitions cannot depend on state views".into(),
                });
            }
            visit(&dependency, spec, nodes, active, done, order)?;
        }
    }
    active.remove(name);
    done.insert(name.into());
    order.push((function, i));
    Ok(())
}
pub(crate) fn type_refs(ty: &Type, refs: &mut BTreeSet<String>) {
    match ty {
        Type::Refined(_, name) => {
            refs.insert(refinements::predicate(name));
        }
        Type::Record(_, fields) => {
            for (_, t) in fields {
                type_refs(t, refs);
            }
        }
        Type::Set(t) | Type::Seq(t) => type_refs(t, refs),
        Type::Map(k, v) => {
            type_refs(k, refs);
            type_refs(v, refs);
        }
        Type::Function(args, result) => {
            for t in args {
                type_refs(t, refs);
            }
            type_refs(result, refs);
        }
        Type::Tuple(items) => {
            for t in items {
                type_refs(t, refs);
            }
        }
        _ => {}
    }
}
