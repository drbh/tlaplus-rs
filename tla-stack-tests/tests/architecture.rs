//! The crate graph follows the layering declared under
//! `[workspace.metadata.architecture]` in the workspace manifest: every crate
//! belongs to a group, a crate may depend only on its own group or on groups
//! its group lists, and the normal-dependency group graph is acyclic.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

fn metadata() -> Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse cargo metadata")
}

fn groups(table: &Value) -> BTreeMap<String, BTreeSet<String>> {
    table
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .map(|(group, targets)| {
                    let targets = targets
                        .as_array()
                        .expect("allowed groups are arrays")
                        .iter()
                        .map(|t| t.as_str().expect("group name").to_owned())
                        .collect();
                    (group.clone(), targets)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn crates_follow_the_declared_layering() {
    let metadata = metadata();
    let architecture = &metadata["metadata"]["architecture"];
    let crates: BTreeMap<String, String> = architecture["crates"]
        .as_object()
        .expect("architecture.crates")
        .iter()
        .map(|(name, group)| (name.clone(), group.as_str().expect("group").to_owned()))
        .collect();
    let described: BTreeSet<&str> = architecture["groups"]
        .as_object()
        .expect("architecture.groups")
        .keys()
        .map(String::as_str)
        .collect();
    let normal = groups(&architecture["allowed"]["normal"]);
    let dev = groups(&architecture["allowed"]["dev"]);
    let build = groups(&architecture["allowed"]["build"]);

    let packages = metadata["packages"].as_array().expect("packages");
    let members: BTreeSet<&str> = packages
        .iter()
        .map(|p| p["name"].as_str().expect("package name"))
        .collect();
    let mut problems = Vec::new();
    for name in &members {
        if !crates.contains_key(*name) {
            problems.push(format!("{name} is not assigned to an architecture group"));
        }
    }
    for (name, group) in &crates {
        if !members.contains(name.as_str()) {
            problems.push(format!(
                "{name} is listed in architecture.crates but is not a workspace member"
            ));
        }
        if !described.contains(group.as_str()) {
            problems.push(format!("{name} belongs to undescribed group {group}"));
        }
    }
    for table in [&normal, &dev, &build] {
        for (group, targets) in table {
            for g in std::iter::once(group).chain(targets) {
                if !described.contains(g.as_str()) {
                    problems.push(format!("allowed matrix names undescribed group {g}"));
                }
            }
        }
    }

    for package in packages {
        let name = package["name"].as_str().unwrap();
        let Some(group) = crates.get(name) else {
            continue;
        };
        for dependency in package["dependencies"].as_array().expect("dependencies") {
            let target = dependency["name"].as_str().unwrap();
            let Some(target_group) = crates.get(target) else {
                continue;
            };
            if target_group == group {
                continue;
            }
            let kind = dependency["kind"].as_str().unwrap_or("normal");
            let allowed = match kind {
                "normal" => normal.get(group).is_some_and(|t| t.contains(target_group)),
                "dev" => [&normal, &dev]
                    .iter()
                    .any(|table| table.get(group).is_some_and(|t| t.contains(target_group))),
                "build" => [&normal, &build]
                    .iter()
                    .any(|table| table.get(group).is_some_and(|t| t.contains(target_group))),
                other => panic!("unknown dependency kind {other}"),
            };
            if !allowed {
                problems.push(format!(
                    "{name} ({group}) has a {kind} dependency on {target} ({target_group}), which the architecture does not allow"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn normal_dependencies_between_groups_are_acyclic() {
    let metadata = metadata();
    let normal = groups(&metadata["metadata"]["architecture"]["allowed"]["normal"]);
    let mut visiting = BTreeSet::new();
    let mut done = BTreeSet::new();
    fn visit(
        group: &str,
        normal: &BTreeMap<String, BTreeSet<String>>,
        visiting: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
        path: &mut Vec<String>,
    ) {
        if done.contains(group) {
            return;
        }
        assert!(
            visiting.insert(group.to_owned()),
            "architecture.allowed.normal has a cycle through {}",
            path.join(" -> ")
        );
        path.push(group.to_owned());
        for target in normal.get(group).into_iter().flatten() {
            visit(target, normal, visiting, done, path);
        }
        path.pop();
        visiting.remove(group);
        done.insert(group.to_owned());
    }
    for group in normal.keys() {
        visit(group, &normal, &mut visiting, &mut done, &mut Vec::new());
    }
}
