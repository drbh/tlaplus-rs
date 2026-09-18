use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use tla_bridge::values::{from_faithful, intern_value, to_faithful, with_value_session};
use tla_values::Value;

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
