//! An experimental typed action DSL. Compilation validates types and temporal
//! levels, then emits an inspectable TLA+ module and a separate TLC configuration.
mod ast;
mod check;
mod composition;
mod definitions;
mod emit;
mod imports;
mod lexer;
mod parser;
mod refinements;
mod walk;

use std::fmt;

/// A location in the original UTF-8 source (one-based line and character column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

/// A source-located compile error. Invalid input never reaches the checker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub location: Location,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.location.line, self.location.column, self.message
        )
    }
}
impl std::error::Error for Error {}

/// An operator's origin, for tools that present generated code or traces.
#[derive(Clone, Debug)]
pub struct Origin {
    pub name: String,
    pub generated_line: usize,
    pub source: Location,
}

/// Portable output: no embedded configuration or dependency on a DSL runtime.
#[derive(Clone, Debug)]
pub struct Artifact {
    pub module: String,
    pub tla: String,
    pub config: String,
    pub origins: Vec<Origin>,
    /// Declared actions, in source order, for source-level trace labels.
    pub actions: Vec<String>,
}

/// Compile one specification. An explicit next block selects action families;
/// otherwise all declared actions participate. Every invariant and property is checked.
pub fn compile(source: &str) -> Result<Artifact, Error> {
    let mut spec = parser::parse(source)?;
    if let Some(import) = spec.imports.first() {
        return Err(Error {
            location: import.at,
            message: "imports require compile_file with a source path".into(),
        });
    }
    composition::lower(&mut spec)?;
    refinements::lower(&mut spec)?;
    definitions::order(&mut spec)?;
    check::validate(&spec)?;
    Ok(emit::emit(&spec))
}

/// Compile a file and its sibling module imports into one standalone artifact.
pub fn compile_file(path: &std::path::Path) -> Result<Artifact, Error> {
    let mut spec = imports::load(path)?;
    composition::lower(&mut spec)?;
    refinements::lower(&mut spec)?;
    definitions::order(&mut spec)?;
    check::validate(&spec)?;
    Ok(emit::emit(&spec))
}

#[cfg(test)]
mod tests;
