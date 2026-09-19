//! Every original lesson has a modern counterpart with matching expected behavior.
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}
fn run(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tla"))
        .args(args)
        .arg(path)
        .output()
        .unwrap()
}
fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
#[test]
fn every_original_has_a_checked_port() {
    let mut count = 0;
    for directory in std::fs::read_dir(root()).unwrap() {
        let directory = directory.unwrap().path();
        if !directory.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(directory).unwrap() {
            let original = file.unwrap().path();
            if original.extension().is_none_or(|e| e != "tla") {
                continue;
            }
            let port = original.with_extension("tsl");
            assert!(port.is_file(), "missing {}", port.display());
            count += 1;
            let name = original.file_stem().unwrap().to_str().unwrap();
            let flags: &[&str] = if name == "Point" { &["--check"] } else { &[] };
            let old = run(&original, flags);
            let new = run(&port, flags);
            assert!(
                matches!(new.status.code(), Some(0 | 1)),
                "{}: {}",
                port.display(),
                text(&new)
            );
            assert_eq!(
                old.status.code(),
                new.status.code(),
                "{}: {}",
                port.display(),
                text(&new)
            );
            if name == "Point" {
                continue;
            }
            let old = text(&old);
            let new = text(&new);
            let verdict = match name {
                "DuplicatesBug" => "Invariant IsCorrect is violated",
                "Calculator" => "Invariant NotTarget is violated",
                "Race" => "Invariant Correct is violated",
                "Deadlock" => "Deadlock reached",
                "Liveness" => "Temporal property Finishes violated",
                "RiverCrossing" | "RiverCrossingSimple" | "DieHard" => {
                    "Invariant NotSolved is violated"
                }
                _ => "Safety checks passed",
            };
            assert!(
                old.contains(verdict) && new.contains(verdict),
                "{name}: {old}\n{new}"
            );
            if verdict == "Safety checks passed" && name != "Dedup" {
                assert_eq!(
                    old.lines().next(),
                    new.lines().next(),
                    "{name}: complete counts"
                );
            }
        }
    }
    assert_eq!(
        count, 19,
        "include every original example and the Point library"
    );
}
#[test]
fn dedup_matches_the_original_without_symmetry_reduction() {
    let temp = tempfile::tempdir().unwrap();
    let original = root().join("05-constants/Dedup.tla");
    let source = std::fs::read_to_string(&original)
        .unwrap()
        .replace("SYMMETRY Symmetry\n", "");
    let path = temp.path().join("Dedup.tla");
    std::fs::write(&path, source).unwrap();
    let old = run(&path, &[]);
    let new = run(&original.with_extension("tsl"), &[]);
    assert!(old.status.success() && new.status.success());
    assert_eq!(text(&old).lines().next(), text(&new).lines().next());
    assert!(text(&new).contains("666 distinct states found (120 initial)"));
}

#[test]
fn river_refinements_preserve_the_solution_and_expose_unsafe_updates() {
    let path = root().join("16-puzzles/RiverCrossing.tsl");
    let source = std::fs::read_to_string(&path).unwrap();
    let solution = run(&path, &[]);
    assert!(text(&solution).contains("search depth 7"));
    assert!(text(&solution).contains("State 8:"));
    assert!(text(&solution).contains("Invariant NotSolved is violated"));
    let dir = tempfile::tempdir().unwrap();
    let unsafe_path = dir.path().join("RiverCrossing.tsl");
    std::fs::write(
        &unsafe_path,
        source.replace("require crossed(side, crew) is SafeRiver;", ""),
    )
    .unwrap();
    let unsafe_output = run(&unsafe_path, &[]);
    assert_eq!(unsafe_output.status.code(), Some(1));
    assert!(
        text(&unsafe_output).contains("Invariant TypeOK is violated"),
        "{}",
        text(&unsafe_output)
    );
    // Explore the full graph without the puzzle's deliberate failure.
    let complete = source.replace("!forall(c in Character) { side[c] == Right }", "true");
    std::fs::write(&unsafe_path, complete).unwrap();
    let complete = run(&unsafe_path, &[]);
    assert!(complete.status.success(), "{}", text(&complete));
    assert!(text(&complete).contains("10 distinct states found"));
}

