//! Parsing with the tree-sitter TLA+ grammar, converted into the trees of
//! `tla-syntax`. [`parse`] checks a source text; [`module`] builds a module.
mod binders;
mod expressions;
mod modules;
pub use modules::module;
use tla_syntax::Diagnostic;

/// SANY reads a PlusCal algorithm as the comment it is; the grammar would parse it.
fn blank_algorithms(source: &str) -> std::borrow::Cow<'_, str> {
    let mut bytes = source.as_bytes().to_vec();
    let mut changed = false;
    let mut at = 0;
    while let Some(start) = source[at..].find("(*").map(|i| i + at) {
        let rest = source[start + 2..].trim_start();
        if !(rest.starts_with("--algorithm") || rest.starts_with("--fair")) {
            at = start + 2;
            continue;
        }
        let (mut depth, mut end) = (1, start + 2);
        while depth > 0 && end + 1 < bytes.len() {
            match &source.as_bytes()[end..end + 2] {
                b"(*" => (depth, end) = (depth + 1, end + 2),
                b"*)" => (depth, end) = (depth - 1, end + 2),
                _ => end += 1,
            }
        }
        for byte in &mut bytes[start + 2..end.saturating_sub(2)] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        changed = true;
        at = end;
    }
    if !changed {
        return source.into();
    }
    // Every replaced byte, including those of multi-byte characters, is a space.
    String::from_utf8(bytes)
        .expect("blanking with spaces keeps UTF-8")
        .into()
}

pub fn parse(source: &str) -> Result<tree_sitter::Tree, Diagnostic> {
    let blanked = blank_algorithms(source);
    let source = blanked.as_ref();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_tlaplus::LANGUAGE.into())
        .map_err(|error| Diagnostic {
            offset: 0,
            message: error.to_string(),
        })?;
    let mut tree = parser.parse(source, None).ok_or_else(|| Diagnostic {
        offset: 0,
        message: "parser interrupted".into(),
    })?;
    if tree.root_node().has_error()
        && let Some(normalized) = normalize_case_separators(tree.root_node(), source)
        && let Some(reparsed) = parser.parse(&normalized, None)
        && !reparsed.root_node().has_error()
    {
        tree = reparsed;
    }
    if tree.root_node().has_error() {
        let mut work = vec![tree.root_node()];
        while let Some(node) = work.pop() {
            if node.is_error() || node.is_missing() {
                return Err(Diagnostic {
                    offset: node.start_byte(),
                    message: format!("invalid syntax: {}", node.kind()),
                });
            }
            let mut cursor = node.walk();
            work.extend(node.children(&mut cursor).filter(|node| node.has_error()));
        }
        return Err(Diagnostic {
            offset: 0,
            message: "invalid syntax".into(),
        });
    }
    validate(tree.root_node(), source)?;
    Ok(tree)
}

fn normalize_case_separators(root: tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let mut changed = false;
    let mut work = vec![root];
    while let Some(node) = work.pop() {
        work.extend(modules::children(node));
        if node.kind() != "always" || modules::text(node, source) != "[]" {
            continue;
        }
        let line_start = source[..node.start_byte()]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        let line_end = source[node.end_byte()..]
            .find('\n')
            .map_or(source.len(), |at| node.end_byte() + at);
        if !source[line_start..node.start_byte()]
            .bytes()
            .all(|b| b == b' ')
            || !source[node.end_byte()..line_end].contains("->")
        {
            continue;
        }
        let mut parent = node.parent();
        let mut column = None;
        let mut inside_case = false;
        while let Some(ancestor) = parent {
            if ancestor.kind() == "conj_item" || ancestor.kind() == "disj_item" {
                column.get_or_insert(ancestor.start_position().column);
            }
            if ancestor.kind() == "case" {
                inside_case = true;
                break;
            }
            parent = ancestor.parent();
        }
        let Some(column) = column else {
            continue;
        };
        let target = line_start + column;
        if inside_case && target + 2 <= node.start_byte() {
            bytes[node.start_byte()..node.end_byte()].fill(b' ');
            bytes[target..target + 2].copy_from_slice(b"[]");
            changed = true;
        }
    }
    changed.then(|| String::from_utf8(bytes).unwrap())
}

