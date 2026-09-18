use std::num::NonZeroUsize;
use tla_runtime::Registers;
use tla_values::Value;

#[test]
fn initialization_broadcasts_and_workers_keep_separate_values() {
    let registers = Registers::default();
    registers.start(NonZeroUsize::new(3).unwrap()).unwrap();
    let index = Value::Int(i32::MAX);
    let name = Value::string("s:count", 1);
    for key in [&index, &name] {
        registers.set(None, key, Value::Int(10)).unwrap();
    }
    std::thread::scope(|scope| {
        for worker in 0..3 {
            let (registers, name, index) = (&registers, &name, &index);
            scope.spawn(move || {
                for key in [index, name] {
                    assert_eq!(registers.get(Some(worker), key).unwrap(), Value::Int(10));
                    registers
                        .set(Some(worker), key, Value::Int(worker as i32))
                        .unwrap();
                }
            });
        }
    });
    for worker in 0..3 {
        for key in [&index, &Value::string("s:count", 999)] {
            assert_eq!(
                registers.get(Some(worker), key).unwrap(),
                Value::Int(worker as i32)
            );
        }
    }
    assert_eq!(registers.get(None, &index).unwrap(), Value::Int(0));
    registers.set(None, &name, Value::Int(7)).unwrap();
    for worker in 0..3 {
        assert_eq!(registers.get(Some(worker), &name).unwrap(), Value::Int(7));
    }
}

#[test]
fn run_boundaries_and_clones_do_not_leak_registers() {
    let registers = Registers::default();
    let index = Value::Int(0);
    assert!(registers.set(None, &index, Value::Int(1)).is_err());
    let name = Value::string("s:setup", 1);
    assert_eq!(
        registers.set(None, &name, Value::Int(1)),
        Ok(Value::Bool(true))
    );
    assert!(registers.get(None, &name).is_err());
    registers.start(NonZeroUsize::MIN).unwrap();
    registers.set(None, &index, Value::Int(9)).unwrap();
    assert!(registers.clone().get(None, &index).is_err());
    registers.start(NonZeroUsize::MIN).unwrap();
    assert_eq!(
        registers.get(None, &index),
        Err(tla_runtime::Error::Undefined("0".into()))
    );
    assert!(registers.set(Some(1), &index, Value::Int(1)).is_err());
    assert!(registers.get(Some(1), &index).is_err());
    assert!(
        registers
            .get(None, &Value::Int(-1))
            .unwrap_err()
            .to_string()
            .contains("nonnegative integer")
    );
    assert!(
        registers
            .set(None, &Value::Bool(true), Value::Int(1))
            .unwrap_err()
            .to_string()
            .contains("first argument")
    );
    assert_eq!(
        registers.get(None, &Value::string("nosuch", 2)),
        Err(tla_runtime::Error::Undefined("nosuch".into()))
    );
    assert!(
        registers
            .get(None, &Value::string("stats", 3))
            .unwrap_err()
            .to_string()
            .starts_with("unsupported")
    );
}

#[test]
fn defaults_apply_only_to_absent_values_and_nonregister_keys() {
    let registers = Registers::default();
    let fallback = Value::Bool(false);
    for key in [
        Value::Int(0),
        Value::Int(-1),
        Value::string("s:missing", 1),
        Value::Bool(true),
    ] {
        assert_eq!(
            registers.get_or_default(None, &key, fallback.clone()),
            Ok(fallback.clone())
        );
    }
    registers.start(NonZeroUsize::MIN).unwrap();
    let name = Value::string("s:name", 1);
    for key in [Value::Int(9), name] {
        registers.set(None, &key, Value::Int(7)).unwrap();
        assert_eq!(
            registers.get_or_default(None, &key, fallback.clone()),
            Ok(Value::Int(7))
        );
    }
    assert!(
        registers
            .get_or_default(None, &Value::Int(-1), fallback.clone())
            .unwrap_err()
            .to_string()
            .ends_with("length 10")
    );
    assert_eq!(
        registers.get_or_default(None, &Value::string("stats", 2), fallback.clone()),
        Ok(fallback)
    );
}

