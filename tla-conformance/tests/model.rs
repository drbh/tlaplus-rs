use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use tla_bridge::model::{ModelInfo, Stage, StateSnapshot};
use tla_bridge::recording as mp;
use tla_bridge::{ModelRequest, Outcome, RunEnv, run_faithful, with_model};

const COUNTER: &str = include_str!("../../tests/fixtures/Counter.tla");

fn fixture(module: &str, spec: &str, cfg: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(format!("{module}.tla")), spec).unwrap();
    fs::write(dir.path().join(format!("{module}.cfg")), cfg).unwrap();
    dir
}

fn snapshot(bindings: &[(&str, &str)]) -> StateSnapshot {
    StateSnapshot {
        values: bindings
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn counter_graph_has_exact_successors_and_matches_faithful_counts() {
    let dir = fixture("Counter", COUNTER, "INIT Init\nNEXT Next\nINVARIANT Safe");
    let (info, states, levels) =
        with_model(ModelRequest::new(dir.path().join("Counter.tla")), |model| {
            let mut states = model.initial_states()?;
            assert_eq!(states.len(), 1);
            assert_eq!(states[0].snapshot()?, snapshot(&[("x", "0")]));
            let mut index = 0;
            while index < states.len() {
                assert!(
                    states.len() <= 4,
                    "state equality failed to recognize the terminal loop"
                );
                let before = states[index].snapshot()?;
                let next = model.successors(&states[index])?;
                assert_eq!(
                    states[index].snapshot()?,
                    before,
                    "enumeration mutated its input"
                );
                assert_eq!(next.len(), 1);
                assert_eq!(next[0].state.level(), states[index].level() + 1);
                assert_eq!(
                    next[0].state.snapshot()?,
                    snapshot(&[("x", &(index + 1).min(3).to_string())])
                );
                assert!(model.info().actions.iter().any(|a| a.id == next[0].action));
                let found = states
                    .iter()
                    .map(|s| s.same_values(&next[0].state))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .any(|equal| equal);
                if !found {
                    states.push(next.into_iter().next().unwrap().state);
                }
                index += 1;
            }
            let levels: Vec<_> = states.iter().map(|s| s.level()).collect();
            let snapshots = states
                .iter()
                .map(|s| s.snapshot())
                .collect::<Result<BTreeSet<_>, _>>()?;
            Ok((model.info().clone(), snapshots, levels))
        })
        .unwrap();
    assert_eq!(info.variables, ["x"]);
    assert_eq!(info.invariants, ["Safe"]);
    assert_eq!(info.initial_predicates.len(), 1);
    assert!(info.has_next);
    assert_eq!(
        states,
        (0..4).map(|x| snapshot(&[("x", &x.to_string())])).collect()
    );
    assert_eq!(levels, [1, 2, 3, 4]);

    let args = [
        "-tool",
        "-workers",
        "1",
        "-fp",
        "0",
        "-seed",
        "1",
        "-metadir",
        dir.path().join("states").to_str().unwrap(),
        "Counter",
    ]
    .map(str::to_owned)
    .to_vec();
    let answer = run_faithful(
        args,
        RunEnv {
            user_dir: dir.path().into(),
            max_heap_bytes: Some(64 << 20),
            max_direct_memory_bytes: Some(16 << 20),
            ..RunEnv::default()
        },
    )
    .unwrap();
    assert_eq!(answer.outcome, Outcome::Success);
    assert_eq!(answer.distinct, Some(states.len() as u64));
    assert_eq!(answer.depth, levels.last().map(|&level| level as u64));
}

#[test]
fn overrides_actions_constraints_and_reductions_stay_distinct() {
    let cfg = "CONSTANTS Limit = 99 Labels = {b, a} Bound <- Ceiling\nINIT Init\nNEXT Next\nINVARIANT TypeOK\nCONSTRAINT KeepState\nACTION_CONSTRAINT KeepStep\nSYMMETRY Sym\nVIEW View\nCHECK_DEADLOCK FALSE";
    let dir = fixture(
        "Choices",
        include_str!("../../tests/fixtures/Choices.tla"),
        cfg,
    );
    with_model(ModelRequest::new(dir.path().join("Choices.tla")), |model| {
        let info = model.info();
        assert_eq!(info.variables, ["x", "label"]);
        assert_eq!(
            info.actions
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["Advance", "Reset", "Stay", "Advance"]
        );
        assert_eq!(
            info.actions
                .iter()
                .map(|a| a.id)
                .collect::<BTreeSet<_>>()
                .len(),
            4
        );
        assert_eq!((info.state_constraints, info.action_constraints), (1, 1));
        assert!(info.has_symmetry && info.has_view);
        assert!(!info.check_deadlock);
        let initial = model.initial_states()?;
        let expected = ["a", "b"]
            .into_iter()
            .flat_map(|label| ["0", "1"].map(|x| snapshot(&[("x", x), ("label", label)])))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            initial
                .iter()
                .map(|s| s.snapshot())
                .collect::<Result<BTreeSet<_>, _>>()?,
            expected
        );
        // VIEW, SYMMETRY, and constraints must not silently collapse/filter roots.
        assert_eq!(initial.len(), 4);
        for state in initial {
            let before = state.snapshot()?;
            let x = &before.values["x"];
            let label = &before.values["label"];
            assert_eq!(model.state_allowed(&state)?, x == "0");
            let next = model.successors(&state)?;
            assert_eq!(next.len(), if x == "0" { 3 } else { 4 });
            let mut per_action = BTreeMap::new();
            for transition in &next {
                let name = &info
                    .actions
                    .iter()
                    .find(|a| a.id == transition.action)
                    .unwrap()
                    .name;
                let target = transition.state.snapshot()?;
                assert_eq!(&target.values["label"], label);
                assert_eq!(
                    model.step_allowed(&state, &transition.state)?,
                    target.values["x"] != "0"
                );
                per_action
                    .entry(name.as_str())
                    .or_insert_with(Vec::new)
                    .push(target.values["x"].clone());
                if target.values["x"] == "2" {
                    assert!(!model.state_allowed(&transition.state)?);
                    // Configured operator override Bound <- Ceiling, not Limit=99.
                    let after = model.successors(&transition.state)?;
                    assert_eq!(after.len(), 2);
                    assert!(after.iter().all(|t| {
                        info.actions.iter().find(|a| a.id == t.action).unwrap().name != "Advance"
                    }));
                }
            }
            assert_eq!(per_action["Advance"], [if x == "0" { "1" } else { "2" }; 2]);
            assert_eq!(per_action["Stay"], std::slice::from_ref(x));
            if x == "1" {
                assert_eq!(per_action["Reset"], ["0"]);
            }
            assert_eq!(state.snapshot()?, before);
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn no_implicit_stuttering_and_properties_do_not_filter_generation() {
    let dir = fixture("Counter", COUNTER, "INIT Init\nNEXT Stop\nINVARIANT Broken");
    with_model(ModelRequest::new(dir.path().join("Counter.tla")), |model| {
        let mut state = model.initial_states()?.remove(0);
        for x in 1..=3 {
            state = model.successors(&state)?.remove(0).state;
            assert_eq!(state.snapshot()?, snapshot(&[("x", &x.to_string())]));
        }
        assert!(model.successors(&state)?.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn duplicate_initial_states_use_value_equality_not_reference_identity() {
    let dir = fixture(
        "Duplicate",
        "---- MODULE Duplicate ----\nVARIABLE x\nInit == x = 0 \\/ x = 0\nNext == UNCHANGED x\n====",
        "INIT Init\nNEXT Next",
    );
    with_model(
        ModelRequest::new(dir.path().join("Duplicate.tla")),
        |model| {
            let initial = model.initial_states()?;
            assert_eq!(initial.len(), 2);
            assert!(initial[0].same_values(&initial[1])?);
            let successor = model.successors(&initial[0])?.remove(0).state;
            assert!(initial[0].same_values(&successor)?);
            assert_ne!(initial[0].level(), successor.level());
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn config_and_library_paths_resolve_without_changing_working_directory() {
    let dir = fixture(
        "Main",
        "---- MODULE Main ----\nEXTENDS Helper\nVARIABLE n\nInit == n = Start\nNext == UNCHANGED n\n====",
        "deliberately invalid default config",
    );
    fs::create_dir(dir.path().join("lib")).unwrap();
    fs::write(
        dir.path().join("lib/Helper.tla"),
        "---- MODULE Helper ----\nCONSTANT Start\n====",
    )
    .unwrap();
    fs::write(
        dir.path().join("alternate.cfg"),
        "CONSTANT Start = 7\nINIT Init\nNEXT Next",
    )
    .unwrap();
    let mut request = ModelRequest::new(dir.path().join("Main.tla"));
    request.config = Some("alternate.cfg".into());
    request.library_paths.push("lib".into());
    let cwd = std::env::current_dir().unwrap();
    let initial = with_model(request, |model| model.initial_states()?[0].snapshot()).unwrap();
    assert_eq!(initial, snapshot(&[("n", "7")]));
    assert_eq!(std::env::current_dir().unwrap(), cwd);
    assert!(!dir.path().join("states").exists());
}

fn load_choice(path: &Path, cfg: &str) -> (ModelInfo, StateSnapshot) {
    let mut request = ModelRequest::new(path);
    request.config = Some(cfg.into());
    with_model(request, |model| {
        Ok((model.info().clone(), model.initial_states()?[0].snapshot()?))
    })
    .unwrap()
}

#[test]
fn repeated_and_concurrent_loads_reset_model_values_slots_and_choose_order() {
    let dir = fixture(
        "Choose",
        "---- MODULE Choose ----\nCONSTANT Names\nVARIABLE chosen\nInit == chosen = CHOOSE n \\in Names : TRUE\nNext == UNCHANGED chosen\n====",
        "CONSTANT Names = {z, a}\nINIT Init\nNEXT Next",
    );
    fs::write(
        dir.path().join("Other.cfg"),
        "CONSTANT Names = {a, z}\nINIT Init\nNEXT Next",
    )
    .unwrap();
    let path = dir.path().join("Choose.tla");
    let first = load_choice(&path, "Choose.cfg");
    assert_eq!(first.1, snapshot(&[("chosen", "z")]));
    assert_eq!(
        load_choice(&path, "Other.cfg").1,
        snapshot(&[("chosen", "a")])
    );
    let other = fixture(
        "Different",
        "---- MODULE Different ----\nVARIABLES x, y\nInit == x = 1 /\\ y = 2\nNext == UNCHANGED <<x, y>>\n====",
        "INIT Init\nNEXT Next",
    );
    with_model(
        ModelRequest::new(other.path().join("Different.tla")),
        |model| {
            assert_eq!(
                model.initial_states()?[0].snapshot()?,
                snapshot(&[("x", "1"), ("y", "2")])
            );
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(load_choice(&path, "Choose.cfg"), first);
    let handles: Vec<_> = ["Choose.cfg", "Other.cfg", "Choose.cfg", "Other.cfg"]
        .into_iter()
        .map(|cfg| {
            let path = path.clone();
            std::thread::spawn(move || load_choice(&path, cfg).1)
        })
        .collect();
    for (index, handle) in handles.into_iter().enumerate() {
        assert_eq!(
            handle.join().unwrap(),
            snapshot(&[("chosen", if index % 2 == 0 { "z" } else { "a" })])
        );
    }
}

#[test]
fn failed_assumptions_and_invalid_loads_do_not_enter_callback_or_poison_next_load() {
    for (spec, cfg, stage, code) in [
        (
            "---- MODULE Bad ----\nEXTENDS MissingLibrary\n====",
            "",
            Stage::Load,
            Some(mp::TLC_PARSING_FAILED),
        ),
        (
            "---- MODULE Bad ----\nVARIABLE\n====",
            "",
            Stage::Load,
            Some(mp::TLC_PARSING_FAILED),
        ),
        (
            "---- MODULE Bad ----\nASSUME FALSE\n====",
            "",
            Stage::Assumptions,
            Some(mp::TLC_ASSUMPTION_FALSE),
        ),
        (
            "---- MODULE Bad ----\nASSUME 1\n====",
            "",
            Stage::Assumptions,
            Some(mp::TLC_ASSUMPTION_EVALUATION_ERROR),
        ),
        (
            "---- MODULE Bad ----\n====",
            "CONSTANT Missing",
            Stage::Load,
            None,
        ),
    ] {
        let dir = fixture("Bad", spec, cfg);
        let error = with_model(
            ModelRequest::new(dir.path().join("Bad.tla")),
            |_| -> Result<(), tla_bridge::ModelError> { panic!("callback after rejected model") },
        )
        .unwrap_err();
        assert_eq!(error.stage, stage, "{error}");
        if let Some(code) = code {
            assert_eq!(error.code, Some(code));
        }
        assert!(!error.message.is_empty());
        if spec.contains("MissingLibrary") {
            assert!(
                error
                    .message
                    .contains("Cannot find source file for module MissingLibrary")
            );
        }
        let good = fixture("Counter", COUNTER, "INIT Init\nNEXT Next");
        with_model(
            ModelRequest::new(good.path().join("Counter.tla")),
            |model| {
                assert_eq!(
                    model.initial_states()?[0].snapshot()?,
                    snapshot(&[("x", "0")])
                );
                Ok(())
            },
        )
        .unwrap();
    }
    let dir = fixture("Counter", COUNTER, "INIT Init\nNEXT Next");
    fs::remove_file(dir.path().join("Counter.cfg")).unwrap();
    let error = with_model(
        ModelRequest::new(dir.path().join("Counter.tla")),
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.stage, Stage::Load);
}

#[test]
fn incomplete_states_and_evaluation_errors_are_errors_not_empty_sets() {
    for (init, next, stage, code) in [
        (
            "x = 0",
            "UNCHANGED <<x, y>>",
            Stage::InitialStates,
            mp::TLC_STATE_NOT_COMPLETELY_SPECIFIED_INITIAL,
        ),
        (
            "x = 0 /\\ y = 1",
            "x' = 1",
            Stage::Successors,
            mp::TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT,
        ),
        (
            "x = 0 /\\ y = 1",
            "x' = 1 \\div 0 /\\ UNCHANGED y",
            Stage::Successors,
            mp::TLC_MODULE_DIVISION_BY_ZERO,
        ),
    ] {
        let spec = format!(
            "---- MODULE Partial ----\nEXTENDS Naturals\nVARIABLES x, y\nInit == {init}\nNext == {next}\n===="
        );
        let dir = fixture("Partial", &spec, "INIT Init\nNEXT Next");
        let error = with_model(ModelRequest::new(dir.path().join("Partial.tla")), |model| {
            let initial = model.initial_states()?;
            model.successors(&initial[0])?;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.stage, stage, "{error}");
        assert_eq!(error.code, Some(code), "{error}");
    }
}

#[test]
fn callback_panic_cleans_up_and_nested_oracle_calls_fail_without_deadlocking() {
    let dir = fixture("Counter", COUNTER, "INIT Init\nNEXT Next");
    let request = ModelRequest::new(dir.path().join("Counter.tla"));
    let panicking = request.clone();
    assert!(
        std::panic::catch_unwind(|| with_model(
            panicking,
            |_| -> Result<(), tla_bridge::ModelError> { panic!("test callback panic") }
        ))
        .is_err()
    );
    let nested = request.clone();
    with_model(request, |model| {
        let error = with_model(nested, |_| Ok(())).unwrap_err();
        assert_eq!(error.stage, Stage::Session);
        assert!(
            run_faithful(Vec::new(), RunEnv::default())
                .unwrap_err()
                .contains("session is already active")
        );
        assert_eq!(
            model.initial_states()?[0].snapshot()?,
            snapshot(&[("x", "0")])
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn reports_property_kinds_and_fairness_without_running_liveness() {
    let spec = COUNTER.replace(
        "Spec == Init /\\ [][Next]_x",
        "Spec == Init /\\ [][Next]_x /\\ WF_x(Next)",
    );
    let cfg = "SPECIFICATION Spec\nINVARIANT Safe\nPROPERTY Init NoChange EventuallyAlwaysZero";
    let dir = fixture("Counter", &spec, cfg);
    let info = with_model(ModelRequest::new(dir.path().join("Counter.tla")), |model| {
        Ok(model.info().clone())
    })
    .unwrap();
    assert_eq!(info.invariants, ["Safe"]);
    assert_eq!(info.initial_properties, ["Init"]);
    assert_eq!(info.action_properties, ["NoChange"]);
    assert_eq!(info.temporal_properties, ["EventuallyAlwaysZero"]);
    assert_eq!(info.temporal_assumptions.len(), 1);
}

#[test]
fn resets_random_seed_and_captures_assumption_output_once_per_session() {
    let spec = "---- MODULE Random ----\nEXTENDS Naturals, TLC\nVARIABLE x\nASSUME PrintT(\"loaded\")\nInit == x = RandomElement(1..1000000)\nNext == UNCHANGED x\n====";
    let dir = fixture("Random", spec, "INIT Init\nNEXT Next");
    let load = |seed| {
        let mut request = ModelRequest::new(dir.path().join("Random.tla"));
        request.seed = seed;
        with_model(request, |model| {
            assert_eq!(model.info().assumptions, 1);
            assert_eq!(model.take_output(), ["\"loaded\""]);
            assert!(model.take_output().is_empty());
            model.initial_states()?[0].snapshot()
        })
        .unwrap()
    };
    let first = load(1);
    let second = load(98765);
    assert_ne!(first, second);
    assert_eq!(load(1), first);
}
