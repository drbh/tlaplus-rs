use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn check(body: &str, cfg: &str, flags: &[&str]) -> Result<i32, String> {
    let dir = std::env::temp_dir().join(format!(
        "rewrite-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Model.tla");
    std::fs::write(&path, format!("---- MODULE Model ----\n{body}\n====")).unwrap();
    std::fs::write(dir.join("Model.cfg"), cfg).unwrap();
    let mut args: Vec<String> = flags.iter().map(|s| (*s).into()).collect();
    args.push(path.display().to_string());
    let result = run(args);
    std::fs::remove_dir_all(dir).unwrap();
    result
}

#[test]
fn complete_violation_deadlock_and_limit() {
    let body = "CONSTANT N\nVARIABLE x\nInit == x = 0\nNext == x' = (x + 1) % N\nInv == x < N\nBad == x < 2";
    let cfg = "CONSTANT N = 4\nINIT Init\nNEXT Next\nINVARIANT Inv";
    assert_eq!(check(body, cfg, &[]).unwrap(), 0);
    assert_eq!(check(body, cfg, &["--workers", "4"]).unwrap(), 0);
    assert_eq!(check(body, &cfg.replace("Inv", "Bad"), &[]).unwrap(), 1);
    assert_eq!(check(body, cfg, &["--max-states", "2"]).unwrap(), 3);
    assert_eq!(
        check(
            "VARIABLE x\nInit == x = 0\nNext == FALSE",
            "INIT Init\nNEXT Next",
            &[]
        )
        .unwrap(),
        1
    );
    assert_eq!(
        check(
            "VARIABLE x\nInit == x = 0\nNext == FALSE",
            "INIT Init\nNEXT Next",
            &["--no-deadlock"]
        )
        .unwrap(),
        0
    );
}

#[test]
fn continuing_reports_violations_after_complete_exploration() {
    let body =
        "VARIABLE x\nInit == x = 0\nNext == x' = (x + 1) % 4\nBad == x < 2\nStep == [][x' > x]_x";
    let cfg = "INIT Init\nNEXT Next\nINVARIANT Bad\nPROPERTY Step";
    for workers in ["1", "4"] {
        assert_eq!(
            check(body, cfg, &["--continue", "--workers", workers]).unwrap(),
            1
        );
    }
    assert_eq!(
        check(body, "INIT Init\nNEXT Next", &["--continue"]).unwrap(),
        0
    );
    assert!(check(body, cfg, &["--continue", "--disk", "unused"]).is_err());
}

#[test]
fn assume_register_values_seed_initial_and_worker_evaluation() {
    let body = "EXTENDS Integers, TLC\nVARIABLE x\nASSUME TLCSet(0, 7)\nASSUME TLCSet(\"s:seed\", {7})\nInit == x = TLCGet(0)\nNext == x < 9 /\\ TLCSet(0, x + 1) /\\ x' = TLCGet(0)\nInv == x \\in 7..9 /\\ TLCGet(\"s:seed\") = {7}";
    let config = "INIT Init\nNEXT Next\nINVARIANT Inv\nCHECK_DEADLOCK FALSE";
    for workers in ["1", "4"] {
        assert_eq!(check(body, config, &["--workers", workers]).unwrap(), 0);
    }
    assert_eq!(
        check(
            &format!("{body}\nASSUME FALSE\nASSUME TLCGet(99) = 0"),
            config,
            &[]
        )
        .unwrap(),
        1
    );
}

#[test]
fn assumptions_errors_and_config() {
    assert_eq!(
        check(
            "ASSUME FALSE\nVARIABLE x\nInit == x = 0\nNext == x' = x",
            "INIT Init\nNEXT Next",
            &[]
        )
        .unwrap(),
        1
    );
    assert!(
        check(
            "VARIABLE x\nInit == x = 0\nNext == x' = x",
            "INIT Init\nNEXT Next",
            &["--workers", "0"]
        )
        .is_err()
    );
    assert!(
        check(
            "VARIABLE x\nInit == x = 0\nNext == x' = x",
            "INIT Init\nNEXT Next",
            &["--unknown"]
        )
        .is_err()
    );
    assert!(
        check(
            "CONSTANT N\nVARIABLE x\nInit == x = N\nNext == x' = x",
            "INIT Init\nNEXT Next",
            &[]
        )
        .is_err()
    );
}

#[test]
fn temporal_stuttering_and_lassos() {
    let body = "VARIABLE x\nInit == x = 0\nNext == x' = 1 - x\nSpec == Init /\\ [][Next]_x\nSafe == [](x < 2)\nLive == <>(x = 1)";
    assert_eq!(
        check(body, "SPECIFICATION Spec\nPROPERTY Safe", &[]).unwrap(),
        0
    );
    assert_eq!(
        check(body, "SPECIFICATION Spec\nPROPERTY Live", &[]).unwrap(),
        1
    );
}

#[test]
fn weak_and_strong_fairness() {
    for fairness in ["WF_x(Next)", "SF_x(Next)"] {
        let body = format!(
            "VARIABLE x\nInit == x = 0\nNext == x' = 1 - x\nSpec == Init /\\ [][Next]_x /\\ {fairness}\nLive == <>(x = 1)"
        );
        assert_eq!(
            check(&body, "SPECIFICATION Spec\nPROPERTY Live", &[]).unwrap(),
            0
        );
    }
}

#[test]
fn lexical_let_scopes_and_records() {
    let body = "VARIABLE x\nInit == x = (LET A == 1 B == A IN LET C == 2 IN B)\nNext == x' = x\nInv == x = 1";
    assert_eq!(
        check(body, "INIT Init\nNEXT Next\nINVARIANT Inv", &[]).unwrap(),
        0
    );
    // SANY: "Operator A already defined or declared."
    let shadowed = body.replace("LET C == 2", "LET A == 2");
    assert!(check(&shadowed, "INIT Init\nNEXT Next\nINVARIANT Inv", &[]).is_err());
    let body =
        "VARIABLE x\nInit == x = [a |-> 1, b |-> 2]\nNext == x' = x\nInv == x.a = 1 /\\ x.b = 2";
    assert_eq!(
        check(body, "INIT Init\nNEXT Next\nINVARIANT Inv", &[]).unwrap(),
        0
    );
}

#[test]
fn case_quantification_and_except() {
    let body = "VARIABLE x\nInit == x = [a |-> 0]\nNext == x' = [x EXCEPT !.a = CASE @ = 0 -> 1 [] OTHER -> 0]\nInv == x.a \\in {y + 1 : y \\in {-1,0}}";
    assert_eq!(
        check(body, "INIT Init\nNEXT Next\nINVARIANT Inv", &[]).unwrap(),
        0
    );
}

#[test]
fn symmetry_and_disk_backend() {
    let body = "EXTENDS TLC\nCONSTANT S\nVARIABLE x\nInit == x \\in S\nNext == x' \\in S\nSym == Permutations(S)";
    assert_eq!(
        check(
            body,
            "CONSTANT S = {a,b,c}\nINIT Init\nNEXT Next\nSYMMETRY Sym",
            &[]
        )
        .unwrap(),
        0
    );
    let disk = std::env::temp_dir().join(format!("rewrite-cli-disk-{}.redb", std::process::id()));
    let disk_text = disk.to_string_lossy();
    let body = "VARIABLE x\nInit == x = 0\nNext == x' = 1 - x";
    assert_eq!(
        check(
            body,
            "INIT Init\nNEXT Next",
            &["--disk", &disk_text, "--disk-cache-bytes", "1048576"]
        )
        .unwrap(),
        0
    );
    assert!(check(body, "INIT Init\nNEXT Next", &["--disk", &disk_text]).is_err());
    std::fs::remove_file(disk).unwrap();
}

#[test]
fn instance_substitution_avoids_variable_capture() {
    let dir = std::env::temp_dir().join(format!("rewrite-instance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Inner.tla"),
        "---- MODULE Inner ----\nCONSTANT N\nF(y) == N + y\nG == \\A x \\in {0} : N = 1\n====",
    )
    .unwrap();
    std::fs::write(dir.join("Outer.tla"), "---- MODULE Outer ----\nVARIABLE x\nI == INSTANCE Inner WITH N <- x\nInit == x = 1\nNext == x' = x\nInv == I!F(2) = 3 /\\ I!G\n====").unwrap();
    std::fs::write(dir.join("Outer.cfg"), "INIT Init\nNEXT Next\nINVARIANT Inv").unwrap();
    assert_eq!(
        run(vec![dir.join("Outer.tla").display().to_string()]).unwrap(),
        0
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn action_properties_require_temporal_wrapper() {
    let body = "VARIABLE x\nInit == x = 0\nNext == x' = 1 - x\nNoChange == UNCHANGED x\nWrapped == [][NoChange]_x";
    assert!(check(body, "INIT Init\nNEXT Next\nPROPERTY NoChange", &[]).is_err());
    assert_eq!(
        check(body, "INIT Init\nNEXT Next\nPROPERTY Wrapped", &[]).unwrap(),
        1
    );
}

#[test]
fn fairness_reads_next_variables_through_a_named_tuple() {
    let body = "VARIABLES x, y\nInit == x = 0 /\\ y \\in 0..2\nAdvance == x' = (x+1)%3 /\\ y' = y\nNext == Advance \\/ (x' = x /\\ y' = (y+1)%3)\nvars == <<x,y>>\nSpec == Init /\\ [][Next]_vars /\\ WF_vars(Advance)\nLive == <> (x = 2)";
    assert_eq!(
        check(body, "SPECIFICATION Spec\nPROPERTY Live", &[]).unwrap(),
        0
    );
}

#[test]
fn tlc_level_tracks_bfs_depth_in_memory_and_on_disk() {
    let body = "EXTENDS TLC\nVARIABLES x, phase, lane\nInit == lane \\in 0..3 /\\ x = 0 /\\ phase = TLCGet(\"level\")\nNext == /\\ lane' = lane\n        /\\ x' = IF x < 3 THEN x + 1 ELSE x\n        /\\ phase' = IF x < 3 THEN TLCGet(\"level\") ELSE phase\nInv == phase = x /\\ TLCGet(\"level\") = x + 1\nBad == x >= 0 /\\ TLCGet(\"level\") < 4";
    let disk = std::env::temp_dir().join(format!(
        "rewrite-level-{}-{}.redb",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let disk_text = disk.to_string_lossy();
    for flags in [vec![], vec!["--workers", "4"], vec!["--disk", &disk_text]] {
        assert_eq!(
            check(body, "INIT Init\nNEXT Next\nINVARIANT Inv", &flags).unwrap(),
            0
        );
        if disk.exists() {
            std::fs::remove_file(&disk).unwrap();
        }
        assert_eq!(
            check(body, "INIT Init\nNEXT Next\nINVARIANT Bad", &flags).unwrap(),
            1
        );
        if disk.exists() {
            std::fs::remove_file(&disk).unwrap();
        }
    }
}

#[test]
fn standard_values_and_local_names() {
    for name in ["AnyMembership", "AnyShadow", "ModelValues"] {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(format!("{name}.tla"));
        assert_eq!(run(vec![path.display().to_string()]).unwrap(), 0);
    }
}

#[test]
fn assumption_random_draws_continue_into_initialization() {
    let body = "EXTENDS Integers, TLC\nVARIABLE x\nASSUME RandomElement({TRUE})\nInit == x = RandomElement(1..100)\nNext == FALSE\nInv == x = 42";
    let cfg = "INIT Init\nNEXT Next\nINVARIANT Inv\nCHECK_DEADLOCK FALSE";
    for workers in ["1", "4"] {
        assert_eq!(check(body, cfg, &["--workers", workers]).unwrap(), 0);
    }
}

#[test]
fn random_constant_overrides_share_the_assumption_and_initialization_stream() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/RandomConstants.tla");
    for workers in ["1", "4"] {
        assert_eq!(
            run(vec![
                "--workers".into(),
                workers.into(),
                path.display().to_string()
            ])
            .unwrap(),
            0
        );
    }
}

#[test]
fn constant_definitions_cache_in_source_order_and_skip_failed_evaluations() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/CachedConstants.tla");
    for workers in ["1", "4"] {
        assert_eq!(
            run(vec![
                "--workers".into(),
                workers.into(),
                path.display().to_string()
            ])
            .unwrap(),
            0
        );
    }
}

#[test]
fn effectful_overrides_are_reevaluated_in_dependent_bindings() {
    for name in ["ConstantDependencies", "ForwardConstantDependencies"] {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(format!("{name}.tla"));
        for workers in ["1", "4"] {
            assert_eq!(
                run(vec![
                    "--workers".into(),
                    workers.into(),
                    path.display().to_string()
                ])
                .unwrap(),
                0
            );
        }
    }
}

#[test]
fn cached_assumptions_survive_initialization_and_worker_runs() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/CacheRegisters.tla");
    for workers in ["1", "4", "1"] {
        assert_eq!(
            run(vec![
                "--workers".into(),
                workers.into(),
                path.display().to_string()
            ])
            .unwrap(),
            0
        );
    }
}

#[test]
fn recursion_higher_order_operators_and_standard_modules() {
    let body = r#"EXTENDS Integers, Sequences, FiniteSets, Bags, TLC
RECURSIVE Sum(_)
Sum(q) == IF q = <<>> THEN 0 ELSE Head(q) + Sum(Tail(q))
RECURSIVE IsEven(_), IsOdd(_)
IsEven(n) == IF n = 0 THEN TRUE ELSE IsOdd(n - 1)
IsOdd(n) == IF n = 0 THEN FALSE ELSE IsEven(n - 1)
RECURSIVE Fold(_, _, _)
Fold(F(_, _), acc, q) == IF q = <<>> THEN acc ELSE Fold(F, F(acc, Head(q)), Tail(q))
Add(a, b) == a + b
fact[n \in 0..5] == IF n = 0 THEN 1 ELSE n * fact[n - 1]
VARIABLE x
Init == x = [a |-> 0, b |-> EmptyBag]
Next == x' = [x EXCEPT !.a = (@ + 1) % 3, !.b = @ (+) SetToBag({x.a})]
Inv == /\ Sum(<<1, 2, 3>>) = 6 /\ IsEven(4) /\ ~IsOdd(4)
       /\ Fold(Add, 0, <<1, 2>>) = 3 /\ Fold(LAMBDA a, b : a * b, 1, <<2, 3>>) = 6
       /\ fact[5] = 120 /\ x \in [a : 0..2, b : {x.b}] /\ IsABag(x.b)
       /\ SelectSeq(<<1, 2, 3>>, LAMBDA e : e # 2) = <<1, 3>>
       /\ {<<m, n>> \in (1..2) \X (1..2) : m < n} = {<<1, 2>>}
       /\ [m \in 1..2, n \in 1..2 |-> m + n][2, 2] = 4"#;
    let cfg = "INIT Init\nNEXT Next\nINVARIANT Inv\nCONSTRAINT Small\n";
    let body = format!("{body}\nSmall == BagCardinality(x.b) < 3");
    assert_eq!(check(&body, cfg, &[]).unwrap(), 0);
}

#[test]
fn nested_and_parameterized_instances_with_quantified_fairness() {
    let body = r#"EXTENDS Naturals
---- MODULE Counter ----
EXTENDS Naturals
VARIABLE c
Small == c <= 2
Step == c' = c + 1
====
VARIABLE cnt
C(p) == INSTANCE Counter WITH c <- cnt[p]
D == INSTANCE Counter WITH c <- cnt[1]
Init == cnt = [p \in {1, 2} |-> 0]
Inc(p) == cnt[p] < 2 /\ cnt' = [cnt EXCEPT ![p] = cnt[p] + 1]
Next == \E p \in {1, 2} : Inc(p)
Spec == Init /\ [][Next]_cnt /\ \A p \in {1, 2} : WF_cnt(Inc(p))
Done == <>(\A p \in {1, 2} : cnt[p] = 2)
Bounded == [](\A p \in {1, 2} : C(p)!Small)
Stepped == [][Inc(1) => D!Step]_cnt"#;
    let cfg = "SPECIFICATION Spec\nPROPERTY Done\nPROPERTY Bounded\nPROPERTY Stepped\n";
    assert_eq!(check(body, cfg, &["--no-deadlock"]).unwrap(), 0);
    let weak = body.replace("\\A p \\in {1, 2} : WF_cnt", "\\A p \\in {1} : WF_cnt");
    assert_eq!(check(&weak, cfg, &["--no-deadlock"]).unwrap(), 1);
}

#[test]
fn function_sets_over_unenumerable_ranges_check_membership_lazily() {
    let body = "EXTENDS Naturals\nVARIABLE x\nInit == x = [a |-> 1]\nNext == x' = x\nInv == /\\ x \\in [{\"a\"} -> {n \\in Nat : n > 0}]\n       /\\ [a |-> 0] \\notin [{\"a\"} -> {n \\in Nat : n > 0}]\n       /\\ [b |-> 1] \\notin [{\"a\"} -> {n \\in Nat : n > 0}]";
    let cfg = "INIT Init\nNEXT Next\nINVARIANT Inv";
    assert_eq!(check(body, cfg, &["--no-deadlock"]).unwrap(), 0);
}

#[test]
fn a_spec_without_a_cfg_file_uses_its_config_section() {
    let dir = std::env::temp_dir().join(format!("rewrite-embedded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Embedded.tla");
    let spec = "---- MODULE Embedded ----\nVARIABLE x\nInit == x = 0\nNext == x' = 1 - x\nSmall == x < 1\n====\n";
    std::fs::write(
        &path,
        format!("{spec}---- CONFIG Embedded ----\nINIT Init\nNEXT Next\nINVARIANT Small\n====\n"),
    )
    .unwrap();
    assert_eq!(run(vec![path.display().to_string()]).unwrap(), 1);
    // A sibling .cfg still takes precedence, and a spec without either is an error.
    std::fs::write(dir.join("Embedded.cfg"), "INIT Init\nNEXT Next\n").unwrap();
    assert_eq!(run(vec![path.display().to_string()]).unwrap(), 0);
    std::fs::remove_file(dir.join("Embedded.cfg")).unwrap();
    std::fs::write(&path, spec).unwrap();
    assert!(run(vec![path.display().to_string()]).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
