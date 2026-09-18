use tla_library::{BagOp, JsonMode, bag, json, stringify};
use tla_value_expr::{Expr, FunctionKind};
use tla_values::Value;
#[test]
fn integer_equality_reports_nested_offending_values() {
    use tla_ir::Builtin as B;
    assert_eq!(
        tla_library::apply(
            &Expr::tuple(vec![Expr::atom(Value::Int(0))]),
            &Expr::atom(Value::string("a", 0)),
        )
        .unwrap_err()
        .0,
        "Attempted to check equality of integer 1 with non-integer:\n\"a\""
    );
    for op in [B::Eq, B::Ne] {
        for nested in [false, true] {
            let wrap = |v| if nested { Value::tuple(vec![v]) } else { v };
            let error = tla_library::apply_builtin(
                op,
                vec![wrap(Value::Int(1)), wrap(Value::string("a", 0))],
            )
            .unwrap_err();
            assert_eq!(
                error.0,
                "Attempted to check equality of integer 1 with non-integer:\n\"a\""
            );
        }
        assert_eq!(
            tla_library::apply_builtin(op, vec![Value::Int(1), Value::model("m", 0)]).unwrap(),
            Value::Bool(op == B::Ne)
        );
    }
}
#[test]
fn numeric_comparisons_reject_nonintegers_in_operand_order() {
    use tla_ir::Builtin as B;
    for (op, name) in [(B::Lt, "<"), (B::Le, "<="), (B::Gt, ">"), (B::Ge, ">=")] {
        for invalid in [
            Value::Bool(false),
            Value::string("a", 0),
            Value::tuple(vec![Value::Int(1)]),
        ] {
            for (position, args) in [
                ("first", vec![invalid.clone(), invalid.clone()]),
                ("second", vec![Value::Int(0), invalid.clone()]),
            ] {
                assert_eq!(
                    tla_library::apply_builtin(op, args).unwrap_err().0,
                    format!(
                        "The {position} argument of {name} should be an integer, but instead it is:\n{}",
                        stringify(&Expr::atom(invalid.clone())).unwrap()
                    )
                );
            }
        }
        for (a, b) in [(i32::MIN, i32::MAX), (0, 0), (1, -1)] {
            let expected = match op {
                B::Lt => a < b,
                B::Le => a <= b,
                B::Gt => a > b,
                _ => a >= b,
            };
            assert_eq!(
                tla_library::apply_builtin(op, vec![Value::Int(a), Value::Int(b)]).unwrap(),
                Value::Bool(expected)
            );
        }
    }
}
#[test]
fn infinite_membership_errors_preserve_the_value_and_set() {
    use tla_value_expr::InfiniteSet;
    assert_eq!(
        tla_library::contains(
            &Expr::atom(Value::string("a", 0)),
            &Expr::atom(Value::Int(2))
        )
        .unwrap_err()
        .0,
        "Attempted to check if the value:\n2\nis an element of the string \"a\""
    );
    for (kind, name, invalid) in [
        (InfiniteSet::Naturals, "Nat", Value::string("a", 0)),
        (InfiniteSet::Integers, "Int", Value::string("a", 0)),
        (InfiniteSet::Strings, "STRING", Value::Int(1)),
    ] {
        let value = Expr::atom(invalid);
        assert_eq!(
            tla_library::contains(&Expr::Infinite(kind), &value)
                .unwrap_err()
                .0,
            format!(
                "Attempted to check if the value:\n{}\nis an element of {name}.",
                stringify(&value).unwrap()
            )
        );
    }
    for (kind, value, expected) in [
        (InfiniteSet::Naturals, Value::Int(-1), false),
        (InfiniteSet::Naturals, Value::Int(0), true),
        (InfiniteSet::Integers, Value::Int(-1), true),
        (InfiniteSet::Strings, Value::string("a", 0), true),
        (InfiniteSet::Any, Value::Bool(false), true),
    ] {
        assert_eq!(
            tla_library::contains(&Expr::Infinite(kind), &Expr::atom(value)).unwrap(),
            expected
        );
    }
}
#[test]
fn bags_preserve_counts_and_reject_nonbags() {
    let a = Value::function(vec![(Value::Int(1), Value::Int(2))]).unwrap();
    let b = Value::function(vec![
        (Value::Int(1), Value::Int(1)),
        (Value::Int(2), Value::Int(1)),
    ])
    .unwrap();
    let sum = bag(BagOp::Add, &[a.clone(), b.clone()]).unwrap();
    assert_eq!(sum.apply(&Value::Int(1)).unwrap(), Value::Int(3));
    assert_eq!(bag(BagOp::Difference, &[sum.clone(), b]).unwrap(), a);
    assert_eq!(bag(BagOp::Subset, &[a.clone(), sum]), Ok(Value::Bool(true)));
    assert!(bag(BagOp::Add, &[a, Value::Int(1)]).is_err());
    assert_eq!(
        bag(BagOp::Union, &[Value::set(vec![Value::Int(4)]).unwrap()]),
        Ok(Value::Int(4))
    );
}
#[test]
fn json_and_tla_rendering_preserve_record_tuple_shapes() {
    let tuple = Expr::tuple(vec![
        Expr::atom(Value::Int(1)),
        Expr::atom(Value::Bool(true)),
    ]);
    assert_eq!(json(&tuple, JsonMode::Value).unwrap(), "[1,true]");
    assert_eq!(
        json(&tuple, JsonMode::Object).unwrap(),
        r#"{"0":1,"1":true}"#
    );
    assert_eq!(stringify(&tuple).unwrap(), "<<1, TRUE>>");
    let record = Expr::Function {
        kind: FunctionKind::Record,
        entries: vec![(
            Value::string("a", 0),
            Expr::atom(Value::string("quote\"", 1)),
        )],
    };
    assert_eq!(
        json(&record, JsonMode::Value).unwrap(),
        r#"{"a":"quote\""}"#
    );
    assert!(json(&record, JsonMode::Array).is_err());
    assert_eq!(
        json(
            &Expr::Function {
                kind: FunctionKind::Record,
                entries: vec![]
            },
            JsonMode::Value
        )
        .unwrap(),
        "{}"
    );
    assert_eq!(json(&Expr::tuple(vec![]), JsonMode::Value).unwrap(), "[]");
}

