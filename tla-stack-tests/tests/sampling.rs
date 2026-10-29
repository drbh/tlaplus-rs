use std::{path::PathBuf, sync::Mutex};
use tlaplus_bridge::{ModelRequest, with_model};
use tlaplus_eval::Evaluator;
use tlaplus_random::Random;
use tlaplus_value_expr::Expr;
use tlaplus_values::{Error, Value};

#[test]
fn model_setup_snapshot_survives_inspection_and_repeated_initialization() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/RandomModel.tla");
    with_model(ModelRequest::new(path), |faithful| {
        let first = faithful.initial_states()?;
        let second = faithful.initial_states()?;
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.detached_values()?, b.detached_values()?);
        }
        let model = faithful.lower()?;
        let stream = model.runtime.random.current().unwrap();
        let expected_x = (stream.lock().unwrap().next_double() * 100.0) as i32 + 1;
        let slot = model
            .program
            .variables
            .iter()
            .position(|name| name == "x")
            .unwrap();
        assert_eq!(first[0].detached_values()?[slot], Value::Int(expected_x));
        Ok(())
    })
    .unwrap();
}

#[test]
fn native_stream_matches_faithful_across_nested_large_sets_and_repeated_calls() {
    for seed in [0, 1, 42, -1, i64::MIN, i64::MAX] {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/RandomSampling.tla");
        let mut request = ModelRequest::new(path);
        request.seed = seed;
        with_model(request, move |model| {
            let random = Mutex::new(Random::new(seed));
            for name in [
                "Interval",
                "Enumeration",
                "Product",
                "Functions",
                "Records",
                "Nested",
                "EmptyFunction",
                "NestedDraw",
                "SubsetInterval",
                "SubsetEnumeration",
                "SubsetZero",
                "SubsetEmpty",
                "SubsetRepeat",
                "SubsetProduct",
                "SubsetBigProduct",
                "SubsetFunction",
                "SubsetHugeFunction",
                "SubsetRecord",
                "SubsetEmptyProduct",
                "SubsetEmptyFunction",
                "SubsetZeroBig",
                "UnionDomainFunction",
                "PrintUnionFunction",
                "SetsOfSubsets",
                "SetsOfSubsetsEnumeration",
                "SetsOfSubsetsAll",
                "SetsOfSubsetsNone",
                "SetsOfSubsetsZero",
                "SetsOfSubsetsMany",
            ]
            .into_iter()
            .cycle()
            .take(204)
            {
                let (program, expression, expected) = model.constant_expression(name)?;
                let mut evaluator = Evaluator::new(&program, &[], &[], &[]);
                evaluator.random = Some(&random);
                assert_eq!(evaluator.evaluate(expression).unwrap(), expected.unwrap());
            }
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn failures_preserve_the_draw_position() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/RandomSampling.tla");
    with_model(ModelRequest::new(path), |model| {
        let random = Mutex::new(Random::new(1));
        for (index, name) in [
            "EmptySet",
            "Interval",
            "EmptyInterval",
            "Interval",
            "TooLarge",
            "Interval",
            "BadProduct",
            "Interval",
            "BadFunction",
            "Interval",
            "SubsetBadCount",
            "Interval",
            "SubsetNegative",
            "Interval",
            "SubsetMaxInterval",
            "Interval",
            "SetsOfSubsetsTooMany",
            "Interval",
            "SetsOfSubsetsBadN",
            "Interval",
            "SetsOfSubsetsNegative",
            "Interval",
            "SetsOfSubsetsBadSet",
            "Interval",
            "SetsOfSubsetsEmpty",
            "Interval",
        ]
        .into_iter()
        .enumerate()
        {
            let (program, expression, expected) = model.constant_expression(name)?;
            let mut evaluator = Evaluator::new(&program, &[], &[], &[]);
            evaluator.random = Some(&random);
            let actual = evaluator.evaluate(expression);
            if name == "Interval" {
                assert_eq!(actual.unwrap(), expected.unwrap(), "call {index}");
            } else {
                assert!(
                    actual.is_err() && expected.is_err(),
                    "{name}: {actual:?}, {expected:?}"
                );
            }
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn structural_sampling_avoids_enumerating_the_result_space() {
    let mut random = Random::new(42);
    let product = Expr::Product(vec![Expr::Interval(1, 4000); 3]);
    let sampled = random.element(&product, 10).unwrap();
    assert!(product.contains(&Expr::atom(sampled)).unwrap());
    assert_eq!(
        random.element(&Expr::Canonical(Value::Int(1)), 10),
        Err(Error::Type("finite set"))
    );
    assert_eq!(
        random.element(&Expr::Interval(i32::MIN, i32::MAX), 10),
        Err(Error::Overflow)
    );
}

#[test]
fn evaluator_requires_an_explicit_session() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/RandomSampling.tla");
    let (program, id, _) = with_model(ModelRequest::new(path), |model| {
        model.constant_expression("Interval")
    })
    .unwrap();
    assert_eq!(
        Evaluator::new(&program, &[], &[], &[])
            .evaluate(id)
            .unwrap_err()
            .0,
        "RandomElement requires a random session"
    );
}
