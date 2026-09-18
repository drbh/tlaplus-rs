use std::cmp::Ordering;
use tla_value_expr::{Error, Expr, FunctionKind, InfiniteSet, SetOp, Value};
fn int(n: i32) -> Expr {
    Expr::atom(Value::Int(n))
}
fn empty() -> Expr {
    Expr::Set(vec![])
}

#[test]
fn union_enumeration_checks_both_operands_before_equality() {
    let invalid = Expr::Binary(
        SetOp::Union,
        Box::new(Expr::Set(vec![int(1)])),
        Box::new(int(2)),
    );
    let expected =
        "Attempted to enumerate S \\cup T when S:\n{1}\nand T:\n2\nare not both enumerable";
    assert_eq!(invalid.enumerate(100).unwrap_err().to_string(), expected);
    assert_eq!(
        invalid.semantic_eq(&int(0)).unwrap_err().to_string(),
        expected
    );
    assert!(invalid.contains(&int(1)).unwrap());
    let infinite = Expr::Binary(
        SetOp::Union,
        Box::new(empty()),
        Box::new(Expr::Infinite(InfiniteSet::Naturals)),
    );
    assert!(matches!(
        infinite.enumerate(100),
        Err(Error::UnionEnumeration { .. })
    ));
    let finite = Expr::Binary(
        SetOp::Union,
        Box::new(Expr::Set(vec![int(1)])),
        Box::new(Expr::Set(vec![int(1), int(2)])),
    );
    assert!(
        finite
            .semantic_eq(&Expr::Set(vec![int(2), int(1)]))
            .unwrap()
    );
}

