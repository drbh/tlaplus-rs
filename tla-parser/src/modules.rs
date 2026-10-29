use crate::{expressions::expression, parse};
use tlaplus_syntax::{Definition, Diagnostic, Expr, Module};
use tree_sitter::Node;

pub fn module(source: &str) -> Result<Module, Diagnostic> {
    let tree = parse(source)?;
    let root = tree.root_node();
    // The first module is the file's; like SANY, later ones are available to it.
    let mut cursor = root.walk();
    let mut modules = root
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "module");
    let first = modules
        .next()
        .ok_or_else(|| error(root, "expected a module"))?;
    let mut result = module_node(first, source)?;
    for other in modules {
        result.modules.push(module_node(other, source)?);
    }
    Ok(result)
}

fn module_node(node: Node<'_>, source: &str) -> Result<Module, Diagnostic> {
    let mut result = Module {
        name: text(field(node, "name")?, source).to_owned(),
        ..Module::default()
    };
    let mut recursive = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "recursive_declaration" => recursive.extend(declared(child, source)),
            "module" => result.modules.push(module_node(child, source)?),
            "local_definition" => {
                let inner = children(child)[0];
                match inner.kind() {
                    "operator_definition" | "function_definition" => {
                        let definition = definition(inner, source)?;
                        result.local_names.push(definition.name.clone());
                        add(&mut result, definition, child)?;
                    }
                    "instance" => {
                        let mut instance = instance(inner, None, Vec::new(), source)?;
                        instance.local = true;
                        result.instances.push(instance);
                    }
                    "module_definition" => {
                        let mut instance = module_definition(inner, source)?;
                        instance.local = true;
                        result.instances.push(instance);
                    }
                    _ => return Err(error(child, "unsupported local declaration")),
                }
            }
            "instance" => result
                .instances
                .push(instance(child, None, Vec::new(), source)?),
            "module_definition" => result.instances.push(module_definition(child, source)?),
            "identifier" | "header_line" | "single_line" | "double_line" | "comment"
            | "block_comment" | "use_or_hide" => {}
            "extends" => result.extends.extend(
                children(child)
                    .into_iter()
                    .map(|n| text(n, source).to_owned()),
            ),
            "variable_declaration" | "constant_declaration" => {
                let mut operators = Vec::new();
                let values = if child.kind() == "variable_declaration" {
                    &mut result.variables
                } else {
                    &mut result.constants
                };
                for name in children(child) {
                    if name.kind() == "operator_declaration"
                        && child.kind() == "constant_declaration"
                    {
                        // An operator constant is defined by the configuration (`F <- G`).
                        let arity = children(name)
                            .into_iter()
                            .filter(|n| n.kind() == "placeholder")
                            .count();
                        let operator = text(field(name, "name")?, source).to_owned();
                        let message = format!("operator constant {operator} is not configured");
                        let definition = Definition {
                            name: operator,
                            parameters: (0..arity).map(|i| format!("$arg{i}")).collect(),
                            arities: vec![0; arity],
                            body: Expr::Call("$Error".into(), vec![Expr::String(message)]),
                            ..Definition::default()
                        };
                        operators.push(definition);
                        continue;
                    }
                    if name.kind() != "identifier" {
                        return Err(error(name, "unsupported declaration"));
                    }
                    let name = text(name, source).to_owned();
                    if values.contains(&name) {
                        return Err(error(child, "duplicate declaration"));
                    }
                    values.push(name.clone());
                    result.declared_at.push((result.definitions.len(), name));
                }
                for definition in operators {
                    add(&mut result, definition, child)?;
                }
            }
            "operator_definition" | "function_definition" => {
                add(&mut result, definition(child, source)?, child)?;
            }
            "assumption" | "theorem" => {
                let statement = child.child_by_field_name("statement");
                let body = statement.or_else(|| {
                    children(child)
                        .into_iter()
                        .rev()
                        .find(|n| !matches!(n.kind(), "def_eq" | "identifier"))
                });
                let body = body.ok_or_else(|| error(child, "empty assumption"))?;
                // A theorem's proof is not checked; a named one still defines its formula.
                let formula = match expression(body, source) {
                    Ok(formula) => formula,
                    Err(_) if child.kind() == "theorem" => continue,
                    Err(diagnostic) => return Err(diagnostic),
                };
                if let Some(name) = child.child_by_field_name("name").or_else(|| {
                    children(child)
                        .into_iter()
                        .find(|n| n.kind() == "identifier")
                }) {
                    let definition = Definition {
                        name: text(name, source).into(),
                        body: formula.clone(),
                        ..Definition::default()
                    };
                    add(&mut result, definition, child)?;
                }
                if child.kind() == "assumption" {
                    result.assumptions.push(formula);
                }
            }
            _ => {
                return Err(error(
                    child,
                    format!("unsupported module construct {}", child.kind()),
                ));
            }
        }
    }
    mark_recursive(&mut result.definitions, &recursive);
    Ok(result)
}

