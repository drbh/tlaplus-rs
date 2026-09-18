use std::fs::{self, File};
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use tla_bridge::{ModelRequest, with_model};
use tla_engine::{Options, TransitionSystem, explore, explore_parallel};

#[test]
fn operator_substitution_preserves_assignment_and_let_capture() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("Template.tla"),
        r#"---- MODULE Template ----
EXTENDS Integers
CONSTANT Assign(_, _)
VARIABLE x
Init == Assign(x, 0)
Next == Assign(x', 1 - x)
====
"#,
    )
    .unwrap();
    let path = directory.path().join("Bindings.tla");
    fs::write(
        &path,
        r#"---- MODULE Bindings ----
EXTENDS Integers
VARIABLE x
Assign(a, b) == LET value == b IN a = value
INSTANCE Template
Left(n) == LET value == n IN value
Right(n) == LET value == n + 1 IN value
Inv == /\ Left(x) = x
       /\ Right(x) = x + 1
       /\ Left(x + 2) = x + 2
====
"#,
    )
    .unwrap();
    fs::write(
        directory.path().join("Bindings.cfg"),
        "INIT Init\nNEXT Next\nINVARIANT Inv\n",
    )
    .unwrap();
    let (model, samples) = with_model(ModelRequest::new(path), |faithful| {
        let report = explore(&faithful.system()?, &Options::default())?;
        let samples = report
            .nodes
            .iter()
            .map(|node| {
                let state = tla_engine::Leveled {
                    values: node.state.detached_values()?,
                    depth: node.depth,
                };
                let successors = faithful
                    .successors(&node.state)?
                    .iter()
                    .map(|edge| edge.state.detached_values())
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((state, successors))
            })
            .collect::<Result<Vec<_>, tla_bridge::model::ModelError>>()?;
        Ok((faithful.lower()?, samples))
    })
    .unwrap();
    assert_eq!(samples.len(), 2);
    for (state, expected) in samples {
        assert!(model.invariant(&state).unwrap().is_none());
        let actual: Vec<_> = model
            .successors(&state)
            .unwrap()
            .into_iter()
            .map(|edge| edge.state.values)
            .collect();
        assert_eq!(actual, expected);
    }
    for workers in [1, 4] {
        let report = explore_parallel(
            &model,
            &Options::default(),
            NonZeroUsize::new(workers).unwrap(),
        )
        .unwrap();
        assert_eq!(report.completion, tla_engine::Completion::Complete);
        assert_eq!(report.nodes.len(), 2);
    }
}

#[test]
fn faithful_sany_lowering_executes_owned_values_on_parallel_evaluator() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Counter.tla");
    fs::write(&path,"---- MODULE Counter ----\nEXTENDS Integers\nCONSTANT N\nVARIABLE x\nInit == x = 0\nNext == x' = (x + 1) % N\nInv == x < N\n====\n").unwrap();
    fs::write(
        directory.path().join("Counter.cfg"),
        "CONSTANT N = 4\nINIT Init\nNEXT Next\nINVARIANT Inv\n",
    )
    .unwrap();
    let (model, expected) = with_model(ModelRequest::new(path), |faithful| {
        let expected = explore(&faithful.system()?, &Options::default())?;
        let model = faithful.lower()?;
        Ok((model, expected.nodes.len()))
    })
    .unwrap();
    assert_eq!(expected, 4);
    for workers in [1, 2, 4] {
        let actual = explore_parallel(
            &model,
            &Options::default(),
            NonZeroUsize::new(workers).unwrap(),
        )
        .unwrap();
        assert_eq!(actual.nodes.len(), expected);
        for node in actual.nodes {
            assert!(model.invariant(&node.state).unwrap().is_none());
        }
    }
}

#[test]
fn faithful_bounded_actions_and_choose_constants_lower_without_runtime_bridge() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Choices.tla");
    fs::write(&path,"---- MODULE Choices ----\nEXTENDS Integers\nVARIABLE x\nInit == x \\in 0..2\nNext == \\E y \\in 0..2: x' = y\n====\n").unwrap();
    fs::write(
        directory.path().join("Choices.cfg"),
        "INIT Init\nNEXT Next\n",
    )
    .unwrap();
    let model = with_model(ModelRequest::new(path), |faithful| faithful.lower()).unwrap();
    let result = explore(
        &model,
        &Options {
            record_graph: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(result.nodes.len(), 3);
    assert_eq!(result.edges.len(), 9);
}

#[test]
fn temporal_properties_lower_to_ir_without_safety_evaluation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Temporal.tla");
    fs::write(
        &path,
        "---- MODULE Temporal ----\nVARIABLE x\nInit == x = 0\nNext == x' = x\nSpec == Init /\\ [][Next]_x /\\ <>(x = 0)\n====\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("Temporal.cfg"),
        "SPECIFICATION Spec\n",
    )
    .unwrap();
    let model = with_model(ModelRequest::new(path), |faithful| faithful.lower()).unwrap();
    assert_eq!(
        model.checks.temporal_properties.len() + model.checks.temporal_assumptions.len(),
        1
    );
    assert!(
        model
            .program
            .nodes
            .iter()
            .any(|node| matches!(node.expr, tla_ir::Expr::Temporal { .. }))
    );
}

