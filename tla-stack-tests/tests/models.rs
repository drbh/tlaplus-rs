use std::{num::NonZeroUsize, path::PathBuf};
use tla_bridge::{ModelRequest, Outcome, RunEnv, observe_faithful, with_model};
use tla_engine::{Completion, Options, explore, explore_parallel, validate_trace};

#[test]
fn serial_register_execution_and_replay_match_the_faithful_driver() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    for config in ["RuntimeRegisters", "RuntimeRegistersFailure"] {
        let metadir = directory
            .join("../target")
            .join(format!("runtime-registers-{}-{config}", std::process::id()));
        let expected = observe_faithful(
            [
                "-tool",
                "-workers",
                "1",
                "-seed",
                "1",
                "-fp",
                "0",
                "-metadir",
                metadir.to_str().unwrap(),
                "-config",
                config,
                "RuntimeRegisters",
            ]
            .map(str::to_owned)
            .to_vec(),
            RunEnv {
                user_dir: directory.clone(),
                ..RunEnv::default()
            },
        )
        .unwrap();
        let mut request = ModelRequest::new(directory.join("RuntimeRegisters.tla"));
        request.config = Some(directory.join(format!("{config}.cfg")));
        let model = with_model(request, |model| model.lower()).unwrap();
        let options = Options {
            check_deadlock: false,
            record_graph: true,
            ..Options::default()
        };
        for _ in 0..2 {
            let report = explore(&model, &options).unwrap();
            if config.ends_with("Failure") {
                assert_eq!(expected.summary.outcome, Outcome::InvariantViolated);
                let Completion::Violated(violation) = &report.completion else {
                    panic!("missing violation")
                };
                assert_eq!(violation.trace.states.len(), 3);
                let trace: Vec<_> = violation
                    .trace
                    .states
                    .iter()
                    .enumerate()
                    .map(|(index, state)| {
                        let tla_model::Value::Int(value) = state[0] else {
                            panic!("expected integer state")
                        };
                        use tla_bridge::values::ValueKey;
                        (
                            index as i64 + 1,
                            ValueKey::Function(vec![(
                                ValueKey::String("x".into()),
                                ValueKey::Int(value),
                            )]),
                        )
                    })
                    .collect();
                assert_eq!(expected.typed_states, Some(Ok(trace)));
                assert!(validate_trace(&model, violation).unwrap());
                let mut forged = violation.clone();
                forged.trace.states.last_mut().unwrap().values[0] = tla_model::Value::Int(99);
                assert!(!validate_trace(&model, &forged).unwrap());
            } else {
                assert_eq!(expected.summary.outcome, Outcome::Success);
                assert_eq!(report.completion, Completion::Complete);
                assert_eq!(report.nodes.len(), 4);
                assert_eq!(expected.summary.distinct, Some(report.nodes.len() as u64));
                assert_eq!(expected.summary.depth, Some(report.statistics.depth as u64));
            }
        }
        assert!(explore_parallel(&model, &options, NonZeroUsize::MIN).is_ok());
        assert!(explore(&model.clone(), &options).is_ok());
    }
}

#[test]
fn assumption_register_writes_survive_initialization_and_repeated_runs() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    let request = ModelRequest::new(directory.join("AssumedRegisters.tla"));
    let (mut model, (mut assumptions, expressions)) =
        tla_bridge::with_loaded_model(request, |model| {
            Ok((model.lower()?, model.lower_assumptions()?))
        })
        .unwrap();
    let ids: Vec<_> = expressions.iter().map(|(_, id)| *id).collect();
    assert_eq!(
        assumptions
            .setup_assumptions(&ids, NonZeroUsize::new(4).unwrap())
            .unwrap(),
        None
    );
    model
        .runtime
        .registers
        .restore_setup(assumptions.runtime.registers.setup_snapshot());
    let bridged = tla_bridge::with_register_model(
        ModelRequest::new(directory.join("AssumedRegisters.tla")),
        |faithful| {
            for _ in 0..2 {
                let report = explore(
                    &faithful.safety_adapter(),
                    &Options {
                        check_deadlock: false,
                        ..Options::default()
                    },
                )?;
                assert!(matches!(report.completion, Completion::Complete));
                assert_eq!(report.nodes.len(), 3);
            }
            faithful.lower()
        },
    )
    .unwrap();
    for workers in [1, 4] {
        let metadir = directory.join("../target").join(format!(
            "assumed-registers-{}-{workers}",
            std::process::id()
        ));
        let expected = observe_faithful(
            [
                "-tool",
                "-workers",
                &workers.to_string(),
                "-metadir",
                metadir.to_str().unwrap(),
                "AssumedRegisters",
            ]
            .map(str::to_owned)
            .to_vec(),
            RunEnv {
                user_dir: directory.clone(),
                ..RunEnv::default()
            },
        )
        .unwrap();
        assert_eq!(expected.summary.outcome, Outcome::Success);
        for run in [&model, &model.clone(), &model, &bridged, &bridged.clone()] {
            let report = explore_parallel(
                run,
                &Options {
                    check_deadlock: false,
                    ..Options::default()
                },
                NonZeroUsize::new(workers).unwrap(),
            )
            .unwrap();
            assert_eq!(report.completion, Completion::Complete);
            assert_eq!(report.nodes.len(), 3);
            assert_eq!(expected.summary.distinct, Some(report.nodes.len() as u64));
            assert_eq!(report.nodes[0].state.values, vec![tla_model::Value::Int(7)]);
        }
    }
}

