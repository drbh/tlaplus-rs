use crate::binders::{function, intro, names_of, nest, product};
use crate::modules::{children, error, field, text};
use tla_syntax::{Diagnostic, Expr};
use tree_sitter::Node;

pub(crate) fn expression(node: Node<'_>, source: &str) -> Result<Expr, Diagnostic> {
    let get = |name| expression(field(node, name)?, source);
    let nodes = children(node);
    let all = |nodes: &[Node<'_>]| {
        nodes
            .iter()
            .map(|n| expression(*n, source))
            .collect::<Result<Vec<_>, _>>()
    };
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
        "label" => get("expression")?,
        "parentheses" | "conj_item" | "disj_item" => expression(
            *nodes
                .last()
                .ok_or_else(|| error(node, "missing expression"))?,
            source,
        )?,
        "finite_set_literal" => Expr::Set(all(&nodes)?),
        "tuple_literal" => Expr::Tuple(all(&operands(&nodes))?),
        "record_literal" => {
            let nodes: Vec<_> = nodes
                .into_iter()
                .filter(|n| n.kind() != "all_map_to")
                .collect();
            Expr::Record(
                nodes
                    .chunks_exact(2)
                    .map(|pair| Ok((text(pair[0], source).into(), expression(pair[1], source)?)))
                    .collect::<Result<_, Diagnostic>>()?,
            )
        }
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
            let mut keys = all(&nodes[1..])?;
            let key = if keys.len() == 1 {
                keys.remove(0)
            } else {
                Expr::Tuple(keys)
            };
            Expr::Index(Box::new(expression(nodes[0], source)?), Box::new(key))
        }
        "except" => {
            let mut args = vec![get("expr_to_update")?];
            for update in nodes.into_iter().filter(|n| n.kind() == "except_update") {
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
                        let mut keys = all(&parts)?;
                        if keys.len() == 1 {
                            keys.remove(0)
                        } else {
                            Expr::Tuple(keys)
                        }
                    });
                }
                args.push(Expr::Tuple(vec![
                    Expr::Tuple(path),
                    expression(field(update, "new_val")?, source)?,
                ]));
            }
            Expr::Call("$Except".into(), args)
        }
        "case" => {
            let mut arms = Vec::new();
            for arm in nodes
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
                arms.push(Expr::Tuple(vec![
                    condition,
                    expression(*parts.last().unwrap(), source)?,
                ]));
            }
            Expr::Call("$Case".into(), arms)
        }
        "if_then_else" => Expr::If(
            Box::new(get("if")?),
            Box::new(get("then")?),
            Box::new(get("else")?),
        ),
        "let_in" => {
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
            Expr::Let(definitions, Box::new(get("expression")?))
        }
        "bounded_quantification" => {
            let kind = symbol(text(field(node, "quantifier")?, source));
            let mut cursor = node.walk();
            let bounds: Vec<_> = node
                .children_by_field_name("bound", &mut cursor)
                .filter(|n| n.kind() == "quantifier_bound")
                .collect();
            nest(&bounds, get("expression")?, kind, source)?
        }
        "unbounded_quantification" => {
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
            names
                .into_iter()
                .rev()
                .fold(get("expression")?, |body, name| Expr::Bound {
                    kind: kind.into(),
                    names: vec![name],
                    tuple: false,
                    domain: None,
                    body: Box::new(body),
                })
        }
        "choose" => {
            let (names, tuple) = intro(field(node, "intro")?, source);
            Expr::Bound {
                kind: "CHOOSE".into(),
                names,
                tuple,
                domain: node
                    .child_by_field_name("set")
                    .map(|set| expression(set, source).map(Box::new))
                    .transpose()?,
                body: Box::new(get("expression")?),
            }
        }
        "set_filter" => nest(
            &[field(node, "generator")?],
            get("filter")?,
            "filter",
            source,
        )?,
        "set_map" => {
            let bounds: Vec<_> = nodes
                .iter()
                .copied()
                .filter(|n| n.kind() == "quantifier_bound")
                .collect();
            let body = get("map")?;
            if bounds.len() == 1
                && (names_of(bounds[0], source).1 || names_of(bounds[0], source).0.len() == 1)
            {
                nest(&bounds, body, "map", source)?
            } else {
                let (names, domain) = product(&bounds, source)?;
                Expr::Bound {
                    kind: "map".into(),
                    names,
                    tuple: true,
                    domain: Some(Box::new(domain)),
                    body: Box::new(body),
                }
            }
        }
        "function_literal" => {
            let bounds: Vec<_> = nodes
                .iter()
                .copied()
                .filter(|n| n.kind() == "quantifier_bound")
                .collect();
            function(&bounds, expression(*nodes.last().unwrap(), source)?, source)?
        }
        "bound_infix_op" if is_product(node, source) => {
            // `A \X B \X C` is one ternary product; parenthesized operands are not.
            let mut factors = Vec::new();
            let mut work = vec![node];
            while let Some(next) = work.pop() {
                if is_product(next, source) {
                    work.extend([field(next, "rhs")?, field(next, "lhs")?]);
                } else {
                    factors.push(expression(next, source)?);
                }
            }
            Expr::Call("$CartesianProd".into(), factors)
        }
        "bound_infix_op" => Expr::Binary(
            symbol(text(field(node, "symbol")?, source)).into(),
            Box::new(get("lhs")?),
            Box::new(get("rhs")?),
        ),
        "bound_prefix_op" => Expr::Unary(
            symbol(text(field(node, "symbol")?, source)).into(),
            Box::new(get("rhs")?),
        ),
        "bound_postfix_op" => Expr::Unary(
            symbol(text(field(node, "symbol")?, source)).into(),
            Box::new(get("lhs")?),
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
        "prefixed_op" => {
            // `C(p)!Op(x)` applies the instantiated `C!Op` to the instance
            // arguments followed by its own.
            let mut names = Vec::new();
            let mut args = Vec::new();
            for part in children(field(node, "prefix")?) {
                match children(part).as_slice() {
                    [name] if name.kind() == "identifier_ref" => names.push(text(*name, source)),
                    [call] if call.kind() == "bound_op" => match expression(*call, source)? {
                        Expr::Call(name, instance) => {
                            names.push(text(field(*call, "name")?, source));
                            debug_assert!(!name.is_empty());
                            args.extend(instance);
                        }
                        _ => return Err(error(part, "invalid module prefix")),
                    },
                    _ => return Err(error(part, "unsupported module prefix")),
                }
            }
            let prefix = names.join("!");
            match get("op")? {
                Expr::Name(name) if args.is_empty() => Expr::Name(format!("{prefix}!{name}")),
                Expr::Name(name) => Expr::Call(format!("{prefix}!{name}"), args),
                Expr::Call(name, own) => {
                    args.extend(own);
                    Expr::Call(format!("{prefix}!{name}"), args)
                }
                _ => return Err(error(node, "unsupported qualified operator")),
            }
        }
        "bound_nonfix_op" => {
            let operands: Vec<_> = nodes
                .iter()
                .copied()
                .filter(|n| !n.kind().ends_with("op_symbol"))
                .collect();
            Expr::Call(
                symbol(text(field(node, "symbol")?, source)).into(),
                all(&operands)?,
            )
        }
        "lambda" => {
            let names = nodes[..nodes.len() - 1]
                .iter()
                .map(|n| Expr::Name(text(*n, source).into()))
                .collect();
            let body = expression(*nodes.last().unwrap(), source)?;
            Expr::Call("$Lambda".into(), vec![Expr::Tuple(names), body])
        }
        "fairness" => {
            let strong = text(node, source).starts_with("SF");
            Expr::Call(if strong { "SF_" } else { "WF_" }.into(), all(&nodes)?)
        }
        "step_expr_or_stutter" => Expr::Stutter(
            Box::new(expression(nodes[0], source)?),
            Box::new(expression(nodes[1], source)?),
        ),
        "step_expr_no_stutter" => Expr::Call("$AngleAct".into(), all(&operands(&nodes))?),
        "conj_list" | "disj_list" => {
            let conjunction = node.kind() == "conj_list";
            all(&operands(&nodes))?
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
fn symbol(text: &str) -> &str {
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

fn is_product(node: Node<'_>, source: &str) -> bool {
    node.kind() == "bound_infix_op"
        && node
            .child_by_field_name("symbol")
            .is_some_and(|symbol| matches!(text(symbol, source), "\\X" | "\\times" | "×"))
}
