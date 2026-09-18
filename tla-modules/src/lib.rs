use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
mod config;
pub use config::{Config, config, embedded_config, override_operators};
use tla_syntax::{Expr, Module};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

/// The standard modules, loaded like user modules when no file shadows them.
const STANDARD: &[(&str, &str)] = &[
    ("Bags", include_str!("../../library/Bags.tla")),
    ("FiniteSets", include_str!("../../library/FiniteSets.tla")),
    ("Integers", include_str!("../../library/Integers.tla")),
    ("Json", include_str!("../../library/Json.tla")),
    ("Naturals", include_str!("../../library/Naturals.tla")),
    (
        "Randomization",
        include_str!("../../library/Randomization.tla"),
    ),
    ("Reals", include_str!("../../library/Reals.tla")),
    ("RealTime", include_str!("../../library/RealTime.tla")),
    ("Sequences", include_str!("../../library/Sequences.tla")),
    ("TLC", include_str!("../../library/TLC.tla")),
    ("TLCExt", include_str!("../../library/TLCExt.tla")),
    ("Toolbox", include_str!("../../library/Toolbox.tla")),
];

/// CommunityModules whose operators TLC may implement natively.
const COMMUNITY: &[&str] = &[
    "BagsExt",
    "Bitwise",
    "Combinatorics",
    "CSV",
    "DyadicRationals",
    "FiniteSetsExt",
    "Folds",
    "Functions",
    "Graphs",
    "GraphViz",
    "HTML",
    "IOUtils",
    "Relation",
    "SequencesExt",
    "ShiViz",
    "Statistics",
    "SVG",
    "UndirectedGraphs",
    "VectorClocks",
];

