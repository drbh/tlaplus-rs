use super::*;
use tla_modules::{config, load};

#[test]
fn configuration_and_lowering() {
    let config = config(
        "CONSTANTS N = 3 S = {a, b}\nINIT Init\nNEXT Next\nINVARIANT Inv\nCHECK_DEADLOCK FALSE",
    )
    .unwrap();
    assert_eq!(config.constants.len(), 2);
    let module = tla_syntax::module("---- MODULE C ----\nCONSTANTS N,S\nVARIABLE x\nInit == x = 0\nNext == x' = (x + 1) % N\nInv == x < N\n====").unwrap();
    let lowered = lower(&module, &config).unwrap();
    assert_eq!(lowered.bindings.len(), 2);
    assert_eq!(lowered.program.operators.len(), 3);
    assert_eq!(
        lowered.program.nodes[lowered.program.operators[1].body].level,
        tla_ir::Level::Action
    );
    for text in [
        "INIT A INIT B",
        "CHECK_DEADLOCK yes",
        "CONSTANT N = 1 N = 2",
        "SPECIFICATION S INIT I",
        "UNKNOWN X",
    ] {
        assert!(tla_modules::config(text).is_err(), "{text}");
    }
}

#[test]
fn names_arity_recursion_and_levels() {
    for body in [
        "Unknown",
        "F == G\nG == F",
        "F(x) == x\nG == F(1,2)",
        "F == x'\nASSUME F",
    ] {
        let source = if body == "Unknown" {
            "---- MODULE M ----\nF == Unknown\n====".into()
        } else {
            format!("---- MODULE M ----\nVARIABLE x\n{body}\n====")
        };
        let module = tla_syntax::module(&source).unwrap();
        let failed =
            lower(&module, &Config::default()).map_or(true, |lowered| !lowered.failures.is_empty());
        assert!(failed, "{body}");
    }
    let module = tla_syntax::module(
        "---- MODULE M ----\nRECURSIVE F(_)\nF(x) == IF x = 0 THEN 0 ELSE F(x - 1)\nG == F(2)\n====",
    );
    // The hand-written parser has no RECURSIVE; mark the declaration directly.
    let mut module = module.unwrap_or_else(|_| {
        tla_syntax::module(
            "---- MODULE M ----\nF(x) == IF x = 0 THEN 0 ELSE F(x - 1)\nG == F(2)\n====",
        )
        .unwrap()
    });
    module.definitions[0].recursive = true;
    let lowered = lower(&module, &Config::default()).unwrap();
    assert!(lowered.failures.is_empty());
    assert_eq!(
        lowered.program.nodes[lowered.program.operators[0].body].level,
        tla_ir::Level::Constant
    );
    module.definitions[0].recursive = false;
    let lowered = lower(&module, &Config::default()).unwrap();
    assert_eq!(
        lowered.failures.len(),
        2,
        "undeclared recursion fails F and G"
    );
}

#[test]
fn module_resolution_and_cycles() {
    let root = std::env::temp_dir().join(format!("rewrite-modules-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("A.tla"),
        "---- MODULE A ----\nEXTENDS B\nF == G + 1\n====",
    )
    .unwrap();
    std::fs::write(root.join("B.tla"), "---- MODULE B ----\nG == 1\n====").unwrap();
    let module = load(&root.join("A.tla"), &[]).unwrap();
    assert_eq!(module.definitions.len(), 2);
    assert!(lower(&module, &Config::default()).is_ok());
    std::fs::write(
        root.join("B.tla"),
        "---- MODULE B ----\nEXTENDS A\nG == 1\n====",
    )
    .unwrap();
    assert!(load(&root.join("A.tla"), &[]).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_operators_remain_private_after_extends() {
    let root = std::env::temp_dir().join(format!("rewrite-private-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("B.tla"),
        "---- MODULE B ----\nLOCAL Hidden == 1\nPublic == Hidden\n====",
    )
    .unwrap();
    std::fs::write(
        root.join("A.tla"),
        "---- MODULE A ----\nEXTENDS B\nF == Public\n====",
    )
    .unwrap();
    let module = load(&root.join("A.tla"), &[]).unwrap();
    assert!(lower(&module, &Config::default()).is_ok());
    std::fs::write(
        root.join("A.tla"),
        "---- MODULE A ----\nEXTENDS B\nF == Hidden\n====",
    )
    .unwrap();
    let module = load(&root.join("A.tla"), &[]).unwrap();
    assert!(
        lower(&module, &Config::default())
            .unwrap()
            .failures
            .contains_key("F")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assert_error_retains_source_literal_syntax_and_user_shadowing() {
    let module = tla_syntax::module(
        r#"---- MODULE M ----
CONSTANT Message
A == AssertError("literal", TRUE)
B == AssertError(Message, TRUE)
C == AssertError("a" \o "b", TRUE)
===="#,
    )
    .unwrap();
    let lowered = lower(&module, &config("CONSTANT Message = \"literal\"").unwrap()).unwrap();
    for (index, expected) in [Some("literal"), None, None].into_iter().enumerate() {
        let body = lowered.program.operators[index].body;
        let tla_ir::Expr::AssertError { message, .. } = &lowered.program.nodes[body].expr else {
            panic!("expected literal-aware error assertion");
        };
        assert_eq!(message.as_deref(), expected);
    }
    let module = tla_syntax::module(
        "---- MODULE M ----\nAssertError(e,b) == b\nA == AssertError(1,TRUE)\n====",
    )
    .unwrap();
    let lowered = lower(&module, &Config::default()).unwrap();
    assert!(
        !lowered
            .program
            .nodes
            .iter()
            .any(|node| matches!(node.expr, tla_ir::Expr::AssertError { .. }))
    );
}
