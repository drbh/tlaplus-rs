use tla_library::stringify;
use tla_value_expr::Expr;
use tla_values::Value;

#[test]
fn canonical_rendering_matches_faithful_values() {
    tla_bridge::values::with_value_session(|| {
        let words = ["a", "word space", "quote\"", "line\nbreak", "tab\t", "α"];
        let atoms = words
            .iter()
            .map(|s| tla_bridge::values::intern_value(s, false).unwrap())
            .collect::<Vec<_>>();
        let mut seed = 0xabc621efu64;
        for _ in 0..10_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = (seed % 6) as usize;
            let value = match seed % 5 {
                0 => Value::Int(seed as i32),
                1 => atoms[n].clone(),
                2 => Value::tuple((0..n).map(|i| atoms[i].clone()).collect()),
                3 => Value::set((0..n).map(|i| Value::Int(i as i32)).collect()).unwrap(),
                _ => Value::function(
                    (0..n)
                        .map(|i| (Value::Int(i as i32 - 2), atoms[i].clone()))
                        .collect(),
                )
                .unwrap(),
            };
            let faithful = tla_bridge::values::to_faithful(&value).unwrap();
            assert_eq!(
                stringify(&Expr::atom(value.clone())).unwrap(),
                faithful.to_string_checked().unwrap(),
                "{value:?}"
            );
        }
    });
}