#[test]
fn symmetry_keys_match_faithful_reduction() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Symmetric.tla");
    fs::write(&path,"---- MODULE Symmetric ----\nEXTENDS TLC\nCONSTANT S\nVARIABLE x\nInit == x \\in S\nNext == x' \\in S\nSymmetry == Permutations(S)\n====\n").unwrap();
    fs::write(
        directory.path().join("Symmetric.cfg"),
        "CONSTANT S = {a, b, c}\nINIT Init\nNEXT Next\nSYMMETRY Symmetry\n",
    )
    .unwrap();
    let (model, expected) = with_model(ModelRequest::new(path), |faithful| {
        let expected = explore(&faithful.system()?, &Options::default())?
            .nodes
            .len();
        Ok((faithful.lower()?, expected))
    })
    .unwrap();
    assert_eq!(expected, 1);
    assert_eq!(
        explore(&model, &Options::default()).unwrap().nodes.len(),
        expected
    );
}

#[test]
fn lowered_recursive_calls_and_except_old_values_evaluate_independently() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Expressions.tla");
    fs::write(&path,"---- MODULE Expressions ----\nEXTENDS Integers\nRECURSIVE Fact(_)\nFact(n) == IF n = 0 THEN 1 ELSE n * Fact(n - 1)\nCheck == <<Fact(6), [<<1, 2>> EXCEPT ![1] = @ + 9]>>\n====\n").unwrap();
    fs::write(directory.path().join("Expressions.cfg"), "").unwrap();
    let (program, id, expected) = with_model(ModelRequest::new(path), |faithful| {
        faithful.constant_expression("Check")
    })
    .unwrap();
    assert_eq!(
        tla_eval::Evaluator::new(&program, &[], &[], &[])
            .evaluate(id)
            .unwrap(),
        expected.unwrap()
    );
}

#[test]
fn multiple_function_domains_and_tuple_binders_preserve_faithful_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Binding.tla");
    fs::write(&path,"---- MODULE Binding ----\nEXTENDS Integers\nCheck == <<[x, y \\in {1, 2} |-> x + y][1, 2], {x + y: x, y \\in {1, 2}}, CHOOSE <<x, y>> \\in {<<1, 2>>, <<3, 4>>}: x < y>>\n====\n").unwrap();
    fs::write(directory.path().join("Binding.cfg"), "").unwrap();
    let (program, id, expected) = with_model(ModelRequest::new(path), |faithful| {
        faithful.constant_expression("Check")
    })
    .unwrap();
    assert_eq!(
        tla_eval::Evaluator::new(&program, &[], &[], &[])
            .evaluate(id)
            .unwrap(),
        expected.unwrap()
    );
}

#[test]
fn symbolic_record_sets_do_not_require_finite_faithful_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("SymbolicRecord.tla");
    fs::write(&path, "---- MODULE SymbolicRecord ----\nEXTENDS Integers\nToken == [pos : 0..2, q : Int, color : {\"white\", \"black\"}]\nVARIABLE token\nInit == token = [pos |-> 0, q |-> 0, color |-> \"white\"]\nNext == UNCHANGED token\nTypeOK == token \\in Token\n====\n").unwrap();
    fs::write(
        directory.path().join("SymbolicRecord.cfg"),
        "INIT Init\nNEXT Next\nINVARIANT TypeOK\n",
    )
    .unwrap();
    let (model, expected) = with_model(ModelRequest::new(path), |faithful| {
        let expected = explore(&faithful.system()?, &Options::default())?
            .nodes
            .len();
        Ok((faithful.lower()?, expected))
    })
    .unwrap();
    assert_eq!(expected, 1);
    assert_eq!(
        explore(&model, &Options::default()).unwrap().nodes.len(),
        expected
    );
}

