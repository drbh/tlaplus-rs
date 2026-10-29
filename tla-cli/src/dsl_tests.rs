//! End-to-end checks of exported semantics and command-line integration.
use super::*;

fn check_dsl(source: &str, flags: &[&str]) -> Result<i32, Error> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.tsl");
    std::fs::write(&path, source).unwrap();
    let mut args = flags.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    args.push(path.display().to_string());
    run(args)
}

#[test]
fn examples_detect_races_and_distinguish_weak_from_strong_fairness() {
    for (source, expected) in [
        (
            include_str!("../../examples/17-modern-dsl/AtomicCounter.tsl"),
            0,
        ),
        (include_str!("../../examples/17-modern-dsl/Race.tsl"), 1),
        (include_str!("../../examples/17-modern-dsl/Lock.tsl"), 0),
        (
            include_str!("../../examples/17-modern-dsl/BrokenLock.tsl"),
            1,
        ),
        (include_str!("../../examples/17-modern-dsl/Retry.tsl"), 0),
        (
            include_str!("../../examples/17-modern-dsl/UnfairRetry.tsl"),
            1,
        ),
        (include_str!("../../examples/17-modern-dsl/Choice.tsl"), 1),
    ] {
        for flags in [&[][..], &["--workers", "2"][..]] {
            assert_eq!(check_dsl(source, flags).unwrap(), expected);
        }
    }
}

#[test]
fn updates_read_old_state_and_omitted_state_is_preserved() {
    let source = "spec Swap {
        state x: Int = 1; state y: Int = 2; state marker: Bool = true;
        action swap { next x = y; next y = x; }
        invariant Sum { x + y == 3 && x != y && marker }
        property VisitsBoth { always(eventually(x == 1)) }
        fair weak swap;
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 0);
}

#[test]
fn choice_explores_all_values_and_empty_domains_disable_actions() {
    let source = "spec Choices {
        enum Color { Red, Blue }
        state color: Color = Red;
        action select { pick c in Color; next color = c; }
        invariant OnlyRed { color == Red }
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 1);
    let source = "spec Empty { state x: Int = 0; action empty { pick n in 1..=0; next x = n; } }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 1);
    assert_eq!(check_dsl(source, &["--no-deadlock"]).unwrap(), 0);
}

#[test]
fn limits_static_errors_and_configuration_are_distinct() {
    let source = "spec Counter { state x: Int = 0; action increment { next x = x + 1; } }";
    assert_eq!(check_dsl(source, &["--max-states", "4"]).unwrap(), 3);
    assert!(matches!(
        check_dsl(&source.replace("x + 1", "true"), &[]),
        Err(Error::Dsl { .. })
    ));
    assert!(matches!(
        check_dsl(source, &["--config", "ignored.cfg"]),
        Err(Error::Usage(_))
    ));
    assert_eq!(check_dsl(source, &["--check"]).unwrap(), 0);
}

#[test]
fn exports_are_portable_and_do_not_overwrite_files() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.tsl");
    std::fs::write(
        &source,
        include_str!("../../examples/17-modern-dsl/Retry.tsl"),
    )
    .unwrap();
    let output = dir.path().join("export");
    dsl::export(&source, &output).unwrap();
    assert_eq!(
        run(vec![output.join("Retry.tla").display().to_string()]).unwrap(),
        0
    );
    let expected = std::fs::read(output.join("Retry.tla")).unwrap();
    assert!(dsl::export(&source, &output).is_err());
    assert_eq!(std::fs::read(output.join("Retry.tla")).unwrap(), expected);
    std::fs::remove_file(output.join("Retry.tla")).unwrap();
    assert!(dsl::export(&source, &output).is_err());
    assert!(
        !output.join("Retry.tla").exists(),
        "a cfg collision must clean up the new tla file"
    );
}

#[test]
fn safety_traces_point_back_to_dsl_actions() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("choice.tsl");
    std::fs::write(
        &source,
        include_str!("../../examples/17-modern-dsl/Choice.tsl"),
    )
    .unwrap();
    let generated = dsl::generate(&source).unwrap();
    let loaded = tlaplus_checker::load(&generated.path, None, &[], None).unwrap();
    let Bound::Ready(mut checker) = tlaplus_checker::bind(loaded, Settings::default()).unwrap() else {
        panic!("ready model")
    };
    generated.annotate(&mut checker.model, &source);
    assert!(checker.model.checks.actions[0].0.starts_with("select ("));
    assert!(
        checker.model.checks.actions[0]
            .0
            .contains("choice.tsl:8:12")
    );
}

#[test]
fn multiple_initial_states_and_stuttering_action_properties_are_preserved() {
    let source = "spec Initials {
        state values: Seq<Int> in sequences(0..=1, 2);
        action stay {}
        invariant Distinct { values[1] != values[2] }
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 1);
    let source = "spec StepProperties {
        state x: Int = 0; state marker: Bool = false;
        action change { next marker = !marker; }
        // A false predicate is allowed when x stutters, even if marker changes.
        transition Stutter over (x) { false }
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 0);
    assert_eq!(
        check_dsl(
            &source.replace(
                "next marker = !marker;",
                "next marker = !marker; next x = x + 1;"
            ),
            &[]
        )
        .unwrap(),
        1
    );
}

