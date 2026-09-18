use super::*;

#[test]
fn comments_strings_and_offsets() {
    let tokens = lex("(* a (* b *) *)\n  x = \"a\\n\" \\* end").unwrap();
    assert_eq!((tokens[0].line, tokens[0].column), (2, 3));
    assert_eq!(expression("\"a\\n\"").unwrap(), Expr::String("a\n".into()));
    for source in ["(*", "\"abc", "\"\\q\"", "x =", "@"] {
        assert!(expression(source).is_err(), "{source}");
    }
}

#[test]
fn module_and_precedence() {
    let parsed = module("---- MODULE Counter ----\nEXTENDS Naturals\nCONSTANT N\nVARIABLE x\nInit == x = 0\nNext == /\\ x < N\n /\\ x' = x + 1\nInv == x \\in 0..N\n====").unwrap();
    assert_eq!(parsed.definitions.len(), 3);
    assert_eq!(parsed.variables, ["x"]);
    assert!(matches!(&parsed.definitions[1].body, Expr::Binary(op, _, _) if op == "/\\"));
    assert!(module("---- MODULE X ----\nA == 1\nA == 2\n====").is_err());
    assert!(module("---- MODULE X ----\nA == LET y == 1 IN y\n====").is_err());
}

#[test]
fn binders_and_function_application() {
    assert!(matches!(
        expression("\\E x \\in {1,2} : x = 2").unwrap(),
        Expr::Bound { .. }
    ));
    assert!(matches!(
        expression("[x \\in 1..3 |-> x * 2][2]").unwrap(),
        Expr::Index(_, _)
    ));
    assert!(matches!(
        expression("IF TRUE THEN <<1,2>> ELSE <<>>").unwrap(),
        Expr::If(_, _, _)
    ));
}