#[test]
fn setup_snapshots_survive_runs_clones_and_worker_count_changes() {
    let mut registers = Registers::default();
    registers
        .begin_setup(NonZeroUsize::new(4).unwrap())
        .unwrap();
    let key = Value::Int(0);
    registers.set(None, &key, Value::Int(7)).unwrap();
    registers.capture_setup().unwrap();
    let snapshot = registers.setup_snapshot();
    registers.set(Some(2), &key, Value::Int(99)).unwrap();
    assert!(registers.capture_setup().is_err());
    assert_eq!(registers.setup_snapshot(), snapshot);
    for mut copy in [registers.clone(), Registers::default()] {
        copy.restore_setup(snapshot.clone());
        for workers in [1, 4, 2] {
            copy.configure(NonZeroUsize::new(workers).unwrap());
            copy.reset().unwrap();
            for worker in 0..workers {
                assert_eq!(copy.get(Some(worker), &key), Ok(Value::Int(7)));
                copy.set(Some(worker), &key, Value::Int(99)).unwrap();
            }
        }
    }
    registers.begin_setup(NonZeroUsize::MIN).unwrap();
    registers.capture_setup().unwrap();
    registers.reset().unwrap();
    assert!(registers.get(None, &key).is_err());
}

#[test]
fn all_register_views_use_worker_zero_keys_and_preserve_named_ordinals() {
    let registers = Registers::default();
    let indexed = Value::string("all", 1);
    let named = Value::string("all:named", 2);
    for key in [&indexed, &named] {
        assert!(
            registers
                .get(None, key)
                .unwrap_err()
                .to_string()
                .ends_with("was undefined.")
        );
    }
    registers.start(NonZeroUsize::new(3).unwrap()).unwrap();
    assert_eq!(
        registers.get(None, &indexed),
        Ok(Value::function(vec![]).unwrap())
    );
    let name = Value::string("s:count", 42);
    for worker in 0..3 {
        for key in [&Value::Int(7), &name] {
            registers
                .set(Some(worker), key, Value::Int(worker as i32))
                .unwrap();
        }
    }
    registers
        .set(Some(2), &Value::Int(99), Value::Bool(true))
        .unwrap();
    registers
        .set(Some(2), &Value::string("s:other", 43), Value::Bool(true))
        .unwrap();
    let tuple = Value::tuple(vec![Value::Int(0), Value::Int(1), Value::Int(2)]);
    for (query, key) in [(&indexed, Value::Int(7)), (&named, name)] {
        for worker in [None, Some(0), Some(2)] {
            assert_eq!(
                registers.get(worker, query).unwrap(),
                Value::function(vec![(key.clone(), tuple.clone())]).unwrap()
            );
        }
    }
    for (query, key) in [
        (&indexed, Value::Int(9)),
        (&named, Value::string("s:missing", 44)),
    ] {
        registers.set(Some(0), &key, Value::Bool(true)).unwrap();
        assert_eq!(
            registers.get(None, query),
            Err(tla_runtime::Error::Tlc(
                "java.lang.NullPointerException".into()
            ))
        );
    }
}

#[test]
fn caches_preserve_setup_isolate_sites_and_retry_failed_computations() {
    use std::cell::Cell;
    use tla_runtime::Caches;
    let mut caches = Caches::default();
    let count = Cell::new(0);
    let metadata = |site| {
        Value::tuple(vec![
            Value::Int(site),
            Value::function(vec![(Value::Int(0), Value::Int(9))]).unwrap(),
        ])
    };
    let evaluate = |caches: &Caches, site, key, fail| {
        caches.active.evaluate(
            &metadata(site),
            Value::Int(key),
            0x9114_98ae_0e66_bad6,
            || {
                count.set(count.get() + 1);
                if fail {
                    Err(tla_library::Error("failure".into()))
                } else {
                    Ok(Value::Int(count.get()))
                }
            },
        )
    };
    assert_eq!(evaluate(&caches, 1, 0, false), Ok(Value::Int(9)));
    assert_eq!(count.get(), 0);
    assert_eq!(evaluate(&caches, 1, 1, false), Ok(Value::Int(1)));
    assert_eq!(evaluate(&caches, 1, 1, false), Ok(Value::Int(1)));
    assert_eq!(evaluate(&caches, 2, 1, false), Ok(Value::Int(2)));
    assert_eq!(
        evaluate(&caches, 1, 2, true),
        Err(tla_runtime::Error::Evaluation(tla_library::Error(
            "failure".into()
        )))
    );
    assert_eq!(evaluate(&caches, 1, 2, false), Ok(Value::Int(4)));
    caches = caches.checkpoint().unwrap();
    evaluate(&caches, 1, 3, false).unwrap();
    caches.capture_setup().unwrap();
    for copy in [caches.clone(), caches] {
        copy.reset().unwrap();
        assert_eq!(evaluate(&copy, 1, 3, false), Ok(Value::Int(5)));
        copy.begin_setup().unwrap();
        assert_eq!(evaluate(&copy, 1, 1, false), Ok(Value::Int(1)));
        let before = count.get();
        assert_eq!(evaluate(&copy, 1, 3, false), Ok(Value::Int(before + 1)));
    }
}
