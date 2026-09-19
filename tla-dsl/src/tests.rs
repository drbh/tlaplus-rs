//! Compiler contracts, including rejected ambiguous or ill-typed specifications.
use super::*;

fn model(body: &str) -> Result<Artifact, Error> {
    compile(&format!("spec Sample {{ state x: Int = 0; {body} }}"))
}

#[test]
fn choice_is_existential_and_unwritten_state_is_preserved() {
    let artifact =
        model("state y: Bool = false; action select { pick n in 0..=2; next x = n; }").unwrap();
    assert!(artifact.tla.contains("\\E n \\in (0 .. 2)"));
    assert!(artifact.tla.contains("UNCHANGED <<y>>"));
    assert!(!artifact.tla.contains("CHOOSE"));
    for origin in artifact.origins {
        assert!(
            artifact
                .tla
                .lines()
                .nth(origin.generated_line - 1)
                .unwrap()
                .starts_with(&origin.name)
        );
    }
}

#[test]
fn types_effects_and_names_are_checked() {
    for (body, expected) in [
        ("action step { next x = true; }", "expected Int"),
        ("action step { next x = missing; }", "unknown value"),
        (
            "action step { next x = 1; next x = 2; }",
            "duplicate update",
        ),
        (
            "action step { require eventually(x == 0); }",
            "temporal expressions",
        ),
        ("state y: Int = x; action step {}", "unknown value"),
        (
            "action step {} invariant Bad { eventually(x == 0) }",
            "temporal expressions",
        ),
        ("action step {} property Bad { x == 0 }", "use invariant"),
        ("action step {} fair weak absent;", "unknown action"),
        ("action step { pick x in 0..=1; }", "duplicate identifier"),
        (
            "action step { require true; pick n in 0..=1; }",
            "must precede",
        ),
        ("state y: Missing = 0; action step {}", "unknown type"),
        ("action Next {}", "reserved identifier"),
        ("action PROVE {}", "reserved identifier"),
        ("action step { next x = 2147483648; }", "32-bit"),
        (
            "action step {} fair weak step; fair strong step;",
            "duplicate fairness",
        ),
        ("enum Empty {} action step {}", "at least one variant"),
    ] {
        let error = model(body).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
}

#[test]
fn enums_are_nominal_and_choice_domains_are_scoped() {
    let source = "enum Color { Red, Blue } enum Mode { On, Off } state color: Color = Red;";
    assert!(
        model(&format!(
            "{source} action select {{ pick c in Color; next color = c; }}"
        ))
        .is_ok()
    );
    assert!(
        model(&format!("{source} action select {{ next color = On; }}"))
            .unwrap_err()
            .message
            .contains("expected Enum")
    );
    assert!(model("action select { pick lo in 0..=1; pick hi in lo..=2; next x = hi; }").is_ok());
}

#[test]
fn diagnostics_have_original_locations() {
    let error =
        compile("// Unicode comment: λ\nspec Sample {\n  state x: Bool = 3;\n  action step {}\n}")
            .unwrap_err();
    assert_eq!(
        error.location,
        Location {
            line: 3,
            column: 19
        }
    );
    assert!(compile("spec Sample { state x: Int = 0; action step {} } trailing").is_err());
    assert!(model("action step { next x = (1 + ); }").is_err());
}

#[test]
fn precedence_and_fairness_survive_export() {
    let artifact = model("action step { next x = 1 + 2 * 3; } fair strong step; property Live { always(eventually(x == 7)) }").unwrap();
    assert!(artifact.tla.contains("x' = (1 + (2 * 3))"));
    assert!(artifact.tla.contains("SF_vars(step)"));
    assert!(artifact.config.contains("PROPERTY Live"));
    assert!(artifact.config.contains("INVARIANT TypeOK"));
}

#[test]
fn excessive_expressions_return_diagnostics() {
    let expression = std::iter::repeat_n("1", 300)
        .collect::<Vec<_>>()
        .join(" + ");
    assert!(
        model(&format!("action step {{ next x = {expression}; }}"))
            .unwrap_err()
            .message
            .contains("256")
    );
    let expression = format!("{}1{}", "(".repeat(140), ")".repeat(140));
    assert!(
        model(&format!("action step {{ next x = {expression}; }}"))
            .unwrap_err()
            .message
            .contains("nesting")
    );
}

#[test]
fn collection_and_action_levels_are_checked_before_export() {
    for (body, expected) in [
        ("state s: Seq<Int> = [true]; action step {}", "expected Seq"),
        (
            "state m: Map<Int, Bool> = map(i in 1..=2) { true }; action step { next m = put(m, 1, 3); }",
            "invalid arguments for put",
        ),
        (
            "state s: Set<Int> = {}; action step { next s = union(s, {true}); }",
            "invalid arguments for union",
        ),
        ("action step { next x = after(x); }", "after(...) is only"),
        (
            "action step {} invariant Bad { after(x) == x }",
            "after(...) is only",
        ),
        (
            "action step {} property Bad { always(after(x) == x) }",
            "use transition",
        ),
        (
            "action step {} property Bad { always(true) && after(x) == x }",
            "not mixed",
        ),
        (
            "action step {} transition Bad over (missing) { after(x) == x }",
            "must name state",
        ),
        ("action step(t in 0..=x) {}", "must be constant"),
        (
            "action a(t in 1..=2) {} action b(t in 1..=3) {} fair weak { a, b };",
            "identical parameter",
        ),
        (
            "fn f(n: Int) -> Bool { n + 1 } action step {}",
            "expected Bool",
        ),
        (
            "fn f(n: Int) -> Int { x + n } action step {}",
            "unknown value",
        ),
        (
            "fn f(n: Int) -> Int { f(n) } action step {}",
            "cyclic definition",
        ),
        (
            "recursive fn f(op: Fn<Int, Int>) -> Int { op(0) } action step {}",
            "cannot take operator",
        ),
        (
            "action step {} invariant Bad { forall(i in 0..=x) { eventually(x == i) } }",
            "constant domains",
        ),
        (
            "state s: Seq<Int> = sequence(i in 0..=3) { i }; action step {}",
            "1..=length",
        ),
        (
            "action step {} assume (1, true)[3] == 0;",
            "tuple index out of range",
        ),
    ] {
        let error = model(body).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
}

#[test]
fn empty_collections_unify_without_making_nonempty_collections_untyped() {
    let source = "state values: Set<Seq<Int>> = {[], [1, 2]}; action step {} invariant Fact { contains(values, []) }";
    assert!(model(source).is_ok());
    assert!(model(&source.replace("[1, 2]", "[1, true]")).is_err());
    assert!(model("action step {} assume contains(product(1..=2, {true}), (1, true));").is_ok());
}

#[test]
fn library_modules_are_distinct_from_models() {
    let artifact = compile("module Point { fn origin() -> Seq<Int> { [0, 0] } }").unwrap();
    assert!(artifact.config.is_empty());
    assert!(!artifact.tla.contains("VARIABLES"));
    assert!(compile("module Point { state x: Int = 0; }").is_err());
    assert!(
        model("import Point; action step {}")
            .unwrap_err()
            .message
            .contains("compile_file")
    );
}

#[test]
fn refinements_reject_invalid_predicates_domains_and_operator_contracts() {
    for (body, expected) in [
        (
            "type Small = Missing where n { true } action stay {}",
            "unknown type",
        ),
        (
            "type Small = Bool where n { 1 } action stay {}",
            "incompatible operands",
        ),
        (
            "type Small = Bool where n { x == 0 } action stay {}",
            "unknown value",
        ),
        (
            "type Small = Bool where n { eventually(n) } action stay {}",
            "temporal expressions",
        ),
        (
            "type Small = Bool where n { !n } state s: Small = 1; action stay {}",
            "expected Refined",
        ),
        (
            "action stay {} invariant Bad { 1 is true }",
            "must be a value set",
        ),
        (
            "action stay {} invariant Bad { 1 is {true} }",
            "incompatible value type",
        ),
        (
            "action stay {} property Bad { eventually(true) is {true} }",
            "incompatible value type",
        ),
        (
            "type Same = Bool where n { true } type Same = Bool where n { false } action stay {}",
            "duplicate identifier",
        ),
        (
            "type Small = Int where n { n < 3 } state s: Small in Small; action stay {}",
            "explicit domain",
        ),
        (
            "domain Missing in 0..=2; action stay {}",
            "must name a refinement",
        ),
        (
            "type Small = Int where n { n < 3 } domain Small in {true}; action stay {}",
            "expected Refined",
        ),
        (
            "type Bad = Fn<Int, Int> where f { true } action stay {}",
            "not operators",
        ),
        (
            "type Small = Int where n { n < 3 } fn bad(op: Fn<Small, Int>) -> Int { 0 } action stay {}",
            "operator signatures",
        ),
    ] {
        let error = model(body).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
}

#[test]
fn composition_rejects_ambiguous_or_cyclic_definitions() {
    for (body, expected) in [
        ("action step { call step(); }", "cyclic"),
        ("action step { call missing(); }", "unknown action"),
        (
            "action helper(p in 0..=1) {} action step { call helper(); }",
            "expects 1 arguments",
        ),
        (
            "action helper { next x = 1; } action step { call helper(); next x = 2; }",
            "duplicate update",
        ),
        ("action step {} next { missing }", "declared actions"),
        ("action step {} next { step, step }", "duplicate action"),
        (
            "action helper {} action step {} next { step } fair weak helper;",
            "selected by next",
        ),
        (
            "init { next x = 0; } action step {}",
            "either declaration initializers",
        ),
        (
            "view current() -> Int { x } fn bad() -> Int { current() } action step {}",
            "pure definitions",
        ),
        (
            "fn a() -> Int { b() } fn b() -> Int { a() } action step {}",
            "cyclic definition",
        ),
    ] {
        let error = model(body).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
    for (body, expected) in [
        ("state x: Int; action stay {}", "explicit init"),
        (
            "state x: Int; state y: Int; init { next x = 0; } action stay {}",
            "every state",
        ),
        (
            "state x: Int; init { next x = 0; next x = 1; } action stay {}",
            "duplicate init",
        ),
        (
            "state x: Int; init { next x = x; } action stay {}",
            "unknown value",
        ),
    ] {
        let error = compile(&format!("spec InitProbe {{ {body} }}")).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
}

#[test]
fn records_are_nominal_complete_and_checked_before_export() {
    for (expression, expected) in [
        ("Message { code: 1 }", "every struct field"),
        (
            "Message { code: 1, valid: true, extra: 0 }",
            "unknown struct field",
        ),
        (
            "Message { code: 1, code: 2, valid: true }",
            "duplicate struct field",
        ),
        ("Message { code: true, valid: true }", "expected Int"),
        ("Other { code: 1, valid: true }", "expected Record"),
    ] {
        let body = format!(
            "struct Message {{ code: Int, valid: Bool }} struct Other {{ code: Int, valid: Bool }} state msg: Message = {expression}; action stay {{}}"
        );
        let error = model(&body).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
    assert!(model("struct Node { link: Node } action stay {}").is_err());
    assert!(model("struct Empty {} action stay {}").is_err());
}