#[test]
fn parameter_fairness_is_per_instance_not_aggregate() {
    let source = "spec Progress {
        const Workers: Set<Int> = 1..=2;
        state done: Map<Int, Bool> = map(w in Workers) { false };
        action finish(w in Workers) { require !done[w]; next done = put(done, w, true); }
        action stay {}
        fair weak finish;
        property Completed { forall(w in Workers) { eventually(done[w]) } }
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 0);
    assert_eq!(
        check_dsl(&source.replace("fair weak finish;", ""), &[]).unwrap(),
        1
    );
}

#[test]
fn module_imports_are_local_deduplicated_and_cycle_checked() {
    let dir = tempfile::tempdir().unwrap();
    for (file, source) in [
        (
            "Common",
            "module Common { fn identity(n: Int) -> Int { n } }",
        ),
        (
            "Left",
            "module Left { import Common; fn left(n: Int) -> Int { identity(n) } }",
        ),
        (
            "Right",
            "module Right { import Common; fn right(n: Int) -> Int { identity(n) } }",
        ),
        (
            "Root",
            "spec Root { import Left; import Right; state x: Int = left(1); action stay {} invariant Same { x == right(1) } }",
        ),
    ] {
        std::fs::write(dir.path().join(format!("{file}.tsl")), source).unwrap();
    }
    let root = dir.path().join("Root.tsl");
    assert_eq!(run(vec![root.display().to_string()]).unwrap(), 0);
    std::fs::write(
        dir.path().join("Common.tsl"),
        "module Common { import Left; }",
    )
    .unwrap();
    assert!(
        dsl::compile(&root)
            .unwrap_err()
            .to_string()
            .contains("cyclic module import")
    );
    std::fs::write(
        dir.path().join("Common.tsl"),
        "module Common { state x: Int = 0; }",
    )
    .unwrap();
    assert!(
        dsl::compile(&root)
            .unwrap_err()
            .to_string()
            .contains("cannot contain state")
    );
}

#[test]
fn zero_argument_recursive_functions_export_without_empty_formal_lists() {
    let source = "spec Zero {
        recursive fn zero() -> Int { if true { 0 } else { zero() } }
        assume zero() == 0;
        state x: Int = zero(); action stay {}
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 0);
}

#[test]
fn refinements_check_initial_states_and_updates_without_filtering_them() {
    let source = "spec Refined {
        type Disabled = Bool where value { !value }
        state enabled: Disabled = false;
        action stay {}
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 0);
    for invalid in [
        source.replace("= false;", "= true;"),
        source.replace("action stay {}", "action enable { next enabled = true; }"),
        source.replace("= false;", "in {false, true};"),
    ] {
        assert_eq!(check_dsl(&invalid, &[]).unwrap(), 1);
    }
    let guarded = source.replace(
        "action stay {}",
        "action enable { require true is Disabled; next enabled = true; }",
    );
    assert_eq!(check_dsl(&guarded, &["--no-deadlock"]).unwrap(), 0);
}

#[test]
fn refinement_domains_are_total_and_compose() {
    let source = "spec Refined {
        enum Person { Alice, Bob }
        type Everyone = Map<Person, Bool> where assignments { true }
        type Disabled = Bool where value { !value }
        type NobodyEnabled = Map<Person, Disabled> where assignments { true }
        type Crew = Set<Person> where people { contains(people, Alice) }
        state all: Everyone = map(p in Person) { false };
        state none: NobodyEnabled in NobodyEnabled;
        state crew: Crew in Crew;
        action stay {}
        invariant Complete { domain(all) == Person && domain(none) == Person }
        invariant Safe { forall(p in Person) { !none[p] } && contains(crew, Alice) }
    }";
    assert_eq!(check_dsl(source, &[]).unwrap(), 0);
    assert_eq!(
        check_dsl(
            &source.replace("map(p in Person) { false }", "entry(Alice, false)"),
            &[]
        )
        .unwrap(),
        1
    );
    assert_eq!(
        check_dsl(
            &source.replace("state crew: Crew in Crew;", "state crew: Crew = {};"),
            &[]
        )
        .unwrap(),
        1
    );
}

#[test]
fn imported_refinements_keep_state_obligations() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Types.tsl"),
        "module Types { type Disabled = Bool where value { !value } }",
    )
    .unwrap();
    let path = dir.path().join("Root.tsl");
    for (initial, code) in [("false", 0), ("true", 1)] {
        std::fs::write(&path, format!("spec Root {{ import Types; state enabled: Disabled = {initial}; action stay {{}} }}")).unwrap();
        assert_eq!(run(vec![path.display().to_string()]).unwrap(), code);
    }
}
