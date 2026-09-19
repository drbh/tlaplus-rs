//! Shared expression traversal for lowering and dependency analysis.
use crate::{Error, ast::*};
pub(crate) fn visit(
    expr: &mut Expr,
    f: &mut impl FnMut(&mut Expr) -> Result<(), Error>,
) -> Result<(), Error> {
    match &mut expr.kind {
        Kind::Field(e, _)
        | Kind::Unary(_, e)
        | Kind::Lambda(_, _, e)
        | Kind::TypeTest(_, e)
        | Kind::TotalTest(_, e) => visit(e, f)?,
        Kind::Binary(_, a, b) | Kind::Index(a, b) => {
            visit(a, f)?;
            visit(b, f)?;
        }
        Kind::If(a, b, c) => {
            visit(a, f)?;
            visit(b, f)?;
            visit(c, f)?;
        }
        Kind::Binder(_, _, a, b) => {
            visit(a, f)?;
            visit(b, f)?;
        }
        Kind::Collection(_, items) | Kind::Tuple(items) | Kind::Call(_, items) => {
            for e in items {
                visit(e, f)?;
            }
        }
        Kind::Record(_, fields) => {
            for (_, e) in fields {
                visit(e, f)?;
            }
        }
        _ => {}
    }
    f(expr)
}
pub(crate) fn action(
    action: &mut Action,
    f: &mut impl FnMut(&mut Expr) -> Result<(), Error>,
) -> Result<(), Error> {
    for (_, e) in action
        .params
        .iter_mut()
        .chain(&mut action.picks)
        .chain(&mut action.updates)
    {
        visit(e, f)?;
    }
    for e in &mut action.guards {
        visit(e, f)?;
    }
    for (_, args) in &mut action.calls {
        for e in args {
            visit(e, f)?;
        }
    }
    Ok(())
}
pub(crate) fn spec(
    spec: &mut Spec,
    f: &mut impl FnMut(&mut Expr) -> Result<(), Error>,
) -> Result<(), Error> {
    for b in spec.constants.iter_mut().chain(&mut spec.states) {
        visit(&mut b.value, f)?;
    }
    for fun in &mut spec.functions {
        visit(&mut fun.body, f)?;
    }
    for a in &mut spec.actions {
        action(a, f)?;
    }
    if let Some(a) = &mut spec.init {
        action(a, f)?;
    }
    for p in &mut spec.properties {
        visit(&mut p.body, f)?;
    }
    for e in spec
        .assumptions
        .iter_mut()
        .chain(&mut spec.refinement_checks)
    {
        visit(e, f)?;
    }
    Ok(())
}
