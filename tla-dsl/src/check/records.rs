//! Nominal records with complete constructors and statically checked field access.
use super::*;
pub(super) fn declare_records(
    spec: &Spec,
    names: &mut BTreeSet<String>,
    env: &mut Environment,
    enums: &BTreeSet<String>,
) -> Result<(), Error> {
    for (name, fields) in &spec.records {
        declare(name, names)?;
        if fields.is_empty() {
            return Err(error(name.at, "structs need at least one field"));
        }
        let mut seen = BTreeSet::new();
        for (field, ty) in fields {
            declare(field, &mut seen)?;
            check_type(ty, enums, field.at)?;
            if has_function(ty) {
                return Err(error(field.at, "struct fields must contain values"));
            }
        }
        let ty = Type::Record(
            name.text.clone(),
            fields
                .iter()
                .map(|(n, t)| (n.text.clone(), t.clone()))
                .collect(),
        );
        env.insert(format!("__tsl_record_{}", name.text), (ty, 0));
    }
    Ok(())
}
pub(super) fn infer_record(
    name: &str,
    fields: &[(Name, Expr)],
    env: &Environment,
    at: Location,
) -> Result<(Type, u8), Error> {
    let Some((ty @ Type::Record(_, expected), _)) = env.get(&format!("__tsl_record_{name}")) else {
        return Err(error(at, "unknown struct"));
    };
    let mut seen = BTreeSet::new();
    let mut level = 0;
    for (field, value) in fields {
        let (_, expected) = expected
            .iter()
            .find(|(n, _)| n == &field.text)
            .ok_or_else(|| error(field.at, "unknown struct field"))?;
        if !seen.insert(&field.text) {
            return Err(error(field.at, "duplicate struct field"));
        }
        expect(value, env, expected, 2)?;
        level = level.max(infer(value, env)?.1);
    }
    if seen.len() != expected.len() {
        return Err(error(at, "constructor must supply every struct field"));
    }
    Ok((ty.clone(), level))
}
pub(super) fn infer_field(
    value: &Expr,
    field: &Name,
    env: &Environment,
) -> Result<(Type, u8), Error> {
    let (ty, level) = infer(value, env)?;
    let Type::Record(_, fields) = ty else {
        return Err(error(field.at, "field access requires a struct"));
    };
    let (_, ty) = fields
        .into_iter()
        .find(|(n, _)| n == &field.text)
        .ok_or_else(|| error(field.at, "unknown struct field"))?;
    Ok((ty, level))
}
