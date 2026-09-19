//! The larger expression forms, each built from its tree-sitter node.
use crate::binders::{function, intro, names_of, nest, product};
use crate::expressions::{all, expression, get, is_product, key_or_tuple, symbol};
use crate::modules::{children, error, field, text};
use tla_syntax::{Diagnostic, Expr};
use tree_sitter::Node;

/// `[f EXCEPT ![a][b] = e, ...]` as `$Except(f, <<<<a, b>>, e>>, ...)`;
/// record fields become string keys.
pub(super) fn except(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let mut args = vec![get(node, "expr_to_update", source)?];
    for update in children(node)
        .into_iter()
        .filter(|n| n.kind() == "except_update")
    {
        let mut path = Vec::new();
        for specifier in children(update)
            .into_iter()
            .filter(|n| n.kind() == "except_update_specifier")
            .flat_map(children)
        {
            let parts = children(specifier);
            path.push(if specifier.kind() == "except_update_record_field" {
                Expr::String(text(parts[0], source).into())
            } else {
                key_or_tuple(all(&parts, source)?)
            });
        }
        args.push(Expr::Tuple(vec![
            Expr::Tuple(path),
            expression(field(update, "new_val")?, source)?,
        ]));
    }
    Ok(Expr::Call("$Except".into(), args))
}

/// `CASE c -> e [] OTHER -> d` as `$Case(<<c, e>>, <<TRUE, d>>)`.
pub(super) fn case(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let mut arms = Vec::new();
    for arm in children(node)
        .into_iter()
        .filter(|n| matches!(n.kind(), "case_arm" | "other_arm"))
    {
        let parts: Vec<_> = children(arm)
            .into_iter()
            .filter(|n| n.kind() != "case_arrow")
            .collect();
        let condition = if arm.kind() == "other_arm" {
            Expr::Name("TRUE".into())
        } else {
            expression(parts[0], source)?
        };
        let value = *parts
            .last()
            .ok_or_else(|| error(arm, "missing case value"))?;
        arms.push(Expr::Tuple(vec![condition, expression(value, source)?]));
    }
    Ok(Expr::Call("$Case".into(), arms))
}

/// `LET ... IN e`, with `RECURSIVE` declarations marking their definitions.
pub(super) fn let_in(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let mut cursor = node.walk();
    let (declarations, definitions): (Vec<_>, Vec<_>) = node
        .children_by_field_name("definitions", &mut cursor)
        .partition(|n| n.kind() == "recursive_declaration");
    let mut definitions = definitions
        .into_iter()
        .map(|n| crate::modules::definition(n, source))
        .collect::<Result<Vec<_>, _>>()?;
    let recursive: Vec<_> = declarations
        .into_iter()
        .flat_map(|n| crate::modules::declared(n, source))
        .collect();
    crate::modules::mark_recursive(&mut definitions, &recursive);
    Ok(Expr::Let(
        definitions,
        Box::new(get(node, "expression", source)?),
    ))
}

/// `\\E x, y : P` and the temporal `\\EE`, nested one name at a time.
pub(super) fn unbounded_quantification(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let kind = match field(node, "quantifier")?.kind() {
        "exists" => "\\E",
        "forall" => "\\A",
        "temporal_exists" => "\\EE",
        _ => "\\AA",
    };
    let mut cursor = node.walk();
    let names: Vec<_> = node
        .children_by_field_name("intro", &mut cursor)
        .filter(|n| n.is_named())
        .map(|n| text(n, source).to_owned())
        .collect();
    let body = get(node, "expression", source)?;
    Ok(names
        .into_iter()
        .rev()
        .fold(body, |body, name| Expr::Bound {
            kind: kind.into(),
            names: vec![name],
            tuple: false,
            domain: None,
            body: Box::new(body),
        }))
}

/// `{e : x \\in S, y \\in T}`. A single bound, or a single tuple pattern,
/// nests directly; several bounds range over their product as one tuple.
pub(super) fn set_map(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let nodes = children(node);
    let bounds: Vec<_> = nodes
        .iter()
        .copied()
        .filter(|n| n.kind() == "quantifier_bound")
        .collect();
    let body = get(node, "map", source)?;
    let single = bounds.len() == 1 && {
        let (names, tuple) = names_of(bounds[0], source);
        tuple || names.len() == 1
    };
    if single {
        return nest(&bounds, body, "map", source);
    }
    let (names, domain) = product(&bounds, source)?;
    Ok(Expr::Bound {
        kind: "map".into(),
        names,
        tuple: true,
        domain: Some(Box::new(domain)),
        body: Box::new(body),
    })
}

/// `C(p)!Op(x)` applies the instantiated `C!Op` to the instance arguments
/// followed by its own.
pub(super) fn prefixed_op(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let mut names = Vec::new();
    let mut args = Vec::new();
    for part in children(field(node, "prefix")?) {
        match children(part).as_slice() {
            [name] if name.kind() == "identifier_ref" => names.push(text(*name, source)),
            [call] if call.kind() == "bound_op" => match expression(*call, source)? {
                Expr::Call(_, instance) => {
                    names.push(text(field(*call, "name")?, source));
                    args.extend(instance);
                }
                _ => return Err(error(part, "invalid module prefix")),
            },
            _ => return Err(error(part, "unsupported module prefix")),
        }
    }
    let prefix = names.join("!");
    Ok(match get(node, "op", source)? {
        Expr::Name(name) if args.is_empty() => Expr::Name(format!("{prefix}!{name}")),
        Expr::Name(name) => Expr::Call(format!("{prefix}!{name}"), args),
        Expr::Call(name, own) => {
            args.extend(own);
            Expr::Call(format!("{prefix}!{name}"), args)
        }
        _ => return Err(error(node, "unsupported qualified operator")),
    })
}

