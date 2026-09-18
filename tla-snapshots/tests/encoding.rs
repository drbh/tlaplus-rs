use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    hash::{BuildHasherDefault, Hasher},
    sync::Arc,
};
use tla_snapshots::{Samples, intern_values};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
enum Value {
    String(String),
    Model(String),
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Snapshot {
    #[serde(
        serialize_with = "tla_snapshots::serialize",
        deserialize_with = "tla_snapshots::deserialize"
    )]
    samples: Samples<Value>,
}

#[derive(Default)]
struct Collision;
impl Hasher for Collision {
    fn finish(&self) -> u64 {
        0
    }
    fn write(&mut self, _: &[u8]) {}
}

#[test]
fn interning_uses_full_values_even_when_hashes_collide() {
    let mut pool = HashSet::with_hasher(BuildHasherDefault::<Collision>::default());
    let value = vec![Value::String("same".into())];
    let first = intern_values(&mut pool, value.clone());
    let repeated = intern_values(&mut pool, value);
    assert!(Arc::ptr_eq(&first, &repeated));
    let distinct = intern_values(&mut pool, vec![Value::Model("same".into())]);
    assert_ne!(first, distinct);
    assert_eq!(pool.len(), 2);
}

#[test]
fn compact_roundtrip_preserves_states_sets_and_sharing() {
    let a = Arc::new(vec![Value::String("a".repeat(100))]);
    let b = Arc::new(vec![Value::Model("a".repeat(100))]);
    let original = Snapshot {
        samples: vec![
            (vec![1, 7], [a.clone(), b.clone()].into()),
            (vec![2, 8], [a.clone()].into()),
            (vec![3, 9], [b].into()),
            (vec![4, 10], [].into()),
        ],
    };
    let compact = serde_json::to_string(&original).unwrap();
    assert_eq!(serde_json::to_string(&original).unwrap(), compact);
    let encoded: serde_json::Value = serde_json::from_str(&compact).unwrap();
    assert_eq!(encoded["samples"]["values"].as_array().unwrap().len(), 2);
    let decoded: Snapshot = serde_json::from_str(&compact).unwrap();
    assert_eq!(decoded, original);
    let first = decoded.samples[0].1.get(&a).unwrap();
    let repeated = decoded.samples[1].1.get(&a).unwrap();
    assert!(Arc::ptr_eq(first, repeated));
    let legacy = serde_json::to_string(&serde_json::json!({"samples": original.samples})).unwrap();
    let legacy_decoded: Snapshot = serde_json::from_str(&legacy).unwrap();
    assert_eq!(legacy_decoded, original);
    assert!(Arc::ptr_eq(
        legacy_decoded.samples[0].1.get(&a).unwrap(),
        legacy_decoded.samples[1].1.get(&a).unwrap()
    ));
    assert!(compact.len() < legacy.len());
}

#[test]
fn malformed_references_and_unknown_formats_are_rejected() {
    for samples in [
        serde_json::json!({"values": [], "states": [[[1], [0]]]}),
        serde_json::json!({"values": [], "states": [[[1], [-1]]]}),
        serde_json::json!({"values": [], "states": "invalid"}),
        serde_json::json!({"other": []}),
    ] {
        assert!(
            serde_json::from_value::<Snapshot>(serde_json::json!({"samples": samples})).is_err()
        );
    }
}
