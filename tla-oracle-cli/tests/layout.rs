use std::path::Path;
use std::process::Command;

#[test]
fn workspace_crates_respect_dependency_rules() {
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
        let faithful = Path::new(metadata["workspace_root"].as_str().unwrap())
            .parent()
            .unwrap()
            .join("tlaplus-rs-explore/crates");
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["name"] == "tlaplus-bridge" && dependency["kind"] != "dev" {
                violations.push(format!(
                    "{}: tla-bridge must be a dev-dependency",
                    package["name"]
                ));
            }
            if package["name"] != "tlaplus-bridge"
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
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}