#[test]
fn canonical_rendering_matches_faithful_values() {
    tla_bridge::checker::with_value_session(|| {
        let words = ["a", "word space", "quote\"", "line\nbreak", "tab\t", "α"];
        let atoms = words
            .iter()
            .map(|s| tla_bridge::checker::intern_value(s, false).unwrap())
            .collect::<Vec<_>>();
        let mut seed = 0xabc621efu64;
        for _ in 0..10_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = (seed % 6) as usize;
            let value = match seed % 5 {
                0 => Value::Int(seed as i32),
                1 => atoms[n].clone(),
                2 => Value::tuple((0..n).map(|i| atoms[i].clone()).collect()),
                3 => Value::set((0..n).map(|i| Value::Int(i as i32)).collect()).unwrap(),
                _ => Value::function(
                    (0..n)
                        .map(|i| (Value::Int(i as i32 - 2), atoms[i].clone()))
                        .collect(),
                )
                .unwrap(),
            };
            let faithful = tla_bridge::checker::to_faithful(&value).unwrap();
            assert_eq!(
                stringify(&Expr::atom(value.clone())).unwrap(),
                faithful.to_string_checked().unwrap(),
                "{value:?}"
            );
        }
    });
}

#[test]
fn model_values_reuse_string_atoms_and_validate_arguments() {
    use tla_ir::Builtin;
    use tla_library::apply_builtin;
    for name in ["", "mv", "k_0", "k_1"] {
        let text = Value::string(name, 17);
        let value = apply_builtin(Builtin::ModelValue, vec![text]).unwrap();
        assert_eq!(value, Value::model(name, 17));
    }
    let a = apply_builtin(Builtin::ModelValue, vec![Value::string("k_0", 3)]).unwrap();
    let b = apply_builtin(Builtin::ModelValue, vec![Value::string("k_1", 4)]).unwrap();
    assert!(!a.semantic_eq(&b).unwrap());
    assert!(a.semantic_eq(&Value::Int(0)).is_err());
    for args in [
        vec![],
        vec![Value::Int(1)],
        vec![Value::Bool(true)],
        vec![Value::string("x", 1), Value::Int(0)],
    ] {
        assert!(apply_builtin(Builtin::ModelValue, args).is_err());
    }
}

#[test]
fn anti_function_inverts_injective_functions() {
    use tla_ir::Builtin;
    let function = Value::function(vec![
        (Value::string("a", 0), Value::Int(1)),
        (Value::string("b", 1), Value::Int(2)),
    ])
    .unwrap();
    let inverse = tla_library::apply_builtin(Builtin::AntiFunction, vec![function]).unwrap();
    assert_eq!(
        inverse.apply(&Value::Int(1)).unwrap(),
        Value::string("a", 0)
    );
    assert_eq!(
        inverse.apply(&Value::Int(2)).unwrap(),
        Value::string("b", 1)
    );
    let duplicate = Value::function(vec![
        (Value::Int(0), Value::Int(1)),
        (Value::Int(1), Value::Int(1)),
    ])
    .unwrap();
    assert!(tla_library::apply_builtin(Builtin::AntiFunction, vec![duplicate]).is_err());
}

#[test]
fn assertion_errors_preserve_the_specification_message() {
    let error = tla_library::apply_builtin(
        tla_ir::Builtin::Assert,
        vec![Value::Bool(false), Value::string("transition rejected", 1)],
    )
    .unwrap_err();
    assert_eq!(error.0, "assertion failed: \"transition rejected\"");
}

#[test]
fn arithmetic_domain_errors_preserve_the_invalid_operand() {
    use tla_ir::Builtin as B;
    use tla_library::apply_builtin;
    let calculate = |op, a, b| apply_builtin(op, vec![Value::Int(a), Value::Int(b)]);
    assert_eq!(calculate(B::Div, 1, 0).unwrap_err().0, "division by zero");
    for divisor in [0, -2, i32::MIN] {
        assert_eq!(
            calculate(B::Mod, 1, divisor).unwrap_err().0,
            format!("non-positive modulus: {divisor}")
        );
    }
    assert_eq!(calculate(B::Div, -7, 3).unwrap(), Value::Int(-3));
    assert_eq!(calculate(B::Div, 7, -3).unwrap(), Value::Int(-3));
    assert_eq!(calculate(B::Mod, -7, 3).unwrap(), Value::Int(2));
    assert_ne!(
        calculate(B::Div, i32::MIN, -1).unwrap_err().0,
        "division by zero"
    );
    assert_ne!(
        apply_builtin(B::Div, vec![Value::Bool(false), Value::Int(0)])
            .unwrap_err()
            .0,
        "division by zero"
    );
}
