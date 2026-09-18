use tla_eval::{Evaluator, Value};
use tla_ir::{Builtin as B, Expr, Level, Literal, Program, Quantifier};

fn literal(program: &mut Program, n: i32) -> usize {
    program.push(Expr::Literal(Literal::Int(n)), Level::Constant)
}
fn apply(program: &mut Program, op: B, args: Vec<usize>) -> usize {
    program.push(Expr::Apply { op, args }, Level::Constant)
}

#[test]
fn sort_validates_comparator_even_without_comparisons() {
    for size in 0..=2 {
        let mut p = Program::default();
        let one = literal(&mut p, 1);
        let sequence = apply(&mut p, B::Tuple, vec![one; size]);
        let sort = apply(&mut p, B::SortSeq, vec![sequence, one]);
        assert_eq!(
            Evaluator::new(&p, &[], &[], &[])
                .evaluate(sort)
                .unwrap_err()
                .0,
            "The second argument of SortSeq should be a operator, but instead it is:\n1"
        );
    }
}

#[test]
fn finite_functions_quantifiers_and_checked_arithmetic() {
    let mut p = Program::default();
    let one = literal(&mut p, 1);
    let three = literal(&mut p, 3);
    let range = apply(&mut p, B::Range, vec![one, three]);
    let local = p.push(Expr::Local(0), Level::Constant);
    let square = apply(&mut p, B::Mul, vec![local, local]);
    let function = p.push(
        Expr::Function {
            slot: 0,
            domain: Some(range),
            body: square,
        },
        Level::Constant,
    );
    let call = apply(&mut p, B::Apply, vec![function, three]);
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(call).unwrap(), Value::Int(9));
    let predicate = apply(&mut p, B::Ge, vec![local, one]);
    let all = p.push(
        Expr::Quantify {
            kind: Quantifier::ForAll,
            slot: 0,
            domain: Some(range),
            body: predicate,
        },
        Level::Constant,
    );
    let choose = p.push(
        Expr::Choose {
            slot: 0,
            domain: Some(range),
            body: predicate,
        },
        Level::Constant,
    );
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(all).unwrap(), Value::Bool(true));
    assert_eq!(eval.evaluate(choose).unwrap(), Value::Int(1));
    let max = literal(&mut p, i32::MAX);
    let overflow = apply(&mut p, B::Add, vec![max, one]);
    assert!(
        Evaluator::new(&p, &[], &[], &[])
            .evaluate(overflow)
            .is_err()
    );
    p.validate().unwrap();
}

#[test]
fn conditionals_short_circuit_and_extensional_sequences() {
    let mut p = Program::default();
    let yes = p.push(Expr::Literal(Literal::Bool(true)), Level::Constant);
    let zero = literal(&mut p, 0);
    let one = literal(&mut p, 1);
    let invalid = apply(&mut p, B::Div, vec![one, zero]);
    let condition = p.push(
        Expr::If {
            condition: yes,
            then_expr: one,
            else_expr: invalid,
        },
        Level::Constant,
    );
    let tuple = apply(&mut p, B::Tuple, vec![one]);
    let append = apply(&mut p, B::Append, vec![tuple, zero]);
    let head = apply(&mut p, B::Head, vec![append]);
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(condition).unwrap(), Value::Int(1));
    assert_eq!(eval.evaluate(head).unwrap(), Value::Int(1));
    assert_eq!(
        eval.evaluate(append).unwrap(),
        Value::function(vec![
            (Value::Int(1), Value::Int(1)),
            (Value::Int(2), Value::Int(0))
        ])
        .unwrap()
    );
}

#[test]
fn infinite_membership_and_lazy_finiteness_do_not_require_enumeration() {
    let mut p = Program::default();
    let one = literal(&mut p, 1);
    let naturals = apply(&mut p, B::Naturals, vec![]);
    let contains = apply(&mut p, B::In, vec![one, naturals]);
    let finite = apply(&mut p, B::IsFiniteSet, vec![naturals]);
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(contains).unwrap(), Value::Bool(true));
    assert_eq!(eval.evaluate(finite).unwrap(), Value::Bool(false));
    assert!(eval.evaluate(naturals).is_err());
}

