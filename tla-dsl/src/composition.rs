//! Hygienic expansion of atomic action helpers; framing happens after composition.
use crate::{Error, Location, ast::*};
use std::collections::BTreeMap;
pub(crate) fn lower(spec: &mut Spec) -> Result<(), Error> {
    let actions: BTreeMap<_, _> = spec
        .actions
        .iter()
        .map(|a| (a.name.text.clone(), a.clone()))
        .collect();
    let mut serial = 0;
    for action in &mut spec.actions {
        expand(action, &actions, &mut vec![], &mut serial)?;
    }
    if let Some(init) = &mut spec.init {
        expand(init, &actions, &mut vec![], &mut serial)?;
    }
    Ok(())
}
fn fresh(name: &Name, serial: &mut usize) -> Name {
    *serial += 1;
    Name {
        text: format!("__tsl_local_{}_{}", serial, name.text),
        at: Location { line: 0, column: 0 },
    }
}
fn expand(
    action: &mut Action,
    actions: &BTreeMap<String, Action>,
    stack: &mut Vec<String>,
    serial: &mut usize,
) -> Result<(), Error> {
    if stack.contains(&action.name.text) || stack.len() >= 64 {
        return Err(Error {
            location: action.name.at,
            message: "cyclic or excessively nested action calls".into(),
        });
    }
    stack.push(action.name.text.clone());
    for (name, args) in std::mem::take(&mut action.calls) {
        let mut helper = actions.get(&name.text).cloned().ok_or_else(|| Error {
            location: name.at,
            message: format!("unknown action {:?}", name.text),
        })?;
        if args.len() != helper.params.len() {
            return Err(Error {
                location: name.at,
                message: format!(
                    "action {} expects {} arguments",
                    name.text,
                    helper.params.len()
                ),
            });
        }
        expand(&mut helper, actions, stack, serial)?;
        let mut replacements = BTreeMap::new();
        for ((param, domain), arg) in helper.params.iter().zip(args) {
            let domain = substitute(domain, &replacements, serial);
            action.guards.push(Expr {
                at: name.at,
                kind: Kind::Call("contains".into(), vec![domain, arg.clone()]),
            });
            replacements.insert(param.text.clone(), arg);
        }
        for (pick, domain) in &helper.picks {
            let domain = substitute(domain, &replacements, serial);
            let new = fresh(pick, serial);
            replacements.insert(
                pick.text.clone(),
                Expr {
                    at: new.at,
                    kind: Kind::Name(new.text.clone()),
                },
            );
            action.picks.push((new, domain));
        }
        for guard in &helper.guards {
            action.guards.push(substitute(guard, &replacements, serial));
        }
        for (target, value) in &helper.updates {
            action
                .updates
                .push((target.clone(), substitute(value, &replacements, serial)));
        }
        if action.guards.len() + action.updates.len() + action.picks.len() > 4096 {
            return Err(Error {
                location: name.at,
                message: "expanded action exceeds 4096 clauses".into(),
            });
        }
    }
    stack.pop();
    Ok(())
}
pub(crate) fn substitute(
    expr: &Expr,
    replacements: &BTreeMap<String, Expr>,
    serial: &mut usize,
) -> Expr {
    if let Kind::Name(n) = &expr.kind
        && let Some(value) = replacements.get(n)
    {
        return value.clone();
    }
    let mut result = expr.clone();
    match &mut result.kind {
        Kind::Binder(_, name, domain, body) => {
            **domain = substitute(domain, replacements, serial);
            let new = fresh(name, serial);
            let mut local = replacements.clone();
            local.insert(
                name.text.clone(),
                Expr {
                    at: new.at,
                    kind: Kind::Name(new.text.clone()),
                },
            );
            *name = new;
            **body = substitute(body, &local, serial);
        }
        Kind::Lambda(name, _, body) => {
            let new = fresh(name, serial);
            let mut local = replacements.clone();
            local.insert(
                name.text.clone(),
                Expr {
                    at: new.at,
                    kind: Kind::Name(new.text.clone()),
                },
            );
            *name = new;
            **body = substitute(body, &local, serial);
        }
        Kind::Field(e, _) | Kind::Unary(_, e) | Kind::TypeTest(_, e) | Kind::TotalTest(_, e) => {
            **e = substitute(e, replacements, serial)
        }
        Kind::Binary(_, a, b) | Kind::Index(a, b) => {
            **a = substitute(a, replacements, serial);
            **b = substitute(b, replacements, serial);
        }
        Kind::If(a, b, c) => {
            **a = substitute(a, replacements, serial);
            **b = substitute(b, replacements, serial);
            **c = substitute(c, replacements, serial);
        }
        Kind::Collection(_, items) | Kind::Tuple(items) | Kind::Call(_, items) => {
            for e in items {
                *e = substitute(e, replacements, serial);
            }
        }
        Kind::Record(_, fields) => {
            for (_, e) in fields {
                *e = substitute(e, replacements, serial);
            }
        }
        _ => {}
    }
    result
}
