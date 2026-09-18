use std::cmp::Ordering;
use tla_values::{Error, SetExpr, Value};

#[test]
fn lazy_sets_and_extensional_functions() {
    let base = SetExpr::Interval(-2, 2).materialize(5).unwrap();
    let subsets = SetExpr::subsets(&base).unwrap();
    assert_eq!(subsets.cardinality(), Ok(32));
    let set = subsets.materialize(32).unwrap();
    for subset in set.elements().unwrap() {
        assert!(subsets.contains(subset).unwrap());
    }
    assert_eq!(
        subsets.iter().unwrap().collect::<Vec<_>>(),
        set.elements().unwrap()
    );
    assert_eq!(subsets.materialize(31), Err(Error::Limit));
    let domain = SetExpr::Interval(1, 2).materialize(2).unwrap();
    let range = Value::set(vec![Value::Bool(false), Value::Bool(true)]).unwrap();
    let functions = SetExpr::functions(&domain, &range).unwrap();
    assert_eq!(functions.cardinality(), Ok(4));
    for f in functions.iter().unwrap() {
        assert!(functions.contains(&f).unwrap());
    }
    let product = SetExpr::product(&[range.clone(), range]).unwrap();
    assert_eq!(product.materialize(4), functions.materialize(4));
    assert_eq!(
        Value::tuple(vec![Value::Int(7)]),
        Value::function(vec![(Value::Int(1), Value::Int(7))]).unwrap()
    );
    assert_eq!(
        SetExpr::Interval(i32::MAX, i32::MAX)
            .iter()
            .unwrap()
            .collect::<Vec<_>>(),
        vec![Value::Int(i32::MAX)]
    );
    assert_eq!(
        SetExpr::Interval(i32::MIN, i32::MAX).cardinality(),
        Ok(4_294_967_296)
    );
    assert_eq!(SetExpr::Interval(3, 2).cardinality(), Ok(0));
    assert_eq!(
        SetExpr::functions(&Value::set(vec![]).unwrap(), &Value::set(vec![]).unwrap())
            .unwrap()
            .cardinality(),
        Ok(1)
    );
}

#[test]
fn ordinal_order_and_persistent_updates() {
    let z = Value::string("z", 0);
    let a = Value::string("a", 1);
    let ordered = Value::set(vec![a.clone(), z.clone(), a.clone()]).unwrap();
    assert_eq!(ordered.elements().unwrap(), &[z.clone(), a.clone()]);
    assert_eq!(ordered.choose(|_| Ok(true)), Ok(z.clone()));
    assert_eq!(z.compare(&a), Ok(Ordering::Less));
    let original = Value::tuple(vec![ordered.clone()]);
    let updated = original.except(&[Value::Int(1)], a.clone()).unwrap();
    assert_eq!(updated.apply(&Value::Int(1)), Ok(a));
    assert_eq!(original.apply(&Value::Int(1)), Ok(ordered));
    assert_eq!(
        Value::Int(0).semantic_eq(&Value::Bool(false)),
        Err(Error::Incomparable)
    );
    assert_eq!(
        Value::model("A_x", 2).semantic_eq(&Value::model("B_x", 3)),
        Err(Error::Incomparable)
    );
}
