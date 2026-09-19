//! DSL compilation, portable export, and temporary generated input for checking.
use super::Error;
use std::path::{Path, PathBuf};

pub fn compile(path: &Path) -> Result<tla_dsl::Artifact, Error> {
    tla_dsl::compile_file(path).map_err(|error| Error::Dsl {
        path: path.into(),
        error,
    })
}

/// Keep the temporary directory alive while the checker loads the exported model.
pub struct Generated {
    pub path: PathBuf,
    artifact: tla_dsl::Artifact,
    _directory: tempfile::TempDir,
}

pub fn generate(path: &Path) -> Result<Generated, Error> {
    let artifact = compile(path)?;
    if artifact.config.is_empty() {
        return Err(super::usage(
            "a module is a library; check it with --check or import it into a spec",
        ));
    }
    let directory = tempfile::tempdir().map_err(|error| Error::Io {
        path: std::env::temp_dir(),
        error,
    })?;
    let generated = write(&artifact, directory.path())?;
    Ok(Generated {
        path: generated,
        artifact,
        _directory: directory,
    })
}

impl Generated {
    /// Attach original locations without changing the transition predicates.
    pub fn annotate(&self, model: &mut tla_checker::BoundModel, source: &Path) {
        for (name, _) in &mut model.checks.actions {
            let original = if name == "Next" && self.artifact.actions.len() == 1 {
                &self.artifact.actions[0]
            } else {
                &*name
            };
            if let Some(origin) = self.artifact.origins.iter().find(|o| &o.name == original) {
                *name = format!(
                    "{} ({}:{}:{})",
                    origin.name,
                    source.display(),
                    origin.source.line,
                    origin.source.column
                );
            }
        }
    }
}

pub fn export(source: &Path, directory: &Path) -> Result<(), Error> {
    let artifact = compile(source)?;
    std::fs::create_dir_all(directory).map_err(|error| Error::Io {
        path: directory.into(),
        error,
    })?;
    let path = write(&artifact, directory)?;
    println!(
        "Exported {} and {}",
        path.display(),
        path.with_extension("cfg").display()
    );
    Ok(())
}

fn write(artifact: &tla_dsl::Artifact, directory: &Path) -> Result<PathBuf, Error> {
    let path = directory.join(format!("{}.tla", artifact.module));
    let mut created = Vec::new();
    let result = (|| {
        for (file, contents) in [
            (&path, &artifact.tla),
            (&path.with_extension("cfg"), &artifact.config),
        ] {
            // Never overwrite an existing hand-written model or an earlier export.
            use std::io::Write;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file)
                .map_err(|error| Error::Io {
                    path: file.into(),
                    error,
                })?;
            created.push(file.clone());
            output
                .write_all(contents.as_bytes())
                .map_err(|error| Error::Io {
                    path: file.into(),
                    error,
                })?;
        }
        Ok::<_, Error>(())
    })();
    if let Err(error) = result {
        for file in created {
            let _ = std::fs::remove_file(file);
        }
        return Err(error);
    }
    Ok(path)
}
