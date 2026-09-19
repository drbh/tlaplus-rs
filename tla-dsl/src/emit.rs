//! Export the typed language into inspectable TLA+ and TLC configuration.
mod expressions;
use crate::{Artifact, Origin, ast::*};
pub(crate) use expressions::expr;
pub fn emit(spec: &Spec) -> Artifact {
    let mut output = Artifact {
        module: spec.name.text.clone(),
        tla: format!(
            "---- MODULE {} ----\nEXTENDS Integers, Sequences, FiniteSets, TLC\n\n",
            spec.name.text
        ),
        config: if spec.library {
            String::new()
        } else {
            "SPECIFICATION Spec\nINVARIANT TypeOK\n".into()
        },
        origins: vec![],
        actions: spec.actions.iter().map(|a| a.name.text.clone()).collect(),
    };
    for enumeration in &spec.enums {
        for variant in &enumeration.variants {
            definition(
                &mut output,
                variant,
                "",
                &format!("\"{}::{}\"", enumeration.name.text, variant.text),
            );
        }
        definition(
            &mut output,
            &enumeration.name,
            "",
            &format!(
                "{{{}}}",
                enumeration
                    .variants
                    .iter()
                    .map(|v| v.text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    for &(function, index) in &spec.definition_order {
        if function {
            if !spec.functions[index].stateful {
                function_definition(&mut output, &spec.functions[index]);
            }
        } else {
            let c = &spec.constants[index];
            definition(&mut output, &c.name, "", &expr(&c.value));
            if crate::refinements::erase(&c.ty) != c.ty {
                output.tla.push_str(&format!(
                    "ASSUME {}\n",
                    expressions::type_predicate(&c.ty, &c.name.text, 0)
                ));
            }
        }
    }
    for assumption in &spec.assumptions {
        output
            .tla
            .push_str(&format!("ASSUME {}\n", expr(assumption)));
    }
    if !spec.library {
        model(&mut output, spec);
    }
    output.tla.push_str("====\n");
    output
}
fn parameters(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!("({})", names.join(", "))
    }
}
fn model(output: &mut Artifact, spec: &Spec) {
    let variables = spec
        .states
        .iter()
        .map(|s| s.name.text.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    output.tla.push_str(&format!(
        "VARIABLES {variables}\nvars == <<{variables}>>\n\n"
    ));
    for &(function, index) in &spec.definition_order {
        if function && spec.functions[index].stateful {
            function_definition(output, &spec.functions[index]);
        }
    }
    let initial = if let Some(init) = &spec.init {
        let mut terms = init.guards.iter().map(expr).collect::<Vec<_>>();
        terms.extend(
            init.updates
                .iter()
                .map(|(n, e)| format!("{} = {}", n.text, expr(e))),
        );
        quantify("\\E", &init.picks, conjunction(&terms))
    } else {
        conjunction(
            &spec
                .states
                .iter()
                .map(|s| {
                    format!(
                        "{} {} {}",
                        s.name.text,
                        if s.choice { "\\in" } else { "=" },
                        expr(&s.value)
                    )
                })
                .collect::<Vec<_>>(),
        )
    };
    output.tla.push_str(&format!("Init ==\n  {initial}\n\n"));
    let mut types = spec
        .states
        .iter()
        .map(|s| expressions::type_predicate(&s.ty, &s.name.text, 0))
        .collect::<Vec<_>>();
    types.extend(spec.refinement_checks.iter().map(expr));
    output
        .tla
        .push_str(&format!("TypeOK ==\n  {}\n\n", conjunction(&types)));
    for action in &spec.actions {
        let params = action
            .params
            .iter()
            .map(|(n, _)| n.text.clone())
            .collect::<Vec<_>>();
        definition(
            output,
            &action.name,
            &parameters(&params),
            &action_body(action, spec),
        );
    }
    let next = spec
        .actions
        .iter()
        .filter(|a| {
            spec.next
                .as_ref()
                .is_none_or(|names| names.iter().any(|n| n.text == a.name.text))
        })
        .map(|a| quantify("\\E", &a.params, action_call(a)))
        .collect::<Vec<_>>()
        .join(" \\/ ");
    output
        .tla
        .push_str(&format!("Next == {next}\n\nSpec == Init /\\ [][Next]_vars"));
    for fair in &spec.fairness {
        let actions = fair
            .actions
            .iter()
            .map(|n| spec.actions.iter().find(|a| a.name.text == n.text).unwrap())
            .collect::<Vec<_>>();
        let body = actions
            .iter()
            .map(|a| action_call(a))
            .collect::<Vec<_>>()
            .join(" \\/ ");
        let formula = format!("{}_vars({body})", if fair.strong { "SF" } else { "WF" });
        output.tla.push_str(&format!(
            "\n  /\\ {}",
            quantify("\\A", &actions[0].params, formula)
        ));
    }
    output.tla.push_str("\n\n");
    for property in &spec.properties {
        let mut body = expr(&property.body);
        if !property.frame.is_empty() {
            let vars = property
                .frame
                .iter()
                .map(|n| n.text.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            body = format!("[][{body}]_<<{vars}>>");
        }
        definition(output, &property.name, "", &body);
        output.config.push_str(&format!(
            "{} {}\n",
            if property.temporal || !property.frame.is_empty() {
                "PROPERTY"
            } else {
                "INVARIANT"
            },
            property.name.text
        ));
    }
}
fn definition(output: &mut Artifact, name: &Name, params: &str, body: &str) {
    output
        .tla
        .push_str(&format!("\\* DSL {}:{}\n", name.at.line, name.at.column));
    output.origins.push(Origin {
        name: name.text.clone(),
        generated_line: output.tla.lines().count() + 1,
        source: name.at,
    });
    output
        .tla
        .push_str(&format!("{}{params} == {body}\n\n", name.text));
}
fn action_call(action: &Action) -> String {
    format!(
        "{}{}",
        action.name.text,
        parameters(
            &action
                .params
                .iter()
                .map(|(n, _)| n.text.clone())
                .collect::<Vec<_>>()
        )
    )
}
fn quantify(quantifier: &str, bindings: &[(Name, Expr)], mut body: String) -> String {
    for (name, domain) in bindings.iter().rev() {
        body = format!(
            "({quantifier} {} \\in {} : {body})",
            name.text,
            expr(domain)
        );
    }
    body
}
fn action_body(action: &Action, spec: &Spec) -> String {
    let mut terms = action.guards.iter().map(expr).collect::<Vec<_>>();
    terms.extend(
        action
            .updates
            .iter()
            .map(|(name, value)| format!("{}' = {}", name.text, expr(value))),
    );
    let unchanged = spec
        .states
        .iter()
        .filter(|s| !action.updates.iter().any(|(n, _)| n.text == s.name.text))
        .map(|s| s.name.text.as_str())
        .collect::<Vec<_>>();
    if !unchanged.is_empty() {
        terms.push(format!("UNCHANGED <<{}>>", unchanged.join(", ")));
    }
    quantify("\\E", &action.picks, format!("({})", conjunction(&terms)))
}
fn conjunction(terms: &[String]) -> String {
    terms
        .iter()
        .map(|s| format!("({s})"))
        .collect::<Vec<_>>()
        .join(" /\\ ")
}

fn function_definition(output: &mut Artifact, function: &Function) {
    let params = function
        .params
        .iter()
        .map(|(n, t)| {
            if let Type::Function(args, _) = t {
                format!("{}({})", n.text, vec!["_"; args.len()].join(", "))
            } else {
                n.text.clone()
            }
        })
        .collect::<Vec<_>>();
    if function.recursive {
        output.tla.push_str(&format!(
            "RECURSIVE {}{}\n",
            function.name.text,
            parameters(&vec!["_".to_owned(); params.len()])
        ));
    }
    let mut checks = vec![];
    for (name, ty) in &function.params {
        if crate::refinements::erase(ty) != *ty {
            checks.push(expressions::type_predicate(ty, &name.text, 0));
        }
    }
    let result_refined = crate::refinements::erase(&function.result) != function.result;
    let body = if function.name.at.line == 0 || (checks.is_empty() && !result_refined) {
        expr(&function.body)
    } else {
        let pre = if checks.is_empty() {
            "TRUE".into()
        } else {
            conjunction(&checks)
        };
        let post = if result_refined {
            expressions::type_predicate(&function.result, "__tsl_result", 0)
        } else {
            "TRUE".into()
        };
        format!(
            "LET __tsl_result == {} IN IF Assert({pre}, \"precondition of {}\") /\\ Assert({post}, \"postcondition of {}\") THEN __tsl_result ELSE __tsl_result",
            expr(&function.body),
            function.name.text,
            function.name.text
        )
    };
    definition(output, &function.name, &parameters(&params), &body);
}
