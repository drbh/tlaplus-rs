//! The shape every crate keeps so the code stays readable: each opens with
//! module docs, errors are typed rather than strings, and no function runs
//! past 170 lines.
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FUNCTION_LINES: usize = 170;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn crates() -> Vec<PathBuf> {
    let mut crates: Vec<_> = fs::read_dir(workspace())
        .expect("workspace")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?;
            (name.starts_with("tla-") && path.join("src").is_dir()).then_some(path)
        })
        .collect();
    crates.sort();
    assert!(
        crates.len() > 10,
        "expected the workspace crates, found {crates:?}"
    );
    crates
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_crate_opens_with_module_docs() {
    let missing: Vec<_> = crates()
        .into_iter()
        .filter(|path| {
            let root = ["src/lib.rs", "src/main.rs"]
                .iter()
                .map(|file| path.join(file))
                .find(|file| file.exists())
                .expect("crate root");
            !fs::read_to_string(root).expect("root").starts_with("//!")
        })
        .collect();
    assert!(
        missing.is_empty(),
        "crates without module docs: {missing:?}"
    );
}

/// Every function signature names an error type; a `String` is a payload,
/// never an error.
#[test]
fn crates_use_typed_errors() {
    let mut offenders = Vec::new();
    for crate_dir in crates() {
        let mut files = Vec::new();
        sources(&crate_dir.join("src"), &mut files);
        for file in files {
            for (number, line) in fs::read_to_string(&file)
                .expect("source")
                .lines()
                .enumerate()
            {
                if line.contains("-> ") && line.contains("Result<") && line.contains(", String>") {
                    offenders.push(format!(
                        "{}:{}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "string-typed errors:\n{}",
        offenders.join("\n")
    );
}

/// A function runs from its `fn` line to the first line that is only its
/// closing brace at the same indentation.
fn function_lengths(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut result = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let is_fn = trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub(crate) fn ")
            || trimmed.starts_with("pub(super) fn ");
        if !is_fn || trimmed.ends_with(';') {
            continue;
        }
        let close = format!("{}}}", " ".repeat(indent));
        let end = lines[start + 1..]
            .iter()
            .position(|l| *l == close)
            .map_or(lines.len(), |offset| start + 1 + offset);
        result.push((end - start + 1, trimmed.to_owned()));
    }
    result
}

#[test]
fn no_function_exceeds_the_length_limit() {
    let mut offenders = Vec::new();
    for crate_dir in crates() {
        let mut files = Vec::new();
        sources(&crate_dir.join("src"), &mut files);
        for file in files {
            for (length, signature) in function_lengths(&fs::read_to_string(&file).expect("source"))
            {
                if length > MAX_FUNCTION_LINES {
                    offenders.push(format!("{length} lines: {}: {signature}", file.display()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "functions over {MAX_FUNCTION_LINES} lines:\n{}",
        offenders.join("\n")
    );
}
