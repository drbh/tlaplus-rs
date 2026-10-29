use std::num::NonZeroUsize;
use tlaplus_disk::store::{Node, Store};

#[test]
fn exact_keys_queue_and_parents_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("states.redb");
    let cache = NonZeroUsize::new(256 << 10).unwrap();
    {
        let store = Store::open(&path, cache).unwrap();
        for i in 0..128u64 {
            let node = Node {
                state: vec![i as u8; 16 << 10],
                parent: (i != 0).then_some((i.saturating_sub(1), i)),
                depth: i + 1,
            };
            assert_eq!(store.insert(&i.to_le_bytes(), &node).unwrap(), (i, true));
            assert_eq!(store.insert(&i.to_le_bytes(), &node).unwrap(), (i, false));
        }
        assert_eq!(store.len().unwrap(), 128);
        for i in 0..64 {
            assert_eq!(store.pop().unwrap().unwrap().0, i);
        }
        assert_eq!(store.queued().unwrap(), 64);
        assert!(std::fs::metadata(&path).unwrap().len() > cache.get() as u64 * 8);
    }
    let store = Store::open(&path, cache).unwrap();
    for i in 64..128u64 {
        let (id, node) = store.pop().unwrap().unwrap();
        assert_eq!(id, i);
        assert_eq!(node.state, vec![i as u8; 16 << 10]);
        assert_eq!(store.find(&i.to_le_bytes()).unwrap(), Some(i));
    }
    assert!(store.pop().unwrap().is_none());
    let trace = store.trace(127).unwrap();
    assert_eq!(trace.len(), 128);
    assert_eq!(trace[0].1.parent, None);
    assert_eq!(trace[127].1.parent, Some((126, 127)));
}

#[test]
fn invalid_parent_is_atomic_and_empty_binary_keys_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("s.redb"), NonZeroUsize::new(4096).unwrap()).unwrap();
    assert!(
        store
            .insert(
                b"bad",
                &Node {
                    state: vec![],
                    parent: Some((0, 0)),
                    depth: 2
                }
            )
            .is_err()
    );
    assert!(store.is_empty().unwrap());
    for (i, key) in [b"".as_slice(), b"\0", b"\0\0", b"\xff"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            store
                .insert(
                    key,
                    &Node {
                        state: key.to_vec(),
                        parent: None,
                        depth: 1
                    }
                )
                .unwrap(),
            (i as u64, true)
        );
    }
    assert_eq!(store.len().unwrap(), 4);
}