/// `[x \\in S |-> e]`, over one or more bounds.
pub(super) fn function_literal(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let nodes = children(node);
    let bounds: Vec<_> = nodes
        .iter()
        .copied()
        .filter(|n| n.kind() == "quantifier_bound")
        .collect();
    let body = *nodes
        .last()
        .ok_or_else(|| error(node, "missing expression"))?;
    function(&bounds, expression(body, source)?, source)
}

/// `CHOOSE x \\in S : P`, or unbounded without the set.
pub(super) fn choose(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let (names, tuple) = intro(field(node, "intro")?, source);
    Ok(Expr::Bound {
        kind: "CHOOSE".into(),
        names,
        tuple,
        domain: node
            .child_by_field_name("set")
            .map(|set| expression(set, source).map(Box::new))
            .transpose()?,
        body: Box::new(get(node, "expression", source)?),
    })
}

/// `[a |-> 1, b |-> 2]`.
pub(super) fn record_literal(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let fields: Vec<_> = children(node)
        .into_iter()
        .filter(|n| n.kind() != "all_map_to")
        .collect();
    Ok(Expr::Record(
        fields
            .chunks_exact(2)
            .map(|pair| Ok((text(pair[0], source).into(), expression(pair[1], source)?)))
            .collect::<Result<_, Diagnostic>>()?,
    ))
}

/// `A \\X B \\X C` is one product of all its factors; a parenthesized
/// operand is a factor of its own.
pub(super) fn cartesian_product(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let mut factors = Vec::new();
    let mut work = vec![node];
    while let Some(next) = work.pop() {
        if is_product(next, source) {
            work.extend([field(next, "rhs")?, field(next, "lhs")?]);
        } else {
            factors.push(expression(next, source)?);
        }
    }
    Ok(Expr::Call("$CartesianProd".into(), factors))
}

/// An infix or prefix operator written as a call, `(+)(a, b)`.
pub(super) fn bound_nonfix_op(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let operands: Vec<_> = children(node)
        .into_iter()
        .filter(|n| !n.kind().ends_with("op_symbol"))
        .collect();
    Ok(Expr::Call(
        symbol(text(field(node, "symbol")?, source)).into(),
        all(&operands, source)?,
    ))
}

/// `LAMBDA x, y : e` as `$Lambda(<<x, y>>, e)`.
pub(super) fn lambda(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let nodes = children(node);
    let (body, parameters) = nodes
        .split_last()
        .ok_or_else(|| error(node, "missing expression"))?;
    let names = parameters
        .iter()
        .map(|n| Expr::Name(text(*n, source).into()))
        .collect();
    let body = expression(*body, source)?;
    Ok(Expr::Call("$Lambda".into(), vec![Expr::Tuple(names), body]))
}

/// `Op!2`, `Op(x)!1!<<` and `Op!:`: a subexpression of a definition's body,
/// as `$Subexpr(reference, selector)` nested per step. The selector is the
/// operand number, or `":"` for the whole body.
pub(super) fn subexpression(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let mut names: Vec<String> = Vec::new();
    let mut reference: Option<Expr> = None;
    let mut work = vec![node];
    let mut sequence = Vec::new();
    while let Some(next) = work.pop() {
        match next.kind() {
            "subexpression" | "subexpr_prefix" => {
                let mut kids = children(next);
                kids.reverse();
                work.extend(kids);
            }
            "subexpr_component" | "subexpr_tree_nav" => sequence.push(next),
            _ => {}
        }
    }
    for step in sequence {
        match step.kind() {
            "subexpr_component" => {
                let inner = children(step)
                    .into_iter()
                    .next()
                    .ok_or_else(|| error(step, "empty subexpression component"))?;
                match reference.take() {
                    None if inner.kind() == "identifier_ref" => {
                        names.push(text(inner, source).to_owned());
                    }
                    None => reference = Some(expression(inner, source)?),
                    Some(Expr::Name(name)) if inner.kind() == "identifier_ref" => {
                        names.push(name);
                        names.push(text(inner, source).to_owned());
                    }
                    Some(_) => return Err(error(step, "unsupported subexpression path")),
                }
            }
            "subexpr_tree_nav" => {
                let base = match reference.take() {
                    Some(base) => base,
                    None if !names.is_empty() => Expr::Name(names.join("!")),
                    None => return Err(error(step, "subexpression without an operator")),
                };
                names.clear();
                let selector = match children(step).first().map(|n| n.kind()) {
                    Some("child_id") => Expr::Integer(
                        text(children(step)[0], source)
                            .parse()
                            .map_err(|_| error(step, "invalid subexpression index"))?,
                    ),
                    Some("langle_bracket") => Expr::Integer(1),
                    Some("rangle_bracket") => Expr::Integer(2),
                    Some("colon") => Expr::String(":".into()),
                    _ => return Err(error(step, "unsupported subexpression selector")),
                };
                reference = Some(Expr::Call("$Subexpr".into(), vec![base, selector]));
            }
            _ => {}
        }
    }
    match reference {
        Some(expr) => Ok(expr),
        None if !names.is_empty() => Ok(Expr::Name(names.join("!"))),
        None => Err(error(node, "empty subexpression")),
    }
}