#[test]
fn r3_gate_levels_and_atom_ordinals_match_the_faithful_front_end() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Gate.tla");
    fs::write(&path, "---- MODULE Gate ----\nEXTENDS Naturals\nCONSTANT K\nVARIABLE x\nInit == x = K /\\ \"left\" # \"right\"\nNext == x' = x + 1\nTypeOK == x \\in Nat\nLiveness == <>(x = K)\n====\n").unwrap();
    fs::write(
        directory.path().join("Gate.cfg"),
        "CONSTANT K = gv\nINIT Init\nNEXT Next\nINVARIANT TypeOK\nPROPERTY Liveness\n",
    )
    .unwrap();
    with_model(ModelRequest::new(path), |faithful| {
        let model = faithful.lower()?;
        use tla_ir::{Expr, Level, Literal};
        let level = |id: usize| model.program.nodes[id].level;
        assert_eq!(
            model
                .checks
                .init
                .iter()
                .map(|&id| level(id))
                .collect::<Vec<_>>(),
            [Level::State]
        );
        assert_eq!(level(model.checks.actions[0].1), Level::Action);
        assert_eq!(level(model.checks.invariants[0].1), Level::State);
        assert_eq!(
            level(model.checks.temporal_properties[0].1),
            Level::Temporal
        );
        let mut ordinals = Vec::new();
        for node in &model.program.nodes {
            if let Expr::Literal(literal) = &node.expr {
                match literal {
                    Literal::String { ordinal, text } => {
                        ordinals.push((*ordinal, text.clone(), false))
                    }
                    Literal::Model { ordinal, text } => {
                        ordinals.push((*ordinal, text.clone(), true))
                    }
                    _ => {}
                }
            }
        }
        ordinals.sort();
        ordinals.dedup();
        assert_eq!(ordinals.len(), 3);
        for (ordinal, text, model_value) in ordinals {
            let expected = tla_bridge::values::intern_value(&text, model_value).unwrap();
            let expected = match expected {
                tla_model::Value::String(atom) | tla_model::Value::Model(atom) => atom.ordinal,
                other => panic!("expected an atom, got {other:?}"),
            };
            assert_eq!(ordinal, expected);
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn unbounded_choose_overrides_and_multi_binder_functions_lower_and_run() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Bind.tla");
    fs::write(&path, "---- MODULE Bind ----\nEXTENDS Naturals\nCONSTANT Vals\nFallback == CHOOSE v : v \\notin Vals\nPair[a \\in Vals, b \\in Vals] == a\nVARIABLE x\nFlag == x \\in Vals\nInit == x \\in Vals\nNext == x' = Pair[x, x] /\\ Flag'\nInv == Fallback \\notin Vals\n====\n").unwrap();
    fs::write(
        directory.path().join("Bind.cfg"),
        "CONSTANT Vals = {v1, v2}\nCONSTANT Fallback = none\nINIT Init\nNEXT Next\nINVARIANT Inv\n",
    )
    .unwrap();
    let (program, id, faithful_value) = with_model(ModelRequest::new(path.clone()), |faithful| {
        faithful.constant_expression("Fallback")
    })
    .unwrap();
    assert!(
        program
            .nodes
            .iter()
            .any(|node| matches!(node.expr, tla_ir::Expr::Choose { domain: None, .. }))
    );
    assert!(
        tla_eval::Evaluator::new(&program, &[], &[], &[])
            .evaluate(id)
            .is_err()
    );
    assert!(faithful_value.is_err());
    let (model, expected) = with_model(ModelRequest::new(path), |faithful| {
        let expected = explore(&faithful.system()?, &Options::default())?
            .nodes
            .len();
        Ok((faithful.lower()?, expected))
    })
    .unwrap();
    assert_eq!(expected, 2);
    assert_eq!(
        explore(&model, &Options::default()).unwrap().nodes.len(),
        expected
    );
}

fn semantic_levels(model: &tla_model::BoundModel) -> Vec<i32> {
    let level = |id: usize| match model.program.nodes[id].level {
        tla_ir::Level::Constant => 0,
        tla_ir::Level::State => 1,
        tla_ir::Level::Action => 2,
        tla_ir::Level::Temporal => 3,
    };
    let mut levels = Vec::new();
    levels.extend(model.checks.init.iter().copied().map(level));
    levels.extend(model.checks.actions.iter().map(|(_, id)| level(*id)));
    for properties in [
        &model.checks.invariants,
        &model.checks.initial_properties,
        &model.checks.action_properties,
        &model.checks.temporal_properties,
        &model.checks.temporal_assumptions,
    ] {
        levels.extend(properties.iter().map(|(_, id)| level(*id)));
    }
    levels.extend(model.checks.state_constraints.iter().copied().map(level));
    levels.extend(model.checks.action_constraints.iter().copied().map(level));
    levels.extend(model.checks.view.map(level));
    levels
}

fn examples_lowering(only: Option<&[&str]>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tlaplus-rs-explore");
    let mut libraries = Vec::new();
    for path in [
        root.join("crates/tla-tlc-eval/community_modules"),
        root.join("crates/tla-sany-semantic/oracle/lib"),
    ] {
        if path.is_dir() {
            libraries.push(path);
        }
    }
    let mut accepted = 0;
    let mut failures = Vec::new();
    let recordings = root.join("tests/tlc-oracle/results/examples");
    for entry in fs::read_dir(recordings).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "gz") {
            continue;
        }
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(File::open(&path).unwrap())
            .read_to_end(&mut bytes)
            .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let (Some(module), Some(cfg)) = (
            data["module"].as_str().map(str::to_owned),
            data["cfg"].as_str().map(str::to_owned),
        ) else {
            continue;
        };
        if only.is_some_and(|items| !items.iter().any(|item| cfg.ends_with(item))) {
            continue;
        }
        let started = data["messages"].as_array().is_some_and(|messages| {
            messages
                .iter()
                .any(|message| message["name"] == "TLC_SANY_END")
                && messages
                    .iter()
                    .any(|message| message["name"] == "TLC_STARTING")
        });
        if !started {
            continue;
        }
        accepted += 1;
        let mut request = ModelRequest::new(root.join(".examples-upstream").join(&module));
        request.config = Some(root.join(".examples-upstream").join(&cfg));
        request.community_modules = true;
        request.library_paths = libraries.clone();
        if let Err(error) = with_model(request, move |faithful| {
            let expected = faithful.configured_levels();
            let lowered = faithful.lower()?;
            faithful.check_atom_ordinals(&lowered.program)?;
            let actual = semantic_levels(&lowered);
            if actual != expected {
                return Err(tla_bridge::model::ModelError {
                    stage: tla_bridge::model::Stage::Load,
                    code: None,
                    message: format!("levels {actual:?} != {expected:?}"),
                });
            }
            Ok(())
        }) {
            let rendered = error.to_string();
            if !rendered.contains("Parsing or semantic")
                && !rendered.contains("semantic analysis")
                && !rendered.starts_with("Assumptions:")
            {
                failures.push(format!("{cfg}: {error}"));
            }
        }
    }
    for entry in fs::read_dir(env!("CARGO_MANIFEST_DIR")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "csv") {
            fs::remove_file(path).unwrap();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(accepted > 0, "no accepted examples");
    if let Some(items) = only {
        assert_eq!(accepted, items.len());
    }
}

#[test]
fn r3_instantiated_levels_and_ordinals_match_on_previously_failing_models() {
    if !PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tlaplus-rs-explore/.examples-upstream")
        .is_dir()
    {
        return;
    }
    examples_lowering(Some(&[
        "MCLiveWriteThroughCache.cfg",
        "product.cfg",
        "CachingMemory/MCWriteThroughCache.cfg",
        "EWD998ChanID.cfg",
        "MCCheckpointCoordinationFailure.cfg",
        "MCKVsnap.cfg",
    ]));
}

#[test]
#[ignore = "full Examples corpus; faithful loads take several minutes"]
fn r3_examples_corpus_lowering_gate() {
    examples_lowering(None);
}

#[test]
fn recursive_function_captures_are_scoped_to_each_let_instance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("FoldCapture.tla");
    fs::write(&path, "---- MODULE FoldCapture ----\nEXTENDS Integers\nSum(op(_,_), S) == LET iter[s \\in SUBSET S] == IF s = {} THEN 0 ELSE LET x == CHOOSE e \\in s : TRUE IN op(x, iter[s \\ {x}]) IN iter[S]\nCheck == <<Sum(+, {1, 2}), Sum(+, {10, 20})>>\n====\n").unwrap();
    fs::write(directory.path().join("FoldCapture.cfg"), "").unwrap();
    let (program, id, expected) = with_model(ModelRequest::new(path), |faithful| {
        faithful.constant_expression("Check")
    })
    .unwrap();
    let actual = tla_eval::Evaluator::new(&program, &[], &[], &[])
        .evaluate(id)
        .unwrap();
    assert_eq!(actual, expected.unwrap());
    assert_eq!(
        actual,
        tla_model::Value::tuple(vec![tla_model::Value::Int(3), tla_model::Value::Int(30)])
    );
}