#[test]
fn diehard_finds_six_moves_and_checks_all_jug_bounds() {
    let path = root().join("16-puzzles/DieHard.tsl");
    let solution = run(&path, &[]);
    assert_eq!(solution.status.code(), Some(1));
    let trace = text(&solution);
    assert!(trace.contains("Invariant NotSolved is violated"), "{trace}");
    assert_eq!(
        trace
            .lines()
            .filter(|line| line.starts_with("State "))
            .count(),
        7
    );
    assert!(trace.contains("\"Jug::Large\" :> 4"), "{trace}");

    let dir = tempfile::tempdir().unwrap();
    for extension in ["tla", "tsl"] {
        let source = std::fs::read_to_string(path.with_extension(extension)).unwrap();
        let complete = source
            .replace(
                "invariant NotSolved { !solved() }",
                "invariant NotSolved { true }",
            )
            .replace("INVARIANT NotSolved", "");
        let full_path = dir.path().join(format!("DieHard.{extension}"));
        std::fs::write(&full_path, complete).unwrap();
        let full = run(&full_path, &[]);
        assert!(full.status.success(), "{}", text(&full));
        assert!(
            text(&full).contains("16 distinct states found"),
            "{}",
            text(&full)
        );
    }
    // Refinements report an illegal update rather than silently excluding it.
    let source = std::fs::read_to_string(path).unwrap().replace(
        "put(water, j, capacity(j))",
        "put(water, j, capacity(j) + 1)",
    );
    let invalid = dir.path().join("DieHard.tsl");
    std::fs::write(&invalid, source).unwrap();
    let result = run(&invalid, &[]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        text(&result).contains("Invariant TypeOK is violated"),
        "{}",
        text(&result)
    );
}

#[test]
fn typed_diehard_preserves_the_puzzle_and_distinguishes_jugs() {
    let path = root().join("16-puzzles/DieHardTyped.tsl");
    let source = std::fs::read_to_string(&path).unwrap();
    let solution = run(&path, &[]);
    let trace = text(&solution);
    assert_eq!(solution.status.code(), Some(1));
    assert!(trace.contains("Invariant NotSolved is violated"), "{trace}");
    assert_eq!(
        trace
            .lines()
            .filter(|line| line.starts_with("State "))
            .count(),
        7
    );
    assert!(trace.contains("large = [gallons |-> 4]"), "{trace}");

    // Struct identity is checked statically, even though both have gallons.
    let swapped = source.replace("next small = SmallJug", "next small = LargeJug");
    assert!(tla_dsl::compile(&swapped).is_err());

    let dir = tempfile::tempdir().unwrap();
    let temporary = dir.path().join("DieHardTyped.tsl");
    let complete = source.replace(
        "invariant NotSolved { !solved() }",
        "invariant NotSolved { true }",
    );
    std::fs::write(&temporary, complete).unwrap();
    let full = run(&temporary, &[]);
    assert!(full.status.success(), "{}", text(&full));
    assert!(
        text(&full).contains("16 distinct states found"),
        "{}",
        text(&full)
    );
    for (valid, invalid) in [
        (
            "next small = SmallJug { gallons: 3 }",
            "next small = SmallJug { gallons: 4 }",
        ),
        (
            "next large = LargeJug { gallons: 5 }",
            "next large = LargeJug { gallons: 6 }",
        ),
    ] {
        std::fs::write(&temporary, source.replace(valid, invalid)).unwrap();
        let result = run(&temporary, &[]);
        assert_eq!(result.status.code(), Some(1));
        assert!(
            text(&result).contains("Invariant TypeOK is violated"),
            "{}",
            text(&result)
        );
    }
}