fn validate(root: tree_sitter::Node<'_>, source: &str) -> Result<(), Diagnostic> {
    use modules::{children, error, text};
    let mut work = vec![root];
    while let Some(node) = work.pop() {
        let symbol = node.child_by_field_name("symbol");
        let incompatible = match node.kind() {
            "string" | "string_literal" => tla_syntax::expression(text(node, source)).is_err(),
            "infix_op_symbol" => {
                matches!(text(node, source), "\\X" | "\\times")
                    && node
                        .parent()
                        .is_some_and(|parent| parent.kind() == "bound_op")
            }
            "bound_nonfix_op" => {
                symbol.is_some_and(|n| text(n, source) == "-")
                    && node
                        .parent()
                        .is_none_or(|parent| parent.kind() != "prefixed_op")
            }
            "excl" => {
                text(node, source) == "!!"
                    && node.start_byte() > 0
                    && source.as_bytes()[node.start_byte() - 1] == b'!'
            }
            "bound_infix_op" => {
                let right = node.child_by_field_name("rhs");
                symbol.is_some_and(|n| {
                    matches!(
                        text(n, source),
                        "&" | "&&" | "*" | "**" | "/" | "//" | "^" | "^^"
                    )
                }) && right.is_some_and(|n| {
                    n.kind() == "bound_prefix_op"
                        && n.child_by_field_name("symbol")
                            .is_some_and(|n| text(n, source) == "-")
                })
            }
            "step_expr_or_stutter" => children(node).first().is_some_and(|action| {
                action.kind() == "bound_infix_op"
                    && action
                        .child_by_field_name("symbol")
                        .is_some_and(|n| matches!(text(n, source), "\\in" | "∈"))
                    && action
                        .child_by_field_name("lhs")
                        .is_some_and(|n| n.kind() == "identifier_ref")
            }),
            _ => false,
        };
        if incompatible {
            return Err(error(node, "syntax is not accepted by SANY"));
        }
        work.extend(children(node));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_tree_supports_proofs_and_unicode() {
        assert!(parse("---- MODULE Test ----\nTHEOREM TRUE\nOBVIOUS\n====").is_ok());
        assert!(parse("---- MODULE Test ----\nA == ∀ x ∈ {1,2} : x > 0\n====").is_ok());
        assert!(parse("---- MODULE Test ----\nA == (\n====").is_err());
    }

    #[test]
    fn cst_lowering_preserves_scopes() {
        let module = module("---- MODULE Test ----\nF == LET A == 1 IN A + 2\n====").unwrap();
        assert!(matches!(
            module.definitions[0].body,
            tla_syntax::Expr::Let(_, _)
        ));
        assert!(module.assumptions.is_empty());
    }

    #[test]
    fn sany_grammar_restrictions() {
        for expression in [
            "f(\\X, \\times)",
            "- (1,2)",
            "A!B!!!(x,y)",
            "A & -B",
            "[][x \\in {1,2}]_x",
            "\"\\*\"",
        ] {
            let source = format!("---- MODULE Test ----\nop == {expression}\n====");
            assert!(parse(&source).is_err(), "{expression}");
        }
        assert!(parse("---- MODULE Test ----\nop == A & (-B)\n====").is_ok());
    }

    #[test]
    fn nested_case_separators_preserve_expression_positions() {
        let source = "---- MODULE Test ----\nop ==\n  /\\ CASE 1 ->\n      /\\ 2\n      /\\ 3\n        [] 4 -> 5\n  /\\ 6\n====";
        let tree = parse(source).unwrap();
        assert!(!tree.root_node().has_error());
        let module = module(source).unwrap();
        assert!(matches!(
            module.definitions[0].body,
            tla_syntax::Expr::Binary(_, _, _)
        ));
        assert!(parse(&source.replace("4 -> 5", "4 5")).is_err());
        assert!(parse(&source.replace("[] 4 -> 5", "[] -> 5")).is_err());
    }
}