#[test]
fn all_register_views_match_the_faithful_driver() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    let model = with_model(
        ModelRequest::new(directory.join("AllRegisters.tla")),
        |model| model.lower(),
    )
    .unwrap();
    for workers in [1, 4] {
        let metadir = directory
            .join("../target")
            .join(format!("all-registers-{}-{workers}", std::process::id()));
        let expected = observe_faithful(
            [
                "-tool",
                "-workers",
                &workers.to_string(),
                "-metadir",
                metadir.to_str().unwrap(),
                "AllRegisters",
            ]
            .map(str::to_owned)
            .to_vec(),
            RunEnv {
                user_dir: directory.clone(),
                ..RunEnv::default()
            },
        )
        .unwrap();
        assert_eq!(expected.summary.outcome, Outcome::Success);
        let report = explore_parallel(
            &model,
            &Options {
                check_deadlock: false,
                ..Options::default()
            },
            NonZeroUsize::new(workers).unwrap(),
        )
        .unwrap();
        assert_eq!(report.completion, Completion::Complete);
        assert_eq!(report.nodes.len(), 4);
        assert_eq!(expected.summary.distinct, Some(report.nodes.len() as u64));
        assert_eq!(expected.summary.depth, Some(report.statistics.depth as u64));
    }
}

#[test]
fn native_assumption_draws_seed_initialization_like_faithful_setup() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    for seed in [0, 1, 42, -1] {
        let mut request = ModelRequest::new(directory.join("RandomModel.tla"));
        request.seed = seed;
        let expected: Vec<Vec<tla_model::Value>> = with_model(request.clone(), |faithful| {
            faithful
                .initial_states()?
                .iter()
                .map(|state| state.detached_values())
                .collect()
        })
        .unwrap();
        let (mut model, (mut assumptions, expressions)) =
            tla_bridge::with_loaded_model(request, |faithful| {
                Ok((faithful.lower()?, faithful.lower_assumptions()?))
            })
            .unwrap();
        let ids: Vec<_> = expressions.iter().map(|(_, id)| *id).collect();
        for workers in [1, 4, 1] {
            assert_eq!(
                assumptions
                    .setup_assumptions(&ids, NonZeroUsize::new(workers).unwrap())
                    .unwrap(),
                None
            );
            model.runtime.random = assumptions.runtime.random.clone();
            for run in [&model, &model.clone(), &model] {
                use tla_engine::TransitionSystem;
                let actual: Vec<_> = run
                    .initial_states()
                    .unwrap()
                    .into_iter()
                    .map(|state| state.values)
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn faithful_driver_agrees_on_random_constant_assumption_and_initial_draws() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures");
    for (name, workers) in [
        ("RandomConstants", 1),
        ("RandomConstants", 4),
        ("CachedConstants", 1),
        ("CachedConstants", 4),
        ("ConstantDependencies", 1),
        ("ConstantDependencies", 4),
        ("ForwardConstantDependencies", 1),
        ("ForwardConstantDependencies", 4),
    ] {
        let metadir = directory.join("../target").join(format!(
            "random-constants-{}-{name}-{workers}",
            std::process::id()
        ));
        let expected = observe_faithful(
            [
                "-tool",
                "-seed",
                "1",
                "-workers",
                &workers.to_string(),
                "-metadir",
                metadir.to_str().unwrap(),
                name,
            ]
            .map(str::to_owned)
            .to_vec(),
            RunEnv {
                user_dir: directory.clone(),
                ..RunEnv::default()
            },
        )
        .unwrap();
        assert_eq!(expected.summary.outcome, Outcome::Success, "{expected:?}");
        assert_eq!(expected.summary.distinct, Some(1));
    }
}

#[test]
fn cached_setup_survives_faithful_inspection_native_runs_and_clones() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/CacheRegisters.tla");
    let (mut assumptions, expressions) =
        tla_bridge::with_loaded_model(ModelRequest::new(path.clone()), |faithful| {
            faithful.lower_assumptions()
        })
        .unwrap();
    let ids: Vec<_> = expressions.iter().map(|(_, id)| *id).collect();
    for _ in 0..2 {
        assert_eq!(
            assumptions
                .setup_assumptions(&ids, NonZeroUsize::MIN)
                .unwrap(),
            None
        );
        assert_eq!(
            assumptions
                .runtime
                .registers
                .get(None, &tla_model::Value::Int(1))
                .unwrap(),
            tla_model::Value::Int(2)
        );
    }
    let model = tla_bridge::with_register_model(ModelRequest::new(path), |faithful| {
        for _ in 0..2 {
            let report = explore(&faithful.safety_adapter(), &Options::default())?;
            assert!(matches!(report.completion, Completion::Complete));
            assert_eq!(report.nodes.len(), 1);
        }
        faithful.lower()
    })
    .unwrap();
    for workers in [1, 4, 1] {
        for model in [&model, &model.clone()] {
            let report = explore_parallel(
                model,
                &Options::default(),
                NonZeroUsize::new(workers).unwrap(),
            )
            .unwrap();
            assert_eq!(report.completion, Completion::Complete);
            assert_eq!(report.nodes.len(), 1);
            assert_eq!(
                model
                    .runtime
                    .registers
                    .get(None, &tla_model::Value::Int(1))
                    .unwrap(),
                tla_model::Value::Int(2)
            );
        }
    }
}
