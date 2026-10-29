//! Optional differential smoke test of the DSL's portable TLA+ exports.
use std::{path::Path, process::Command};

#[test]
#[ignore = "requires TLA2TOOLS_JAR and JAVA (defaults to java)"]
fn dsl_exports_agree_with_tlc() {
    let jar = std::env::var("TLA2TOOLS_JAR")
        .expect("set TLA2TOOLS_JAR to an absolute tla2tools.jar path");
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/17-modern-dsl");
    let temp = tempfile::tempdir().unwrap();
    for (name, native_code, tlc_code, message, states) in [
        (
            "AtomicCounter",
            0,
            0,
            "Model checking completed. No error has been found.",
            Some(4),
        ),
        (
            "Lock",
            0,
            0,
            "Model checking completed. No error has been found.",
            Some(3),
        ),
        (
            "Retry",
            0,
            0,
            "Model checking completed. No error has been found.",
            Some(3),
        ),
        (
            "Race",
            1,
            12,
            "Invariant BothIncrementsCounted is violated",
            None,
        ),
        (
            "BrokenLock",
            1,
            12,
            "Invariant MutualExclusion is violated",
            None,
        ),
        ("Choice", 1, 12, "Invariant BelowThree is violated", None),
        (
            "UnfairRetry",
            1,
            13,
            "Temporal property DeliveredEventually was violated",
            None,
        ),
    ] {
        let source = examples.join(format!("{name}.tsl"));
        let output = temp.path().join(name);
        let export = Command::new(env!("CARGO_BIN_EXE_tla"))
            .arg("--export")
            .arg(&source)
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            export.status.success(),
            "{}",
            String::from_utf8_lossy(&export.stderr)
        );
        let native = Command::new(env!("CARGO_BIN_EXE_tla"))
            .arg(&source)
            .output()
            .unwrap();
        let native_text = format!(
            "{}{}",
            String::from_utf8_lossy(&native.stdout),
            String::from_utf8_lossy(&native.stderr)
        );
        assert_eq!(
            native.status.code(),
            Some(native_code),
            "{name}: {native_text}"
        );
        let tlc = Command::new(&java)
            .args([
                "-XX:+UseParallelGC",
                "-cp",
                &jar,
                "tlc2.TLC",
                "-workers",
                "1",
                name,
            ])
            .current_dir(&output)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&tlc.stdout),
            String::from_utf8_lossy(&tlc.stderr)
        );
        assert_eq!(tlc.status.code(), Some(tlc_code), "{name}: {text}");
        assert!(text.contains(message), "{name}: {text}");
        if let Some(states) = states {
            let count = format!("{states} distinct states found");
            assert!(text.contains(&count), "{name}: {text}");
            assert!(native_text.contains(&count), "{name}: {native_text}");
        }
    }
}

#[test]
#[ignore = "requires TLA2TOOLS_JAR and JAVA (defaults to java)"]
fn all_lesson_exports_agree_with_tlc() {
    let jar = std::env::var("TLA2TOOLS_JAR").expect("absolute path to tla2tools.jar");
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let temp = tempfile::tempdir().unwrap();
    let mut checked = 0;
    for directory in std::fs::read_dir(&examples).unwrap() {
        let directory = directory.unwrap().path();
        if !directory.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(directory).unwrap() {
            let original = file.unwrap().path();
            if original.extension().is_none_or(|e| e != "tla") {
                continue;
            }
            let name = original.file_stem().unwrap().to_str().unwrap();
            if name == "Point" {
                continue;
            }
            let port = original.with_extension("tsl");
            let output = temp.path().join(name);
            let export = Command::new(env!("CARGO_BIN_EXE_tla"))
                .arg("--export")
                .arg(&port)
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                export.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&export.stderr)
            );
            let native = Command::new(env!("CARGO_BIN_EXE_tla"))
                .arg(&port)
                .output()
                .unwrap();
            let native_text = String::from_utf8_lossy(&native.stdout);
            let (code, message) = match name {
                "DuplicatesBug" => (12, "Invariant IsCorrect is violated"),
                "Calculator" => (12, "Invariant NotTarget is violated"),
                "Race" => (12, "Invariant Correct is violated"),
                "Deadlock" => (11, "Deadlock reached"),
                "Liveness" => (13, "Temporal property Finishes was violated"),
                "RiverCrossing" | "RiverCrossingSimple" | "DieHard" => {
                    (12, "Invariant NotSolved is violated")
                }
                _ => (0, "Model checking completed. No error has been found."),
            };
            assert_eq!(
                native.status.code(),
                Some(i32::from(code != 0)),
                "{name}: {}",
                String::from_utf8_lossy(&native.stderr)
            );
            let tlc = Command::new(&java)
                .args([
                    "-XX:+UseParallelGC",
                    "-cp",
                    &jar,
                    "tlc2.TLC",
                    "-workers",
                    "1",
                    name,
                ])
                .current_dir(&output)
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&tlc.stdout),
                String::from_utf8_lossy(&tlc.stderr)
            );
            assert_eq!(tlc.status.code(), Some(code), "{name}: {text}");
            assert!(text.contains(message), "{name}: {text}");
            if code == 0 {
                let count = native_text.split_whitespace().next().unwrap();
                assert!(
                    text.contains(&format!("{count} distinct states found")),
                    "{name}: {text}"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 18);
}

