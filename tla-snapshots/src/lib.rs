use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error, MapAccess, SeqAccess, Visitor, value::MapAccessDeserializer},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    hash::{BuildHasher, Hash},
    sync::Arc,
};

pub type SharedValues<K> = Arc<Vec<K>>;
pub type Samples<K> = Vec<(Vec<u8>, BTreeSet<SharedValues<K>>)>;

pub fn intern_values<K: Eq + Hash, S: BuildHasher>(
    pool: &mut HashSet<SharedValues<K>, S>,
    values: Vec<K>,
) -> SharedValues<K> {
    if let Some(existing) = pool.get(&values) {
        return existing.clone();
    }
    let values = Arc::new(values);
    pool.insert(values.clone());
    values
}

#[derive(Serialize, Deserialize)]
struct Compact<K> {
    values: Vec<SharedValues<K>>,
    states: Vec<(Vec<u8>, Vec<usize>)>,
}

pub fn serialize<K: Serialize + Eq + Hash, S: Serializer>(
    samples: &Samples<K>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut values = Vec::new();
    let mut indices = HashMap::new();
    let states = samples
        .iter()
        .map(|(state, successors)| {
            let successors = successors
                .iter()
                .map(|value| {
                    *indices.entry(value.clone()).or_insert_with(|| {
                        values.push(value.clone());
                        values.len() - 1
                    })
                })
                .collect();
            (state.clone(), successors)
        })
        .collect();
    Compact { values, states }.serialize(serializer)
}

pub fn deserialize<'de, K: Deserialize<'de> + Ord + Hash, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Samples<K>, D::Error> {
    struct Encoding<K>(std::marker::PhantomData<K>);
    impl<'de, K: Deserialize<'de> + Ord + Hash> Visitor<'de> for Encoding<K> {
        type Value = Samples<K>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("indexed snapshots or a legacy snapshot array")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut pool = HashSet::<SharedValues<K>>::new();
            let mut samples = Vec::new();
            while let Some((state, successors)) =
                input.next_element::<(Vec<u8>, BTreeSet<SharedValues<K>>)>()?
            {
                let successors = successors
                    .into_iter()
                    .map(|value| {
                        pool.get(&value).cloned().unwrap_or_else(|| {
                            pool.insert(value.clone());
                            value
                        })
                    })
                    .collect();
                samples.push((state, successors));
            }
            Ok(samples)
        }
        fn visit_map<A: MapAccess<'de>>(self, input: A) -> Result<Self::Value, A::Error> {
            let compact = Compact::<K>::deserialize(MapAccessDeserializer::new(input))?;
            compact
                .states
                .into_iter()
                .map(|(state, successors)| {
                    let successors = successors
                        .into_iter()
                        .map(|index| {
                            compact.values.get(index).cloned().ok_or_else(|| {
                                A::Error::custom(format!("unknown successor value index {index}"))
                            })
                        })
                        .collect::<Result<_, _>>()?;
                    Ok((state, successors))
                })
                .collect()
        }
    }
    deserializer.deserialize_any(Encoding(std::marker::PhantomData))
}
