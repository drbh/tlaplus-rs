mod forms;
use crate::binders::nest;
use crate::modules::{children, error, field, text};
use forms::{
    bound_nonfix_op, cartesian_product, case, choose, except, function_literal, lambda, let_in,
    prefixed_op, record_literal, set_map, unbounded_quantification,
};
use tla_syntax::{Diagnostic, Expr};
use tree_sitter::Node;

/// The expression in `node`'s field `name`.
pub(super) fn get(node: Node<'_>, name: &str, source: &str) -> Result<Expr, Diagnostic> {
    expression(field(node, name)?, source)
}

/// Every node as an expression, in order.
pub(super) fn all(nodes: &[Node<'_>], source: &str) -> Result<Vec<Expr>, Diagnostic> {
    nodes.iter().map(|n| expression(*n, source)).collect()
}

/// One key as itself, several as the tuple `f[a, b]` abbreviates.
pub(super) fn key_or_tuple(mut keys: Vec<Expr>) -> Expr {
    if keys.len() == 1 {
        keys.remove(0)
    } else {
        Expr::Tuple(keys)
    }
}

pub(crate) fn expression(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let nodes = children(node);
    Ok(match node.kind() {
        "identifier_ref" | "identifier" => Expr::Name(text(node, source).into()),
        "nat_number" | "binary_number" | "octal_number" | "hex_number" => number(node, source)?,
        "string" => string(node, source)?,
        "boolean" => Expr::Name(text(node, source).into()),
        "string_set" => Expr::Name("STRING".into()),
        "boolean_set" => Expr::Name("BOOLEAN".into()),
        "nat_number_set" => Expr::Name("Nat".into()),
        "int_number_set" => Expr::Name("Int".into()),
        "real_number_set" => Expr::Name("Real".into()),
        "infix_op_symbol" | "prefix_op_symbol" | "postfix_op_symbol" => {
            Expr::Name(symbol(text(node, source)).into())
        }
        "prev_func_val" => Expr::Name("@".into()),
        "label" => get(node, "expression", source)?,
        "parentheses" | "conj_item" | "disj_item" => expression(
            *nodes
                .last()
                .ok_or_else(|| error(node, "missing expression"))?,
            source,
        )?,
        "finite_set_literal" => Expr::Set(all(&nodes, source)?),
        "tuple_literal" => Expr::Tuple(all(&operands(&nodes), source)?),
        "record_literal" => record_literal(node, source)?,
        "record_value" => Expr::Index(
            Box::new(expression(nodes[0], source)?),
            Box::new(Expr::String(text(nodes[1], source).into())),
        ),
        "set_of_records" => Expr::Call(
            "$SetOfRcds".into(),
            nodes
                .chunks_exact(2)
                .map(|pair| {
                    let field = Expr::String(text(pair[0], source).into());
                    Ok(Expr::Tuple(vec![field, expression(pair[1], source)?]))
                })
                .collect::<Result<_, Diagnostic>>()?,
        ),
        "set_of_functions" => {
            let parts = operands(&nodes);
            Expr::Binary(
                "->".into(),
                Box::new(expression(parts[0], source)?),
                Box::new(expression(parts[1], source)?),
            )
        }
        "function_evaluation" => {
            let key = key_or_tuple(all(&nodes[1..], source)?);
            Expr::Index(Box::new(expression(nodes[0], source)?), Box::new(key))
        }
        "except" => except(node, source)?,
        "case" => case(node, source)?,
        "if_then_else" => Expr::If(
            Box::new(get(node, "if", source)?),
            Box::new(get(node, "then", source)?),
            Box::new(get(node, "else", source)?),
        ),
        "let_in" => let_in(node, source)?,
        "bounded_quantification" => {
            let kind = symbol(text(field(node, "quantifier")?, source));
            let mut cursor = node.walk();
            let bounds: Vec<_> = node
                .children_by_field_name("bound", &mut cursor)
                .filter(|n| n.kind() == "quantifier_bound")
                .collect();
            nest(&bounds, get(node, "expression", source)?, kind, source)?
        }
        "unbounded_quantification" => unbounded_quantification(node, source)?,
        "choose" => choose(node, source)?,
        "set_filter" => nest(
            &[field(node, "generator")?],
            get(node, "filter", source)?,
            "filter",
            source,
        )?,
        "set_map" => set_map(node, source)?,
        "function_literal" => function_literal(node, source)?,
        "bound_infix_op" if is_product(node, source) => cartesian_product(node, source)?,
        "bound_infix_op" => Expr::Binary(
            symbol(text(field(node, "symbol")?, source)).into(),
            Box::new(get(node, "lhs", source)?),
            Box::new(get(node, "rhs", source)?),
        ),
        "bound_prefix_op" => Expr::Unary(
            symbol(text(field(node, "symbol")?, source)).into(),
            Box::new(get(node, "rhs", source)?),
        ),
        "bound_postfix_op" => Expr::Unary(
            symbol(text(field(node, "symbol")?, source)).into(),
            Box::new(get(node, "lhs", source)?),
        ),
        "bound_op" => {
            let mut cursor = node.walk();
            let args = node
                .children_by_field_name("parameter", &mut cursor)
                .filter(|n| n.is_named())
                .map(|n| expression(n, source))
                .collect::<Result<_, _>>()?;
            Expr::Call(text(field(node, "name")?, source).into(), args)
        }
        "prefixed_op" => prefixed_op(node, source)?,
        "bound_nonfix_op" => bound_nonfix_op(node, source)?,
        "lambda" => lambda(node, source)?,
        "fairness" => {
            let strong = text(node, source).starts_with("SF");
            Expr::Call(
                if strong { "SF_" } else { "WF_" }.into(),
                all(&nodes, source)?,
            )
        }
        "step_expr_or_stutter" => Expr::Stutter(
            Box::new(expression(nodes[0], source)?),
            Box::new(expression(nodes[1], source)?),
        ),
        "step_expr_no_stutter" => Expr::Call("$AngleAct".into(), all(&operands(&nodes), source)?),
        "conj_list" | "disj_list" => {
            let conjunction = node.kind() == "conj_list";
            all(&operands(&nodes), source)?
                .into_iter()
                .reduce(|a, b| {
                    let op = if conjunction { "/\\" } else { "\\/" };
                    Expr::Binary(op.into(), Box::new(a), Box::new(b))
                })
                .ok_or_else(|| error(node, "empty conjunction"))?
        }
        kind => return Err(error(node, format!("unsupported syntax {kind}"))),
    })
}

/// Expression children, without bracket and bullet tokens.
fn operands<'a>(nodes: &[Node<'a>]) -> Vec<Node<'a>> {
    nodes
        .iter()
        .copied()
        .filter(|n| {
            !matches!(
                n.kind(),
                "langle_bracket"
                    | "rangle_bracket"
                    | "rangle_bracket_sub"
                    | "maps_to"
                    | "bullet_conj"
                    | "bullet_disj"
                    | "land"
                    | "lor"
            )
        })
        .collect()
}

