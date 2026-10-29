use crate::Error;
use tlaplus_syntax::{Expr, Module};

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub init: Option<String>,
    pub next: Option<String>,
    pub specification: Option<String>,
    pub invariants: Vec<String>,
    pub properties: Vec<String>,
    pub constraints: Vec<String>,
    pub action_constraints: Vec<String>,
    pub constants: Vec<(String, Expr)>,
    pub overrides: Vec<(String, String)>,
    /// `F <-[M] G`: `F` is replaced by `G` only inside instances of module `M`.
    pub scoped_overrides: Vec<(String, String, String)>,
    pub view: Option<String>,
    pub symmetry: Option<String>,
    /// Only changes how states are displayed, which is outside the answer contract.
    pub alias: Option<String>,
    pub postconditions: Vec<String>,
    /// `_POSSIBLE`: predicates some reachable state or step must satisfy.
    pub possible: Vec<String>,
    pub check_deadlock: Option<bool>,
}

pub fn config(source: &str) -> Result<Config, Error> {
    let tokens = tlaplus_syntax::lex(source).map_err(|e| Error(e.to_string()))?;
    let mut result = Config::default();
    let mut at = 0;
    while at < tokens.len() {
        let section = tokens[at].text.as_str();
        at += 1;
        let start = at;
        while at < tokens.len() && !cfg_keyword(&tokens[at].text) {
            at += 1;
        }
        let values = &tokens[start..at];
        let one = || -> Result<String, Error> {
            if values.len() != 1 {
                return Err(Error(format!("{section} requires one identifier")));
            }
            Ok(values[0].text.clone())
        };
        match section {
            "INIT" => assign(&mut result.init, one()?, section)?,
            "NEXT" => assign(&mut result.next, one()?, section)?,
            "SPECIFICATION" => assign(&mut result.specification, one()?, section)?,
            "VIEW" => assign(&mut result.view, one()?, section)?,
            "SYMMETRY" => assign(&mut result.symmetry, one()?, section)?,
            "ALIAS" => assign(&mut result.alias, one()?, section)?,
            "_POSSIBLE" => result
                .possible
                .extend(values.iter().map(|v| v.text.clone())),
            "POSTCONDITION" | "POSTCONDITIONS" => result
                .postconditions
                .extend(values.iter().map(|v| v.text.clone())),
            "CHECK_DEADLOCK" => {
                let value = match one()?.as_str() {
                    "TRUE" => true,
                    "FALSE" => false,
                    _ => return Err(Error("CHECK_DEADLOCK requires TRUE or FALSE".into())),
                };
                if result.check_deadlock.replace(value).is_some() {
                    return Err(Error("duplicate CHECK_DEADLOCK".into()));
                }
            }
            "INVARIANT" | "INVARIANTS" => result
                .invariants
                .extend(values.iter().map(|v| v.text.clone())),
            "PROPERTY" | "PROPERTIES" => result
                .properties
                .extend(values.iter().map(|v| v.text.clone())),
            "CONSTRAINT" | "CONSTRAINTS" => result
                .constraints
                .extend(values.iter().map(|v| v.text.clone())),
            "ACTION_CONSTRAINT" | "ACTION_CONSTRAINTS" => result
                .action_constraints
                .extend(values.iter().map(|v| v.text.clone())),
            "CONSTANT" | "CONSTANTS" => {
                let mut index = 0;
                while index < values.len() {
                    let name = values[index].text.clone();
                    index += 1;
                    let scoped = values.get(index).is_some_and(|v| v.text == "<")
                        && values.get(index + 2).is_some_and(|v| v.text == "[");
                    if !scoped
                        && (result.constants.iter().any(|(n, _)| *n == name)
                            || result.overrides.iter().any(|(n, _)| *n == name))
                    {
                        return Err(Error(format!("duplicate constant binding {name}")));
                    }
                    if values.get(index).is_some_and(|v| v.text == "<")
                        && values.get(index + 1).is_some_and(|v| v.text == "-")
                    {
                        index += 2;
                        let mut scope = None;
                        if values.get(index).is_some_and(|v| v.text == "[") {
                            let module = values
                                .get(index + 1)
                                .ok_or_else(|| Error("missing override module".into()))?
                                .text
                                .clone();
                            if values.get(index + 2).is_none_or(|v| v.text != "]") {
                                return Err(Error("override module requires [Module]".into()));
                            }
                            index += 3;
                            scope = Some(module);
                        }
                        let replacement = values
                            .get(index)
                            .ok_or_else(|| Error("missing override".into()))?
                            .text
                            .clone();
                        index += 1;
                        match scope {
                            Some(module) => {
                                result.scoped_overrides.push((module, name, replacement))
                            }
                            None => result.overrides.push((name, replacement)),
                        }
                        continue;
                    }
                    if values.get(index).is_none_or(|v| v.text != "=") {
                        return Err(Error("constant binding requires = or <-".into()));
                    }
                    index += 1;
                    let begin = index;
                    let mut depth = 0i32;
                    while index < values.len() {
                        if depth == 0
                            && index > begin
                            && values
                                .get(index + 1)
                                .is_some_and(|v| v.text == "=" || v.text == "<")
                        {
                            break;
                        }
                        match values[index].text.as_str() {
                            "{" | "(" | "[" | "<<" => depth += 1,
                            "}" | ")" | "]" | ">>" => depth -= 1,
                            _ => {}
                        }
                        index += 1;
                    }
                    let text = values[begin..index]
                        .iter()
                        .map(|v| v.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    result.constants.push((
                        name,
                        tlaplus_syntax::expression(&text).map_err(|e| Error(e.to_string()))?,
                    ));
                }
            }
            _ => {
                return Err(Error(format!(
                    "unsupported configuration directive {section}"
                )));
            }
        }
    }
    if result.specification.is_some() && (result.init.is_some() || result.next.is_some()) {
        return Err(Error(
            "SPECIFICATION cannot be combined with INIT or NEXT".into(),
        ));
    }
    Ok(result)
}

/// Applies configuration overrides of operators (`F <- G`, for operator constants
/// and definitions alike): the operator's body becomes an application of `G`.
/// A scoped `F <-[M] G` applies to `F` as instantiated from module `M`.
pub fn override_operators(module: &mut Module, config: &mut Config) -> Result<(), Error> {
    let mut remaining = Vec::new();
    for (name, replacement) in std::mem::take(&mut config.overrides) {
        if module.constants.contains(&name) {
            remaining.push((name, replacement));
            continue;
        }
        override_definition(module, &name, replacement.clone())?;
    }
    config.overrides = remaining;
    for (scope, name, replacement) in std::mem::take(&mut config.scoped_overrides) {
        let instances: Vec<_> = module
            .instance_origins
            .iter()
            .filter(|(_, origin)| *origin == scope)
            .map(|(prefix, _)| match prefix {
                Some(prefix) => format!("{prefix}!{name}"),
                None => name.clone(),
            })
            .collect();
        if instances.is_empty() {
            return Err(Error(format!(
                "no instance of module {scope} to override {name} in"
            )));
        }
        for instantiated in instances {
            override_definition(module, &instantiated, replacement.clone())?;
        }
    }
    Ok(())
}

fn override_definition(module: &mut Module, name: &str, replacement: String) -> Result<(), Error> {
    let definition = module
        .definitions
        .iter_mut()
        .find(|d| d.name == name)
        .ok_or_else(|| Error(format!("cannot override undefined {name}")))?;
    let args = definition
        .parameters
        .iter()
        .cloned()
        .map(Expr::Name)
        .collect();
    definition.body = Expr::Call(replacement, args);
    definition.standard = false;
    Ok(())
}

/// The `---- CONFIG Name ----` section a `.tla` file may carry after its module,
/// which TLC reads when the file is given as the configuration; empty otherwise.
pub fn embedded_config(text: &str) -> String {
    let Some(start) = text.lines().position(|line| {
        let words: Vec<_> = line.split_whitespace().collect();
        words.len() >= 3 && words[0].starts_with("----") && words[1] == "CONFIG"
    }) else {
        return String::new();
    };
    text.lines()
        .skip(start + 1)
        .take_while(|line| !line.trim_start().starts_with("===="))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assign(target: &mut Option<String>, value: String, section: &str) -> Result<(), Error> {
    if target.replace(value).is_some() {
        return Err(Error(format!("duplicate {section}")));
    }
    Ok(())
}

fn cfg_keyword(text: &str) -> bool {
    matches!(
        text,
        "INIT"
            | "NEXT"
            | "SPECIFICATION"
            | "VIEW"
            | "SYMMETRY"
            | "ALIAS"
            | "POSTCONDITION"
            | "POSTCONDITIONS"
            | "_POSSIBLE"
            | "CHECK_DEADLOCK"
            | "INVARIANT"
            | "INVARIANTS"
            | "PROPERTY"
            | "PROPERTIES"
            | "CONSTRAINT"
            | "CONSTRAINTS"
            | "ACTION_CONSTRAINT"
            | "ACTION_CONSTRAINTS"
            | "CONSTANT"
            | "CONSTANTS"
    )
}
