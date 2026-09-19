//! Collection primitives, comprehensions, and runtime type predicates.
use super::*;
pub(crate) fn expr(expression: &Expr) -> String {
    match &expression.kind {
        Kind::Record(_, fields) => format!(
            "[{}]",
            fields
                .iter()
                .map(|(n, e)| format!("{} |-> {}", n.text, expr(e)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Kind::Field(value, field) => format!("({}).{}", expr(value), field.text),
        Kind::TotalTest(ty, value) => format!("({} \\in {})", expr(value), type_domain(ty, 0)),
        Kind::TypeTest(ty, value) => type_predicate(ty, &format!("({})", expr(value)), 0),
        Kind::Int(value) | Kind::String(value) | Kind::Name(value) => value.clone(),
        Kind::Bool(value) => if *value { "TRUE" } else { "FALSE" }.into(),
        Kind::Unary(op, body) => {
            if op == "after" {
                return format!("({})'", expr(body));
            }
            let op = match op.as_str() {
                "!" => "~",
                "always" => "[]",
                "eventually" => "<>",
                op => op,
            };
            format!("({op}({}))", expr(body))
        }
        Kind::Binary(op, a, b) => {
            let op = match op.as_str() {
                "is" => "\\in",
                "==" => "=",
                "!=" => "#",
                "&&" => "/\\",
                "||" => "\\/",
                "..=" => "..",
                op => op,
            };
            format!("({} {op} {})", expr(a), expr(b))
        }
        Kind::Tuple(items) => format!(
            "<<{}>>",
            items.iter().map(expr).collect::<Vec<_>>().join(", ")
        ),
        Kind::Collection(set, items) => {
            let values = items.iter().map(expr).collect::<Vec<_>>().join(", ");
            if *set {
                format!("{{{values}}}")
            } else {
                format!("<<{values}>>")
            }
        }
        Kind::Index(base, key) => format!("({})[{}]", expr(base), expr(key)),
        Kind::If(condition, yes, no) => format!(
            "(IF {} THEN {} ELSE {})",
            expr(condition),
            expr(yes),
            expr(no)
        ),
        Kind::Call(name, args) => call(name, &args.iter().map(expr).collect::<Vec<_>>()),
        Kind::Lambda(name, ty, body) => {
            let body = expr(body);
            if crate::refinements::erase(ty) != *ty {
                format!(
                    "LAMBDA {} : IF Assert({}, \"lambda precondition\") THEN {body} ELSE {body}",
                    name.text,
                    type_predicate(ty, &name.text, 0)
                )
            } else {
                format!("LAMBDA {} : {body}", name.text)
            }
        }
        Kind::Binder(op, name, domain, body) => {
            let (n, d, b) = (&name.text, expr(domain), expr(body));
            match op.as_str() {
                "forall" => format!("(\\A {n} \\in {d} : {b})"),
                "exists" => format!("(\\E {n} \\in {d} : {b})"),
                "choose" => format!("(CHOOSE {n} \\in {d} : {b})"),
                "map" | "sequence" => format!("[{n} \\in {d} |-> {b}]"),
                "set_of" => format!("{{{b} : {n} \\in {d}}}"),
                "filter" => format!("{{{n} \\in {d} : {b}}}"),
                _ => unreachable!(),
            }
        }
    }
}
fn call(name: &str, args: &[String]) -> String {
    match (name, args) {
        ("contains", [set, item]) => format!("({item} \\in {set})"),
        ("union" | "intersect" | "difference" | "subset" | "concat" | "merge", [a, b]) => {
            let op = match name {
                "union" => "\\union",
                "intersect" => "\\intersect",
                "difference" => "\\",
                "subset" => "\\subseteq",
                "concat" => "\\o",
                "merge" => "@@",
                _ => unreachable!(),
            };
            format!("({a} {op} {b})")
        }
        ("product" | "product3", items) => format!("({})", items.join(" \\X ")),
        ("powerset", [set]) => format!("(SUBSET {set})"),
        ("domain", [map]) => format!("(DOMAIN {map})"),
        ("put", [map, key, value]) => format!("[{map} EXCEPT ![{key}] = {value}]"),
        ("entry", [key, value]) => format!("({key} :> {value})"),
        ("functions", [keys, values]) => format!("[{keys} -> {values}]"),
        ("sequences", [values, length]) => format!("[1..({length}) -> {values}]"),
        ("sequences_up_to", [values, length]) => {
            format!("(UNION {{[1..__tsl_length -> {values}] : __tsl_length \\in 1..({length})}})")
        }
        _ => {
            let target = match name {
                "len" => "Len",
                "head" => "Head",
                "tail" => "Tail",
                "append" => "Append",
                "slice" => "SubSeq",
                "size" => "Cardinality",
                name => name,
            };
            if args.is_empty() {
                target.into()
            } else {
                format!("{target}({})", args.join(", "))
            }
        }
    }
}
pub(super) fn type_predicate(ty: &Type, value: &str, depth: usize) -> String {
    match ty {
        Type::Int => format!("{value} \\in Int"),
        Type::Bool => format!("{value} \\in BOOLEAN"),
        Type::String => format!("{value} \\in STRING"),
        Type::Record(_, fields) => {
            let keys = fields
                .iter()
                .map(|(n, _)| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let mut terms = vec![format!("DOMAIN ({value}) = {{{keys}}}")];
            terms.extend(
                fields
                    .iter()
                    .map(|(n, t)| type_predicate(t, &format!("({value}).{n}"), depth + 1)),
            );
            format!("({})", terms.join(" /\\ "))
        }
        Type::Refined(_, name) => format!("{}({value})", crate::refinements::predicate(name)),
        Type::Enum(name) => format!("{value} \\in {name}"),
        Type::Set(element) => {
            let item = format!("__tsl_element{depth}");
            format!(
                "(\\A {item} \\in {value} : {})",
                type_predicate(element, &item, depth + 1)
            )
        }
        Type::Seq(element) => {
            let index = format!("__tsl_index{depth}");
            format!(
                "(\\A {index} \\in 1..Len({value}) : {})",
                type_predicate(element, &format!("({value})[{index}]"), depth + 1)
            )
        }
        Type::Map(key, element) => {
            let index = format!("__tsl_key{depth}");
            format!(
                "(\\A {index} \\in DOMAIN {value} : ({}) /\\ ({}))",
                type_predicate(key, &index, depth + 1),
                type_predicate(element, &format!("({value})[{index}]"), depth + 1)
            )
        }
        Type::Tuple(items) => items
            .iter()
            .enumerate()
            .map(|(i, t)| type_predicate(t, &format!("({value})[{}]", i + 1), depth + 1))
            .collect::<Vec<_>>()
            .join(" /\\ "),
        Type::Unknown | Type::Function(..) => unreachable!("state types were checked"),
    }
}

// Mathematical value sets, independent of any configured exploration domain.
fn type_domain(ty: &Type, depth: usize) -> String {
    match ty {
        Type::Int => "Int".into(),
        Type::Bool => "BOOLEAN".into(),
        Type::String => "STRING".into(),
        Type::Enum(n) => n.clone(),
        Type::Record(_, fields) => format!(
            "[{}]",
            fields
                .iter()
                .map(|(n, t)| format!("{n}: {}", type_domain(t, depth + 1)))
                .collect::<Vec<_>>()
                .join(", ")
        ),

        Type::Refined(base, name) => {
            let v = format!("__tsl_domain{depth}");
            format!(
                "{{{v} \\in {} : {}({v})}}",
                type_domain(base, depth + 1),
                crate::refinements::predicate(name)
            )
        }
        Type::Set(t) => format!("SUBSET ({})", type_domain(t, depth + 1)),
        Type::Seq(t) => format!("Seq({})", type_domain(t, depth + 1)),
        Type::Map(k, v) => format!(
            "[{} -> {}]",
            type_domain(k, depth + 1),
            type_domain(v, depth + 1)
        ),
        _ => unreachable!("refinement bases contain values"),
    }
}