#[test]
fn infinite_function_application_and_overrides_are_deferred() {
    let mut p = Program::default();
    let naturals = apply(&mut p, B::Naturals, vec![]);
    let x = p.push(Expr::Local(0), Level::Constant);
    let function = p.push(
        Expr::Function {
            slot: 0,
            domain: Some(naturals),
            body: x,
        },
        Level::Constant,
    );
    let two = literal(&mut p, 2);
    let three = literal(&mut p, 3);
    let seven = literal(&mut p, 7);
    let negative = literal(&mut p, -1);
    let path = apply(&mut p, B::Tuple, vec![two]);
    let update = apply(&mut p, B::Tuple, vec![path, seven]);
    let updated = apply(&mut p, B::Except, vec![function, update]);
    let changed = apply(&mut p, B::Apply, vec![updated, two]);
    let unchanged = apply(&mut p, B::Apply, vec![updated, three]);
    let outside = apply(&mut p, B::Apply, vec![function, negative]);
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(changed).unwrap(), Value::Int(7));
    assert_eq!(eval.evaluate(unchanged).unwrap(), Value::Int(3));
    assert!(eval.evaluate(outside).is_err());
    let at = p.push(Expr::At, Level::Constant);
    let increment = apply(&mut p, B::Add, vec![at, three]);
    let second_update = apply(&mut p, B::Tuple, vec![path, increment]);
    let updated_twice = apply(&mut p, B::Except, vec![function, update, second_update]);
    let changed_twice = apply(&mut p, B::Apply, vec![updated_twice, two]);
    assert_eq!(
        Evaluator::new(&p, &[], &[], &[])
            .evaluate(changed_twice)
            .unwrap(),
        Value::Int(10)
    );
    let zero = literal(&mut p, 0);
    let invalid = apply(&mut p, B::Div, vec![two, zero]);
    let invalid_update = apply(&mut p, B::Tuple, vec![path, invalid]);
    let invalid_function = apply(&mut p, B::Except, vec![function, invalid_update]);
    let other = apply(&mut p, B::Apply, vec![invalid_function, three]);
    assert!(Evaluator::new(&p, &[], &[], &[]).evaluate(other).is_err());
}

