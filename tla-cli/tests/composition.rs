//! End-to-end contracts for composable models, records, and predicate refinements.
use std::{
    path::Path,
    process::{Command, Output},
};
fn run(source: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Probe.tsl");
    std::fs::write(&path, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_tla"))
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
fn expect(source: &str, code: i32, message: &str) {
    let output = run(source);
    assert_eq!(output.status.code(), Some(code), "{}", text(&output));
    assert!(text(&output).contains(message), "{}", text(&output));
}
#[test]
fn predicates_do_not_shrink_to_the_exploration_domain() {
    let source = "spec Probe {
        type Percent = Int where n { n >= 0 && n <= limit() }
        fn limit() -> Int { 100 }
        domain Percent in 0..=2;
        state x: Percent = 50;
        action stay {}
        invariant Independent { x is Percent && size(Percent) == 3 }
    }";
    expect(source, 0, "Safety checks passed");
    expect(
        &source.replace("= 50;", "= 101;"),
        1,
        "Invariant TypeOK is violated",
    );
    expect(
        &source
            .replace("= 50;", "in Percent;")
            .replace("x is Percent &&", "x <= 2 &&"),
        0,
        "3 distinct states found (3 initial)",
    );
    expect(
        &source.replace("domain Percent in 0..=2;", ""),
        2,
        "explicit domain",
    );
    let no_domain = source
        .replace("domain Percent in 0..=2;", "")
        .replace(" && size(Percent) == 3", "");
    expect(&no_domain, 0, "Safety checks passed");
}
#[test]
fn contracts_check_calls_results_nested_state_and_constants() {
    let source = "spec Probe {
        type Small = Int where n { n >= 0 && n < 3 }
        fn bump(n: Small) -> Small { n + 1 }
        state x: Int = 0;
        action step { require x < 1; next x = bump(x); }
        action done { require x == 1; }
    }";
    expect(source, 0, "2 distinct states found");
    expect(
        &source.replace("n + 1", "n + 3"),
        2,
        "postcondition of bump",
    );
    expect(
        &source.replace("bump(x)", "bump(-1)"),
        2,
        "precondition of bump",
    );
    expect(
        "spec Probe { type Off = Bool where b { !b } state xs: Seq<Off> = [true]; action stay {} }",
        1,
        "TypeOK",
    );
    expect(
        "spec Probe { type Off = Bool where b { !b } const Bad: Off = true; state x: Int = 0; action stay {} }",
        1,
        "ASSUME",
    );
    expect(
        "spec Probe { enum Key { A, B } type Complete = Map<Key, Map<Key, Bool>> where m { true } state m: Complete = map(k in Key) { entry(A, false) }; action stay {} }",
        1,
        "TypeOK",
    );
}
#[test]
fn correlated_init_and_atomic_calls_preserve_all_writes() {
    let source = "spec Probe {
        state x: Int; state y: Int;
        init { pick seed in 0..=2; next x = seed; next y = seed; }
        view equal() -> Bool { x == y }
        action inc { require x < 3; next x = x + 1; }
        action both { call inc(); next y = y + 1; }
        action done { require x == 3; }
        next { both, done }
        invariant Same { equal() }
    }";
    expect(source, 0, "4 distinct states found (3 initial)");
    expect(
        &source.replace("next { both, done }", "next { inc, both, done }"),
        1,
        "Invariant Same is violated",
    );
    // The caller's pick must not be captured by the helper's quantified name.
    expect(
        "spec Probe {
        state x: Int = 0;
        action helper(p in 0..=2) { require forall(v in {1}) { p != v }; next x = p; }
        action step { pick v in {2}; call helper(v); }
        next { step }
        invariant Value { x == 0 || x == 2 }
    }",
        0,
        "2 distinct states found",
    );
}
#[test]
fn records_and_refined_record_contracts_preserve_fields() {
    expect(
        "spec Probe {
        struct Message { amount: Int, accepted: Bool }
        type Valid = Message where m { m.amount >= 0 }
        fn accept(m: Valid) -> Valid { Message { amount: m.amount, accepted: true } }
        state message: Valid = Message { amount: 2, accepted: false };
        action receive { next message = accept(message); }
        invariant Amount { message.amount == 2 }
    }",
        0,
        "2 distinct states found",
    );
}
#[test]
fn reliable_delivery_exercises_failures_and_an_abstract_projection() {
    let source = include_str!("../../examples/18-reliable-delivery/ReliableDelivery.tsl");
    expect(source, 0, "100 distinct states found");
    expect(
        &source.replace("fair strong retry;", ""),
        1,
        "Temporal property AllComplete violated",
    );
    expect(
        &source.replace("call apply_once(p);", ""),
        1,
        "Invariant MessagesAreSound is violated",
    );
    expect(
        &source.replace("-> Count { 1 }", "-> Count { n + 1 }"),
        2,
        "postcondition of applied_once",
    );
}
#[test]
fn imported_records_and_refinements_share_dependency_order() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Types.tsl"), "module Types { struct Message { number: Int } type Valid = Message where m { m.number >= lower() } fn lower() -> Int { 0 } }").unwrap();
    let path = dir.path().join("Root.tsl");
    std::fs::write(
        &path,
        "spec Root { import Types; state m: Valid = Message { number: 1 }; action stay {} }",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tla"))
        .arg(Path::new(&path))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
}

#[test]
fn action_calls_respect_domains_and_keep_choices_independent() {
    expect(
        "spec Probe {
        type Small = Int where n { n >= 0 && n < 3 }
        domain Small in {0};
        state x: Int = 0;
        action helper(p in Small) { next x = p; }
        action outside { call helper(1); }
        action idle {}
        next { outside, idle }
        invariant Unchanged { x == 0 }
    }",
        0,
        "1 distinct states found",
    );
    expect(
        "spec Probe {
        state x: Int = 0; state y: Int = 0;
        action first { pick n in 0..=1; next x = n; }
        action second { pick n in 0..=1; next y = n; }
        action both { call first(); call second(); }
        next { both }
    }",
        0,
        "4 distinct states found",
    );
}