pub fn load(path: &Path, search: &[PathBuf]) -> Result<Module, Error> {
    let text =
        std::fs::read_to_string(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let mut loader = Loader {
        search: std::iter::once(path.parent().unwrap_or(Path::new(".")).to_path_buf())
            .chain(search.iter().cloned())
            .collect(),
        modules: BTreeMap::new(),
        active: BTreeSet::new(),
        nested: Vec::new(),
    };
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let module = tla_parser::module(&text).map_err(|e| Error(format!("{name}: {e}")))?;
    loader.visit(name, module, false)
}

struct Loader {
    search: Vec<PathBuf>,
    modules: BTreeMap<String, Module>,
    active: BTreeSet<String>,
    /// Modules nested in the modules being loaded, innermost last.
    nested: Vec<Vec<Module>>,
}

impl Loader {
    fn resolve(&mut self, name: &str) -> Result<Module, Error> {
        if let Some(module) = self.modules.get(name) {
            return imported(module.clone());
        }
        let inner = self
            .nested
            .iter()
            .rev()
            .flatten()
            .find(|m| m.name == name)
            .cloned();
        if let Some(inner) = inner {
            let module = self.visit(name, inner, false)?;
            self.modules.insert(name.into(), module.clone());
            return imported(module);
        }
        let file = self
            .search
            .iter()
            .map(|dir| dir.join(format!("{name}.tla")))
            .find(|file| file.is_file());
        let module = match (file, STANDARD.iter().find(|(n, _)| *n == name)) {
            (Some(file), _) => {
                let text = std::fs::read_to_string(&file)
                    .map_err(|e| Error(format!("{}: {e}", file.display())))?;
                let module =
                    tla_parser::module(&text).map_err(|e| Error(format!("{name}: {e}")))?;
                self.visit(name, module, COMMUNITY.contains(&name))?
            }
            (None, Some((_, text))) => {
                let module = tla_parser::module(text).map_err(|e| Error(format!("{name}: {e}")))?;
                self.visit(name, module, true)?
            }
            (None, None) => return Err(Error(format!("module {name} not found"))),
        };
        self.modules.insert(name.into(), module.clone());
        imported(module)
    }

    fn visit(&mut self, name: &str, mut module: Module, standard: bool) -> Result<Module, Error> {
        if !self.active.insert(name.into()) {
            return Err(Error(format!("cyclic module dependency: {name}")));
        }
        self.nested.push(std::mem::take(&mut module.modules));
        let result = self.flatten(name, module, standard);
        self.nested.pop();
        self.active.remove(name);
        result
    }

    fn flatten(&mut self, name: &str, mut module: Module, standard: bool) -> Result<Module, Error> {
        if module.name != name {
            return Err(Error(format!(
                "file name '{name}' does not match the name '{}' of the module it contains",
                module.name
            )));
        }
        for definition in &mut module.definitions {
            definition.standard = standard;
        }
        let mut inherited = Vec::new();
        for dependency in module.extends.clone() {
            let parent = self.resolve(&dependency)?;
            for definition in parent.definitions {
                if let Some(existing) = inherited
                    .iter()
                    .find(|d: &&tla_syntax::Definition| d.name == definition.name)
                {
                    if *existing != definition {
                        return Err(Error(format!(
                            "conflicting inherited operator {}",
                            definition.name
                        )));
                    }
                } else {
                    inherited.push(definition);
                }
            }
            for variable in parent.variables {
                if !module.variables.contains(&variable) {
                    module.variables.push(variable);
                }
            }
            for constant in parent.constants {
                if !module.constants.contains(&constant) {
                    module.constants.push(constant);
                }
            }
            module.assumptions.extend(parent.assumptions);
        }
        for definition in &module.definitions {
            if inherited.iter().any(|d| d.name == definition.name) {
                return Err(Error(format!(
                    "operator {} conflicts with EXTENDS",
                    definition.name
                )));
            }
        }
        for instance in std::mem::take(&mut module.instances) {
            let parent = self.resolve(&instance.module)?;
            self.instantiate(&mut module, &mut inherited, instance, parent)?;
        }
        if !standard {
            let own: std::collections::BTreeSet<_> =
                module.declared_at.iter().map(|(_, name)| name).collect();
            let scope = inherited
                .iter()
                .map(|d| d.name.clone())
                .chain(module.variables.iter().cloned())
                .chain(module.constants.iter().cloned())
                .filter(|name| !own.contains(name))
                .collect();
            tla_syntax::check_scopes(
                &module.definitions,
                &module.assumptions,
                &module.declared_at,
                scope,
            )
            .map_err(|e| Error(format!("{name}: {}", e.message)))?;
        }
        inherited.append(&mut module.definitions);
        module.definitions = inherited;
        Ok(module)
    }

    fn instantiate(
        &mut self,
        module: &mut Module,
        inherited: &mut Vec<tla_syntax::Definition>,
        instance: tla_syntax::Instance,
        parent: Module,
    ) -> Result<(), Error> {
        let mut replacements = BTreeMap::new();
        for (name, value) in instance.substitutions {
            if !parent.constants.contains(&name) && !parent.variables.contains(&name) {
                return Err(Error(format!(
                    "cannot substitute undeclared parameter {name}"
                )));
            }
            replacements.insert(name, value);
        }
        for name in parent.constants.iter().chain(&parent.variables) {
            // An omitted substitution is implicitly by the same name in scope.
            if !replacements.contains_key(name)
                && !module.constants.contains(name)
                && !module.variables.contains(name)
                && !inherited
                    .iter()
                    .chain(&module.definitions)
                    .any(|d| &d.name == name)
                && !instance.parameters.contains(name)
            {
                return Err(Error(format!(
                    "instance parameter {name} needs a substitution or declaration"
                )));
            }
        }
        let prefix: Vec<_> = instance
            .parameters
            .iter()
            .cloned()
            .map(Expr::Name)
            .collect();
        let mut renamed = BTreeMap::new();
        for definition in &parent.definitions {
            let name = instance.name.as_ref().map_or_else(
                || definition.name.clone(),
                |prefix| format!("{prefix}!{}", definition.name),
            );
            // Parameterized instances pass their parameters to every operator.
            let reference = if prefix.is_empty() {
                Expr::Name(name.clone())
            } else {
                Expr::Call(name.clone(), prefix.clone())
            };
            replacements.insert(definition.name.clone(), reference);
            renamed.insert(definition.name.clone(), name);
        }
        for definition in parent.definitions {
            let name = renamed[&definition.name].clone();
            if let Some(existing) = inherited
                .iter()
                .chain(&module.definitions)
                .find(|d| d.name == name)
            {
                // Standard modules instantiated along several paths are identical.
                if existing.standard && definition.standard && instance.name.is_none() {
                    continue;
                }
                return Err(Error(format!("duplicate instantiated operator {name}")));
            }
            let mut definition = tla_syntax::substitute_definition(&definition, &replacements)
                .map_err(|e| Error(e.to_string()))?;
            definition.name = name;
            definition
                .parameters
                .splice(0..0, instance.parameters.iter().cloned());
            definition
                .arities
                .splice(0..0, instance.parameters.iter().map(|_| 0));
            if instance.local {
                module.local_names.push(definition.name.clone());
            }
            inherited.push(definition);
        }
        for assumption in parent.assumptions {
            module.assumptions.push(
                tla_syntax::substitute(&assumption, &replacements)
                    .map_err(|e| Error(e.to_string()))?,
            );
        }
        Ok(())
    }
}

fn imported(mut module: Module) -> Result<Module, Error> {
    let replacements = module
        .local_names
        .iter()
        .map(|name| (name.clone(), Expr::Name(format!("{}@{name}", module.name))))
        .collect();
    for definition in &mut module.definitions {
        let mut renamed = tla_syntax::substitute_definition(definition, &replacements)
            .map_err(|e| Error(e.to_string()))?;
        if module.local_names.contains(&definition.name) {
            renamed.name = format!("{}@{}", module.name, definition.name);
        }
        *definition = renamed;
    }
    module.assumptions = module
        .assumptions
        .iter()
        .map(|a| tla_syntax::substitute(a, &replacements).map_err(|e| Error(e.to_string())))
        .collect::<Result<_, _>>()?;
    module.local_names.clear();
    Ok(module)
}
