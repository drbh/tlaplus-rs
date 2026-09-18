use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use tla_bridge::checker::{from_faithful, intern_value, to_faithful, with_value_session};
use tla_values::{Error, SetExpr, Value};

fn random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}
fn value(seed: &mut u64, atoms: &[Value], depth: usize) -> Value {
    let n = random(seed);
    if depth == 0 {
        return atoms[n as usize % atoms.len()].clone();
    }
    match n % 5 {
        0 => Value::Int(n as i32),
        1 => atoms[n as usize % atoms.len()].clone(),
        2 => Value::set(
            (0..n % 5)
                .map(|_| Value::Int((random(seed) % 11) as i32 - 5))
                .collect(),
        )
        .unwrap(),
        3 => Value::tuple((0..n % 4).map(|_| value(seed, atoms, depth - 1)).collect()),
        _ => Value::set(
            (0..n % 4)
                .map(|_| Value::tuple(vec![value(seed, atoms, depth - 1)]))
                .collect(),
        )
        .unwrap_or_else(|_| Value::set(vec![]).unwrap()),
    }
}
fn hash(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn one_million_faithful_value_cases() {
    with_value_session(|| {
        let mut atoms = vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Int(i32::MIN),
            Value::Int(i32::MAX),
        ];
        for (name, model) in [
            ("z", false),
            ("a", false),
            ("m", true),
            ("n", true),
            ("A_a", true),
            ("A_b", true),
            ("B_c", true),
        ] {
            atoms.push(intern_value(name, model).unwrap());
        }
        let mut seed = 0x4531a072d9b8u64;
        for iteration in 0..1_000_000 {
            let a = value(&mut seed, &atoms, 2);
            let b = if iteration % 5 == 0 {
                a.clone()
            } else {
                value(&mut seed, &atoms, 2)
            };
            let faithful_a = to_faithful(&a).unwrap();
            let faithful_b = to_faithful(&b).unwrap();
            let restored = from_faithful(&faithful_a).unwrap();
            assert_eq!(a, restored);
            assert_eq!(hash(&a), hash(&restored));
            assert_eq!(
                a.semantic_eq(&b).ok(),
                faithful_a.equals(&faithful_b).ok(),
                "equality {iteration}: {a:?} {b:?}"
            );
            assert_eq!(
                a.compare(&b).ok(),
                faithful_a.compare_to(&faithful_b).map(|n| n.cmp(&0)).ok(),
                "order {iteration}: {a:?} {b:?}"
            );
            if let Value::Set(_) = &a {
                assert_eq!(a.cardinality().unwrap() as i32, faithful_a.size().unwrap());
                assert_eq!(
                    a.contains(&b).ok(),
                    faithful_a.member(&faithful_b).ok(),
                    "membership {iteration}: {a:?} {b:?}"
                );
                let mut enumeration = faithful_a.elements().unwrap();
                let all = enumeration
                    .all()
                    .unwrap()
                    .iter()
                    .map(from_faithful)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(a.choose(|_| Ok(true)).ok(), all.first().cloned());
                if let Some(first) = all.first() {
                    let expected = all.iter().find(|v| *v != first).cloned();
                    assert_eq!(a.choose(|v| Ok(v != first)).ok(), expected);
                }
                assert_eq!(a, Value::set(all).unwrap());
            }
        }
    });
}

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
