use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn source_files(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .flat_map(|entry| {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "crate file is a symlink: {}",
                path.display()
            );
            if kind.is_dir() {
                if entry.file_name() == "target" {
                    Vec::new()
                } else {
                    source_files(&path)
                }
            } else {
                vec![path]
            }
        })
        .collect()
}

#[test]
fn workspace_crates_stay_within_file_and_line_limits() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version=1", "--no-deps", "--offline"])
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let members = metadata["workspace_members"].as_array().unwrap();
    let mut violations = Vec::new();
    for package in metadata["packages"].as_array().unwrap() {
        if !members.contains(&package["id"]) {
            continue;
        }
        let manifest = Path::new(package["manifest_path"].as_str().unwrap());
        let faithful = Path::new(metadata["workspace_root"].as_str().unwrap())
            .parent()
            .unwrap()
            .join("tlaplus-rs-explore/crates");
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["name"] == "tla-bridge" && dependency["kind"] != "dev" {
                violations.push(format!(
                    "{}: tla-bridge must be a dev-dependency",
                    package["name"]
                ));
            }
            if package["name"] != "tla-bridge"
                && dependency["path"]
                    .as_str()
                    .is_some_and(|path| Path::new(path).starts_with(&faithful))
            {
                violations.push(format!(
                    "{}: direct faithful dependency {}",
                    package["name"], dependency["name"]
                ));
            }
        }
        let files = source_files(manifest.parent().unwrap());
        // tla-bridge is the one documented exception (AGENTS.md): it holds five
        // milestone deliverables that cannot leave the crate, and its lowering
        // is split into `lower` and `lower::apply` to stay under the line limit.
        let maximum = if package["name"] == "tla-bridge" {
            6
        } else {
            5
        };
        if files.len() > maximum {
            violations.push(format!(
                "{}: {} files; maximum is {maximum}",
                package["name"],
                files.len()
            ));
        }
        for file in files {
            let count = fs::read_to_string(&file).unwrap().lines().count();
            if count > 510 {
                violations.push(format!("{}: {count} lines; maximum is 510", file.display()));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}