#[test]
#[ignore = "requires TLA2TOOLS_JAR and JAVA (defaults to java)"]
fn river_refinement_obligations_agree_with_tlc() {
    let jar = std::env::var("TLA2TOOLS_JAR").expect("absolute path to tla2tools.jar");
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let source = include_str!("../../examples/16-puzzles/RiverCrossing.tsl");
    for (source, expected, message) in [
        (
            source.replace("!forall(c in Character) { side[c] == Right }", "true"),
            0,
            "10 distinct states found",
        ),
        (
            source.replace("require crossed(side, crew) is SafeRiver;", ""),
            12,
            "Invariant TypeOK is violated",
        ),
        (
            source.replace("map(c in Character) { Left };", "entry(Farmer, Left);"),
            12,
            "Invariant TypeOK is violated",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let artifact = tlaplus_dsl::compile(&source).unwrap();
        std::fs::write(temp.path().join("RiverCrossing.tla"), artifact.tla).unwrap();
        std::fs::write(temp.path().join("RiverCrossing.cfg"), artifact.config).unwrap();
        let tlc = Command::new(&java)
            .args([
                "-XX:+UseParallelGC",
                "-cp",
                &jar,
                "tlc2.TLC",
                "-workers",
                "1",
                "RiverCrossing",
            ])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&tlc.stdout),
            String::from_utf8_lossy(&tlc.stderr)
        );
        assert_eq!(tlc.status.code(), Some(expected), "{text}");
        assert!(text.contains(message), "{text}");
    }
}

#[test]
#[ignore = "requires TLA2TOOLS_JAR and JAVA (defaults to java)"]
fn composition_and_contract_exports_agree_with_tlc() {
    let jar = std::env::var("TLA2TOOLS_JAR").expect("absolute path to tla2tools.jar");
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let protocol = include_str!("../../examples/18-reliable-delivery/ReliableDelivery.tsl");
    let predicates = "spec PredicateDomains {
        type Percent = Int where n { n >= 0 && n <= limit() }
        fn limit() -> Int { 100 }
        domain Percent in 0..=2;
        fn keep(n: Percent) -> Percent { n }
        state x: Percent = keep(50);
        action stay {}
        invariant Independent { x is Percent && size(Percent) == 3 }
    }";
    let composition = "spec AtomicComposition {
        state x: Int; state y: Int;
        init { pick seed in 0..=2; next x = seed; next y = seed; }
        view equal() -> Bool { x == y }
        action inc { require x < 3; next x = x + 1; }
        action both { call inc(); next y = y + 1; }
        action done { require x == 3; }
        next { both, done }
        invariant Same { equal() }
    }";
    for (source, success, message) in [
        (protocol.to_string(), true, "100 distinct states found"),
        (
            protocol.replace("fair strong retry;", ""),
            false,
            "Temporal property AllComplete was violated",
        ),
        (
            protocol.replace("-> Count { 1 }", "-> Count { n + 1 }"),
            false,
            "postcondition of applied_once",
        ),
        (predicates.into(), true, "1 distinct states found"),
        (composition.into(), true, "4 distinct states found"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let artifact = tlaplus_dsl::compile(&source).unwrap();
        std::fs::write(
            temp.path().join(format!("{}.tla", artifact.module)),
            artifact.tla,
        )
        .unwrap();
        std::fs::write(
            temp.path().join(format!("{}.cfg", artifact.module)),
            artifact.config,
        )
        .unwrap();
        let tlc = Command::new(&java)
            .args([
                "-XX:+UseParallelGC",
                "-cp",
                &jar,
                "tlc2.TLC",
                "-workers",
                "1",
                &artifact.module,
            ])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&tlc.stdout),
            String::from_utf8_lossy(&tlc.stderr)
        );
        assert_eq!(tlc.status.success(), success, "{text}");
        assert!(text.contains(message), "{text}");
    }
}

#[test]
#[ignore = "requires TLA2TOOLS_JAR and JAVA (defaults to java)"]
fn typed_diehard_exports_agree_with_tlc() {
    let jar = std::env::var("TLA2TOOLS_JAR").expect("absolute path to tla2tools.jar");
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".into());
    let source = include_str!("../../examples/16-puzzles/DieHardTyped.tsl");
    for (source, code, message) in [
        (source.to_string(), 12, "Invariant NotSolved is violated"),
        (
            source.replace(
                "invariant NotSolved { !solved() }",
                "invariant NotSolved { true }",
            ),
            0,
            "16 distinct states found",
        ),
        (
            source.replace(
                "next small = SmallJug { gallons: 3 }",
                "next small = SmallJug { gallons: 4 }",
            ),
            12,
            "Invariant TypeOK is violated",
        ),
        (
            source.replace(
                "next large = LargeJug { gallons: 5 }",
                "next large = LargeJug { gallons: 6 }",
            ),
            12,
            "Invariant TypeOK is violated",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let artifact = tlaplus_dsl::compile(&source).unwrap();
        std::fs::write(temp.path().join("DieHardTyped.tla"), artifact.tla).unwrap();
        std::fs::write(temp.path().join("DieHardTyped.cfg"), artifact.config).unwrap();
        let tlc = Command::new(&java)
            .args([
                "-XX:+UseParallelGC",
                "-cp",
                &jar,
                "tlc2.TLC",
                "-workers",
                "1",
                "DieHardTyped",
            ])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&tlc.stdout),
            String::from_utf8_lossy(&tlc.stderr)
        );
        assert_eq!(tlc.status.code(), Some(code), "{text}");
        assert!(text.contains(message), "{text}");
    }
}