fn add(module: &mut Module, definition: Definition, node: Node<'_>) -> Result<(), Diagnostic> {
    if module.definitions.iter().any(|d| d.name == definition.name) {
        return Err(error(node, "duplicate operator"));
    }
    module.definitions.push(definition);
    Ok(())
}

/// Names introduced by a `RECURSIVE` declaration.
pub(crate) fn declared(node: Node<'_>, source: &str) -> Vec<String> {
    children(node)
        .into_iter()
        .map(|n| match n.child_by_field_name("name") {
            Some(name) => text(name, source).to_owned(),
            None => text(n, source).to_owned(),
        })
        .collect()
}

pub(crate) fn mark_recursive(definitions: &mut [Definition], names: &[String]) {
    for definition in definitions {
        definition.recursive |= names.contains(&definition.name);
    }
}

fn module_definition(node: Node<'_>, source: &str) -> Result<tlaplus_syntax::Instance, Diagnostic> {
    let mut cursor = node.walk();
    let parameters = node
        .children_by_field_name("parameter", &mut cursor)
        .filter(|n| n.is_named())
        .map(|n| {
            if n.kind() != "identifier" {
                return Err(error(n, "operator parameters of instances are unsupported"));
            }
            Ok(text(n, source).to_owned())
        })
        .collect::<Result<_, _>>()?;
    instance(
        field(node, "definition")?,
        Some(text(field(node, "name")?, source).into()),
        parameters,
        source,
    )
}

fn instance(
    node: Node<'_>,
    name: Option<String>,
    parameters: Vec<String>,
    source: &str,
) -> Result<tlaplus_syntax::Instance, Diagnostic> {
    let nodes = children(node);
    let module = nodes
        .first()
        .ok_or_else(|| error(node, "missing instance module"))?;
    let mut substitutions = Vec::new();
    for substitution in nodes.iter().skip(1) {
        let parts = children(*substitution);
        let name = text(parts[0], source).to_owned();
        if substitutions.iter().any(|(existing, _)| existing == &name) {
            return Err(error(node, "duplicate substitution"));
        }
        substitutions.push((name, expression(*parts.last().unwrap(), source)?));
    }
    Ok(tlaplus_syntax::Instance {
        local: false,
        name,
        parameters,
        module: text(*module, source).into(),
        substitutions,
    })
}

/// An operator or function definition. Unsupported syntax in its body is reported
/// only if the definition is used.
pub(crate) fn definition(node: Node<'_>, source: &str) -> Result<Definition, Diagnostic> {
    let name = text(field(node, "name")?, source).to_owned();
    if node.kind() == "function_definition" {
        let bounds: Vec<_> = children(node)
            .into_iter()
            .filter(|n| n.kind() == "quantifier_bound")
            .collect();
        let body = expression(field(node, "definition")?, source).and_then(|body| {
            let mut function = crate::binders::function(&bounds, body, source)?;
            if let Expr::Bound { kind, .. } = &mut function {
                *kind = "recursive function".into();
            }
            Ok(function)
        });
        return Ok(Definition {
            name,
            body: body.unwrap_or_else(|diagnostic| deferred(&diagnostic)),
            recursive: true,
            ..Definition::default()
        });
    }
    let mut cursor = node.walk();
    let mut parameters = Vec::new();
    let mut arities = Vec::new();
    for parameter in node
        .children_by_field_name("parameter", &mut cursor)
        .filter(|n| n.is_named())
    {
        let arity = if parameter.kind() == "operator_declaration" {
            children(parameter)
                .into_iter()
                .filter(|n| n.kind() == "placeholder")
                .count()
        } else {
            0
        };
        let name = parameter.child_by_field_name("name").unwrap_or(parameter);
        parameters.push(symbol_name(text(name, source)));
        arities.push(arity);
    }
    let body = expression(field(node, "definition")?, source);
    Ok(Definition {
        name: symbol_name(&name),
        parameters,
        arities,
        body: body.unwrap_or_else(|diagnostic| deferred(&diagnostic)),
        ..Definition::default()
    })
}

fn symbol_name(name: &str) -> String {
    match name {
        "\\oplus" | "⊕" => "(+)".into(),
        "\\ominus" | "⊖" => "(-)".into(),
        other => other.into(),
    }
}

fn deferred(diagnostic: &Diagnostic) -> Expr {
    Expr::Call("$Error".into(), vec![Expr::String(diagnostic.to_string())])
}

pub(crate) fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
pub(crate) fn field<'a>(node: Node<'a>, name: &str) -> Result<Node<'a>, Diagnostic> {
    node.child_by_field_name(name)
        .ok_or_else(|| error(node, format!("missing syntax field {name}")))
}
pub(crate) fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|n| !matches!(n.kind(), "comment" | "block_comment"))
        .collect()
}
pub(crate) fn error(node: Node<'_>, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        offset: node.start_byte(),
        message: message.into(),
    }
}