#[test]
fn record_and_product_enumeration_preserve_error_order() {
    let records = Expr::Records(vec![
        (
            tla_value_expr::Atom::new("a", 2),
            Expr::Infinite(InfiniteSet::Integers),
        ),
        (
            tla_value_expr::Atom::new("z", 1),
            Expr::Infinite(InfiniteSet::Strings),
        ),
    ]);
    assert_eq!(
        records.enumerate(10).unwrap_err(),
        Error::RecordEnumeration {
            field: "z".into(),
            value: "STRING".into()
        }
    );
    let product = Expr::Product(vec![
        Expr::Set(vec![int(0)]),
        Expr::Infinite(InfiniteSet::Integers),
    ]);
    assert_eq!(
        product.enumerate(10).unwrap_err(),
        Error::ProductEnumeration {
            index: 1,
            value: "Int".into()
        }
    );
    assert!(Expr::Product(vec![int(0), empty()]).enumerate(10).is_err());
    assert!(
        Expr::Product(vec![empty(), int(0)])
            .enumerate(10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn function_enumeration_keeps_the_failing_operand() {
    let functions = |domain, range| Expr::Functions {
        domain: Box::new(domain),
        range: Box::new(range),
    };
    let infinite = Expr::Infinite(InfiniteSet::Strings);
    for (domain, range, position) in [
        (infinite.clone(), Expr::Set(vec![int(0)]), "domain D"),
        (Expr::Set(vec![int(0)]), infinite.clone(), "range R"),
        (infinite.clone(), infinite.clone(), "domain D"),
    ] {
        assert_eq!(
            functions(domain, range).enumerate(10).unwrap_err(),
            Error::FunctionEnumeration {
                position,
                value: "STRING".into()
            }
        );
    }
    assert_eq!(
        functions(empty(), infinite.clone())
            .enumerate(10)
            .unwrap()
            .len(),
        1
    );
    assert!(
        functions(infinite, empty())
            .enumerate(10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn membership_forces_set_operations_in_order() {
    let interval = Expr::Interval(1, 3);
    let disjoint = Expr::Binary(
        SetOp::Difference,
        Box::new(interval.clone()),
        Box::new(interval),
    );
    assert_eq!(disjoint.normalize(3).unwrap(), Value::set(vec![]).unwrap());
    assert!(disjoint.contains(&empty()).is_err());
    let left_empty = Expr::Binary(SetOp::Intersection, Box::new(empty()), Box::new(int(42)));
    assert!(!left_empty.contains(&int(1)).unwrap());
    let left_bad = Expr::Binary(SetOp::Intersection, Box::new(int(42)), Box::new(empty()));
    assert!(left_bad.contains(&int(1)).is_err());
    assert!(left_bad.empty().unwrap());
}

#[test]
fn functions_keep_dispatch_and_defer_ranges() {
    let invalid = Expr::Binary(
        SetOp::Difference,
        Box::new(Expr::Interval(1, 1)),
        Box::new(Expr::Set(vec![Expr::atom(Value::string("bad", 1))])),
    );
    let tuple = Expr::tuple(vec![invalid]);
    assert!(!tuple.semantic_eq(&Expr::tuple(vec![])).unwrap());
    assert_eq!(tuple.size(), Ok(1));
    assert_eq!(tuple.cardinality(), Err(Error::Type("enumerable set")));
    assert!(tuple.normalize(10).is_err());
    let record = Expr::Function {
        kind: FunctionKind::Record,
        entries: vec![],
    };
    let function = Expr::Function {
        kind: FunctionKind::Explicit,
        entries: vec![(Value::Int(3), int(1))],
    };
    assert!(record.compare(&function).is_err());
    assert_eq!(
        record
            .normalize(10)
            .unwrap()
            .compare(&function.normalize(10).unwrap()),
        Ok(Ordering::Less)
    );
}

#[test]
fn finite_empty_sequences_and_infinite_order() {
    let naturals = Expr::Infinite(InfiniteSet::Naturals);
    let integers = Expr::Infinite(InfiniteSet::Integers);
    assert_eq!(naturals.compare(&integers), Ok(Ordering::Less));
    assert_eq!(naturals.semantic_eq(&integers), Ok(false));
    assert_eq!(
        naturals.semantic_eq(&Expr::Infinite(InfiniteSet::Strings)),
        Err(Error::Incomparable)
    );
    assert_eq!(Expr::Sequences(Box::new(empty())).finite(), Ok(true));
    assert_eq!(
        Expr::Sequences(Box::new(naturals.clone())).finite(),
        Ok(false)
    );
    let typed = Expr::atom(Value::model("A_x", 2));
    assert_eq!(naturals.semantic_eq(&typed), Ok(false));
    assert_eq!(typed.semantic_eq(&naturals), Err(Error::Incomparable));
}

#[test]
fn any_contains_values_without_enumerating_them() {
    let any = Expr::Infinite(InfiniteSet::Any);
    for value in [
        int(3),
        Expr::atom(Value::Bool(true)),
        Expr::tuple(vec![int(1), int(2)]),
        Expr::Infinite(InfiniteSet::Naturals),
        any.clone(),
    ] {
        assert!(any.contains(&value).unwrap());
    }
    assert!(!any.empty().unwrap());
    assert!(!any.finite().unwrap());
    assert!(any.enumerate(100).is_err());
    assert!(any.semantic_eq(&any).is_err());
    assert!(any.compare(&any).is_err());
}

#[test]
fn power_set_inclusion_compares_bases_without_enumeration() {
    let subsets = |base| Expr::Subsets {
        base: Box::new(base),
        size: None,
    };
    let small = subsets(Expr::Interval(1, 23));
    let large = subsets(Expr::Interval(1, 42));
    assert!(small.subset_of(&large, 1).unwrap());
    assert!(!large.subset_of(&small, 1).unwrap());
    assert!(subsets(small).subset_of(&subsets(large), 1).unwrap());
    assert!(
        subsets(Expr::Interval(2, 1))
            .subset_of(&subsets(Expr::Interval(1, 0)), 1)
            .unwrap()
    );
    let single = subsets(Expr::Interval(1, 1));
    assert!(
        single
            .subset_of(&subsets(Expr::Infinite(InfiniteSet::Integers)), 1)
            .unwrap()
    );
    let strings = subsets(Expr::Set(vec![Expr::atom(Value::string("a", 1))]));
    assert!(
        strings
            .subset_of(&subsets(Expr::Infinite(InfiniteSet::Integers)), 1)
            .is_err()
    );
}