#[test]
fn lazy_membership_and_canonical_local_values_preserve_semantics() {
    let mut p = Program::default();
    let local = p.push(Expr::Local(0), Level::Constant);
    let empty = apply(&mut p, B::Tuple, vec![]);
    let equal = apply(&mut p, B::Eq, vec![local, empty]);
    p.operators.push(tla_ir::Operator {
        name: "Empty".into(),
        parameters: vec![0],
        body: equal,
    });
    let call = p.push(
        Expr::Call {
            operator: 0,
            args: vec![empty],
        },
        Level::Constant,
    );
    assert_eq!(
        Evaluator::new(&p, &[], &[], &[]).evaluate(call).unwrap(),
        Value::Bool(true)
    );
    let model = p.push(
        Expr::Literal(Literal::Model {
            text: "m".into(),
            ordinal: 1,
        }),
        Level::Constant,
    );
    let naturals = apply(&mut p, B::Naturals, vec![]);
    let sequences = apply(&mut p, B::Sequences, vec![naturals]);
    let member = apply(&mut p, B::In, vec![model, sequences]);
    assert_eq!(
        Evaluator::new(&p, &[], &[], &[]).evaluate(member).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn frozen_string_table_allows_existing_strings_and_sequences() {
    let mut p = Program::default();
    let text = p.push(
        Expr::Literal(Literal::String {
            ordinal: 1,
            text: "a".into(),
        }),
        Level::Constant,
    );
    let concat = apply(&mut p, B::Concat, vec![text, text]);
    let one = literal(&mut p, 1);
    let tuple = apply(&mut p, B::Tuple, vec![one, one]);
    let tail = apply(&mut p, B::Tail, vec![tuple]);
    let mut eval = Evaluator::new(&p, &[], &[], &[]);
    eval.intern_new_strings = false;
    assert!(
        eval.evaluate(concat)
            .unwrap_err()
            .0
            .contains("new string ordinal")
    );
    assert_eq!(eval.strings.lock().unwrap().len(), 1);
    assert_eq!(
        eval.evaluate(tail).unwrap(),
        Value::tuple(vec![Value::Int(1)])
    );
    eval.intern_new_strings = true;
    let expected = eval.evaluate(concat).unwrap();
    eval.intern_new_strings = false;
    assert_eq!(eval.evaluate(concat).unwrap(), expected);
    let mut shared =
        Evaluator::with_shared_storage(&p, &[], &[], &[], eval.strings.clone(), eval.cache.clone());
    shared.intern_new_strings = false;
    assert_eq!(shared.evaluate(concat).unwrap(), expected);
    let mut independent = Evaluator::new(&p, &[], &[], &[]);
    independent.intern_new_strings = false;
    assert!(independent.evaluate(concat).is_err());
}

#[test]
fn tuple_binding_validation_terminates_on_recursive_ir() {
    let mut p = Program::default();
    let zero = literal(&mut p, 0);
    let domain = apply(&mut p, B::Set, vec![zero]);
    let yes = p.push(Expr::Literal(Literal::Bool(true)), Level::Constant);
    let function = p.push(
        Expr::RecursiveFunction {
            slot: 1,
            domain: Some(domain),
            body: zero,
        },
        Level::Constant,
    );
    let recurse = apply(&mut p, B::Apply, vec![function, zero]);
    let body = p.push(
        Expr::If {
            condition: yes,
            then_expr: zero,
            else_expr: recurse,
        },
        Level::Constant,
    );
    p.nodes[function].expr = Expr::RecursiveFunction {
        slot: 1,
        domain: Some(domain),
        body,
    };
    let result = p.push(
        Expr::Quantify {
            kind: Quantifier::SetMap,
            slot: 0,
            domain: Some(domain),
            body: recurse,
        },
        Level::Constant,
    );
    assert_eq!(
        Evaluator::new(&p, &[], &[], &[]).evaluate(result).unwrap(),
        Value::set(vec![Value::Int(0)]).unwrap()
    );
}

#[test]
fn substitution_rebinds_captures_inside_recursive_operator_bodies() {
    let mut p = Program::default();
    let capture = p.push(Expr::Local(0), Level::Constant);
    let argument = p.push(Expr::Local(1), Level::Constant);
    let zero = literal(&mut p, 0);
    let one = literal(&mut p, 1);
    let three = literal(&mut p, 3);
    let ten = literal(&mut p, 10);
    let done = apply(&mut p, B::Eq, vec![argument, zero]);
    let decrement = apply(&mut p, B::Sub, vec![argument, one]);
    p.operators.push(tla_ir::Operator {
        name: "fold".into(),
        parameters: vec![1],
        body: zero,
    });
    let recurse = p.push(
        Expr::Call {
            operator: 0,
            args: vec![decrement],
        },
        Level::Constant,
    );
    let sum = apply(&mut p, B::Add, vec![capture, recurse]);
    p.operators[0].body = p.push(
        Expr::If {
            condition: done,
            then_expr: zero,
            else_expr: sum,
        },
        Level::Constant,
    );
    let call = p.push(
        Expr::Call {
            operator: 0,
            args: vec![three],
        },
        Level::Constant,
    );
    assert_eq!(p.replace_local(call, 99, ten), call);
    let first = p.replace_local(call, 0, one);
    let second = p.replace_local(call, 0, ten);
    let shadowed = p.replace_local(call, 1, ten);
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(first).unwrap(), Value::Int(3));
    assert_eq!(eval.evaluate(second).unwrap(), Value::Int(30));
    assert_eq!(
        eval.with_locals(call, &[Some(Value::Int(2))]).unwrap(),
        Value::Int(6)
    );
    assert_eq!(
        eval.with_locals(shadowed, &[Some(Value::Int(2))]).unwrap(),
        Value::Int(6)
    );
}

#[test]
fn injectivity_checks_function_ranges() {
    for (values, expected) in [(vec![], true), (vec![1, 2], true), (vec![1, 1], false)] {
        let function = Value::tuple(values.into_iter().map(Value::Int).collect());
        assert_eq!(
            tla_library::apply_builtin(B::IsInjective, vec![function]).unwrap(),
            Value::Bool(expected)
        );
    }
    assert!(tla_library::apply_builtin(B::IsInjective, vec![Value::Int(1)]).is_err());
}

#[test]
fn unused_quantifier_tuple_bindings_still_require_exact_arity() {
    for values in [vec![1], vec![1, 2], vec![1, 2, 3]] {
        let mut p = Program::default();
        let arity = values.len();
        let domain = p.push(
            Expr::Literal(Literal::Set(vec![Literal::Tuple(
                values.into_iter().map(Literal::Int).collect(),
            )])),
            Level::Constant,
        );
        let body = p.push(Expr::Literal(Literal::Bool(true)), Level::Constant);
        for kind in [Quantifier::Exists, Quantifier::ForAll] {
            let domain = p.tuple_domain(0, domain, 2, true);
            let expression = p.push(
                Expr::Quantify {
                    kind,
                    slot: 0,
                    domain: Some(domain),
                    body,
                },
                Level::Constant,
            );
            let result = Evaluator::new(&p, &[], &[], &[]).evaluate(expression);
            if arity == 2 {
                assert_eq!(result.unwrap(), Value::Bool(true));
            } else {
                assert!(result.is_err());
            }
        }
        let prefix = p.tuple_domain(0, domain, 2, false);
        assert_eq!(
            Evaluator::new(&p, &[], &[], &[]).evaluate(prefix).is_ok(),
            arity >= 2
        );
    }
}

#[test]
fn clock_operators_use_wall_time_and_a_shared_elapsed_origin() {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let mut p = Program::default();
    let wall = apply(&mut p, B::JavaTime, vec![]);
    let name = p.push(
        Expr::Literal(Literal::String {
            ordinal: 1,
            text: "duration".into(),
        }),
        Level::Constant,
    );
    let elapsed = apply(&mut p, B::TlcGet, vec![name]);
    let mut evaluator = Evaluator::new(&p, &[], &[], &[]);
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let actual = evaluator.evaluate(wall).unwrap().integer().unwrap() as u64;
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(actual.wrapping_sub(before & 0x7fff_ffff) & 0x7fff_ffff <= after - before);
    evaluator.started = Instant::now() - Duration::from_secs(2);
    let observations = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let started = evaluator.started;
                let program = &p;
                scope.spawn(move || {
                    let mut worker = Evaluator::new(program, &[], &[], &[]);
                    worker.started = started;
                    worker.clone().evaluate(elapsed).unwrap().integer().unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let upper = evaluator.started.elapsed().as_secs() as i32;
    assert!(
        observations
            .into_iter()
            .all(|value| (2..=upper).contains(&value))
    );
    let invalid = apply(&mut p, B::JavaTime, vec![name]);
    assert!(Evaluator::new(&p, &[], &[], &[]).evaluate(invalid).is_err());
}

#[test]
fn quantified_domains_use_set_semantics_without_a_new_enumeration_cap() {
    let mut p = Program::default();
    let zero = literal(&mut p, 0);
    let large = literal(&mut p, 100_000);
    let range = apply(&mut p, B::Range, vec![zero, large]);
    let naturals = apply(&mut p, B::Naturals, vec![]);
    let body = p.push(Expr::Literal(Literal::Bool(true)), Level::Constant);
    let finite = p.push(
        Expr::Quantify {
            kind: Quantifier::Exists,
            slot: 0,
            domain: Some(range),
            body,
        },
        Level::Constant,
    );
    let infinite = p.push(
        Expr::Quantify {
            kind: Quantifier::Exists,
            slot: 0,
            domain: Some(naturals),
            body,
        },
        Level::Constant,
    );
    let eval = Evaluator::new(&p, &[], &[], &[]);
    assert_eq!(eval.evaluate(finite).unwrap(), Value::Bool(true));
    assert_eq!(
        eval.evaluate(infinite).unwrap_err().0,
        "non-enumerable domain: Nat"
    );
}

#[test]
fn choose_over_unions_preserves_canonical_member_order() {
    let mut p = Program::default();
    let zero = literal(&mut p, 0);
    let two = literal(&mut p, 2);
    let left = apply(&mut p, B::Set, vec![two]);
    let right = apply(&mut p, B::Set, vec![zero]);
    let domain = apply(&mut p, B::Union, vec![left, right]);
    let body = p.push(Expr::Literal(Literal::Bool(true)), Level::Constant);
    let choose = p.push(
        Expr::Choose {
            slot: 0,
            domain: Some(domain),
            body,
        },
        Level::Constant,
    );
    let value = Evaluator::new(&p, &[], &[], &[]).evaluate(choose).unwrap();
    assert_eq!(value, Value::Int(0));
}
