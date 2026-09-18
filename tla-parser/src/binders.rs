use crate::expressions::expression;
use crate::modules::{children, error, field, text};
use tla_syntax::{Diagnostic, Expr};
use tree_sitter::Node;

/// A binder's names and whether they destructure a tuple.
pub(crate) fn intro(node: Node<'_>, source: &str) -> (Vec<String>, bool) {
    if node.kind() == "tuple_of_identifiers" {
        let names = children(node)
            .into_iter()
            .filter(|n| n.kind() == "identifier")
            .map(|n| text(n, source).to_owned())
            .collect();
        return (names, true);
    }
    (vec![text(node, source).to_owned()], false)
}

/// The names of a `quantifier_bound`: `x, y \in S` binds two names, `<<a, b>> \in S` one tuple.
pub(crate) fn names_of(bound: Node<'_>, source: &str) -> (Vec<String>, bool) {
    let mut cursor = bound.walk();
    let intros: Vec<_> = bound
        .children_by_field_name("intro", &mut cursor)
        .filter(|n| n.is_named())
        .collect();
    if let [single] = intros[..] {
        return intro(single, source);
    }
    let names = intros.into_iter().map(|n| text(n, source).to_owned());
    (names.collect(), false)
}

/// Nested binders, innermost last, one per name or tuple.
pub(crate) fn nest(
    bounds: &[Node<'_>],
    mut body: Expr,
    kind: &str,
    source: &str,
) -> Result<Expr, Diagnostic> {
    for bound in bounds.iter().rev() {
        let domain = expression(field(*bound, "set")?, source)?;
        let (names, tuple) = names_of(*bound, source);
        let groups = if tuple {
            vec![names]
        } else {
            names.into_iter().map(|n| vec![n]).collect()
        };
        for names in groups.into_iter().rev() {
            body = Expr::Bound {
                kind: kind.into(),
                names,
                tuple,
                domain: Some(Box::new(domain.clone())),
                body: Box::new(body),
            };
        }
    }
    Ok(body)
}

/// Names and domain of several binders as one tuple over their product.
pub(crate) fn product(
    bounds: &[Node<'_>],
    source: &str,
) -> Result<(Vec<String>, Expr), Diagnostic> {
    let mut names = Vec::new();
    let mut factors = Vec::new();
    for bound in bounds {
        let domain = expression(field(*bound, "set")?, source)?;
        let (bound_names, tuple) = names_of(*bound, source);
        if tuple {
            return Err(error(*bound, "tuple binders in products are unsupported"));
        }
        for name in bound_names {
            names.push(name);
            factors.push(domain.clone());
        }
    }
    Ok((names, Expr::Call("$CartesianProd".into(), factors)))
}

/// `[x \in S |-> e]`; several arguments make a function over their product.
pub(crate) fn function(bounds: &[Node<'_>], body: Expr, source: &str) -> Result<Expr, Diagnostic> {
    if let [bound] = bounds {
        let (names, tuple) = names_of(*bound, source);
        if tuple || names.len() == 1 {
            return nest(bounds, body, "function", source);
        }
    }
    let (names, domain) = product(bounds, source)?;
    Ok(Expr::Bound {
        kind: "function".into(),
        names,
        tuple: true,
        domain: Some(Box::new(domain)),
        body: Box::new(body),
    })
}