/// The canonical ASCII spelling of an operator symbol.
pub(super) fn symbol(text: &str) -> &str {
    match text {
        "\\land" | "∧" => "/\\",
        "\\lor" | "∨" => "\\/",
        "\\lnot" | "\\neg" | "¬" => "~",
        "\\leq" | "=<" | "≤" => "<=",
        "\\geq" | "≥" => ">=",
        "\\equiv" | "≡" | "⇔" => "<=>",
        "⇒" => "=>",
        "/=" | "≠" => "#",
        "\\union" | "∪" => "\\cup",
        "\\intersect" | "∩" => "\\cap",
        "\\times" | "×" => "\\X",
        "\\circ" | "∘" => "\\o",
        "\\oplus" | "⊕" => "(+)",
        "\\ominus" | "⊖" => "(-)",
        "∈" => "\\in",
        "∉" => "\\notin",
        "⊆" => "\\subseteq",
        "\\forall" | "∀" => "\\A",
        "\\exists" | "∃" => "\\E",
        "□" => "[]",
        "◇" => "<>",
        "⇝" => "~>",
        other => other,
    }
}

fn number(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let digits = text(node, source);
    let (radix, digits) = match digits.get(..2) {
        Some("\\b" | "\\B") => (2, &digits[2..]),
        Some("\\o" | "\\O") => (8, &digits[2..]),
        Some("\\h" | "\\H") => (16, &digits[2..]),
        _ => (10, digits),
    };
    i64::from_str_radix(digits, radix)
        .map(Expr::Integer)
        .map_err(|_| error(node, "integer literal out of range"))
}

fn string(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let raw = text(node, source);
    let mut result = String::new();
    let mut chars = raw[1..raw.len() - 1].chars();
    while let Some(c) = chars.next() {
        result.push(if c == '\\' {
            match chars.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('r') => '\r',
                Some('f') => '\u{c}',
                Some(other) => other,
                None => return Err(error(node, "unterminated escape")),
            }
        } else {
            c
        });
    }
    Ok(Expr::String(result))
}

pub(super) fn is_product(node: Node<'_>, source: &str) -> bool {
    node.kind() == "bound_infix_op"
        && node
            .child_by_field_name("symbol")
            .is_some_and(|symbol| matches!(text(symbol, source), "\\X" | "\\times" | "×"))
}
