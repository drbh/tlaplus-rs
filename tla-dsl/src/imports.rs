//! Resolve local module imports with cycle checks; imports share one namespace.
use crate::{Error, Location, ast::Spec, parser};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub(crate) fn load(path: &Path) -> Result<Spec, Error> {
    visit(path, &mut BTreeSet::new(), &mut BTreeSet::new())
}
fn visit(
    path: &Path,
    active: &mut BTreeSet<PathBuf>,
    loaded: &mut BTreeSet<PathBuf>,
) -> Result<Spec, Error> {
    let at = Location { line: 1, column: 1 };
    let io = |e| Error {
        location: at,
        message: format!("{}: {e}", path.display()),
    };
    let canonical = path.canonicalize().map_err(io)?;
    if !active.insert(canonical.clone()) {
        return Err(Error {
            location: at,
            message: "cyclic module import".into(),
        });
    }
    let source = std::fs::read_to_string(path).map_err(io)?;
    let mut spec = parser::parse(&source)?;
    let mut enums = vec![];
    let mut records = vec![];
    let mut refinements = vec![];
    let mut domains = vec![];
    let mut constants = vec![];
    let mut functions = vec![];
    let mut assumptions = vec![];
    for import in std::mem::take(&mut spec.imports) {
        let file = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{}.tsl", import.text));
        let target = file.canonicalize().map_err(|e| Error {
            location: import.at,
            message: format!("{}: {e}", file.display()),
        })?;
        if loaded.contains(&target) {
            continue;
        }
        let module = visit(&file, active, loaded).map_err(|e| Error {
            location: import.at,
            message: format!("import {}: {e}", file.display()),
        })?;
        if !module.library || module.name.text != import.text {
            return Err(Error {
                location: import.at,
                message: "import must name a matching module declaration".into(),
            });
        }
        if !module.states.is_empty()
            || !module.actions.is_empty()
            || !module.properties.is_empty()
            || !module.fairness.is_empty()
            || module.init.is_some()
            || module.next.is_some()
            || module.functions.iter().any(|f| f.stateful)
        {
            return Err(Error {
                location: import.at,
                message: "imported modules cannot contain state, actions, properties, or fairness"
                    .into(),
            });
        }
        enums.extend(module.enums);
        records.extend(module.records);
        refinements.extend(module.refinements);
        domains.extend(module.domains);
        constants.extend(module.constants);
        functions.extend(module.functions);
        assumptions.extend(module.assumptions);
    }
    enums.append(&mut spec.enums);
    records.append(&mut spec.records);
    refinements.append(&mut spec.refinements);
    domains.append(&mut spec.domains);
    constants.append(&mut spec.constants);
    functions.append(&mut spec.functions);
    assumptions.append(&mut spec.assumptions);
    spec.enums = enums;
    spec.records = records;
    spec.refinements = refinements;
    spec.domains = domains;
    spec.constants = constants;
    spec.functions = functions;
    spec.assumptions = assumptions;
    active.remove(&canonical);
    loaded.insert(canonical);
    Ok(spec)
}
