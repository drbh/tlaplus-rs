//! Static types and levels: constant (0), state (1), action (2), temporal (3).
mod builtins;
mod expressions;
mod records;
use crate::{Error, Location, ast::*};
use expressions::{expect, infer};
use std::collections::{BTreeMap, BTreeSet};
type Environment = BTreeMap<String, (Type, u8)>;
fn error(at: Location, message: impl Into<String>) -> Error {
    Error {
        location: at,
        message: message.into(),
    }
}
fn declare(name: &Name, names: &mut BTreeSet<String>) -> Result<(), Error> {
    const RESERVED: &str = "spec module import enum struct type where is view init call state const fn recursive lambda assume action invariant property transition over fair weak strong pick in require next true false always eventually after if else forall exists map sequence set_of filter choose Int Bool String Set Seq Map Fn Len Head Tail Append SubSeq Cardinality IsFiniteSet Permutations Print PrintT Assert TLCGet TLCSet TLCSetRandomElement RandomElement SortSeq SelectSeq Integers Naturals Sequences FiniteSets TLC Init Next Spec TypeOK vars Nat BOOLEAN TRUE FALSE STRING Infinity UNION SUBSET DOMAIN UNCHANGED ENABLED CHOOSE IF THEN ELSE CASE OTHER LET IN EXCEPT MODULE EXTENDS INSTANCE LOCAL WITH CONSTANT CONSTANTS VARIABLE VARIABLES ASSUME ASSUMPTION AXIOM THEOREM LEMMA COROLLARY PROPOSITION RECURSIVE LAMBDA PROOF PROVE DEF BY OBVIOUS OMITTED QED SUFFICES HAVE TAKE WITNESS PICK DEFINE USE HIDE ONLY NEW STATE ACTION TEMPORAL WF_ SF_";
    if name.at.line != 0
        && (name.text.starts_with('_')
            || RESERVED.split_whitespace().any(|s| s == name.text)
            || builtins::is_builtin(&name.text))
    {
        return Err(error(
            name.at,
            format!("reserved identifier {:?}", name.text),
        ));
    }
    if !names.insert(name.text.clone()) {
        return Err(error(
            name.at,
            format!("duplicate identifier {:?}", name.text),
        ));
    }
    Ok(())
}
fn check_type(ty: &Type, enums: &BTreeSet<String>, at: Location) -> Result<(), Error> {
    match ty {
        Type::Enum(name) if !enums.contains(name) => {
            return Err(error(at, format!("unknown type {name:?}")));
        }
        Type::Set(t) | Type::Seq(t) | Type::Refined(t, _) => check_type(t, enums, at)?,
        Type::Record(_, fields) => {
            for (_, t) in fields {
                check_type(t, enums, at)?;
            }
        }
        Type::Tuple(ts) => {
            for t in ts {
                check_type(t, enums, at)?;
            }
        }
        Type::Map(k, v) => {
            check_type(k, enums, at)?;
            check_type(v, enums, at)?;
        }
        Type::Function(args, result) => {
            if args.iter().any(has_function) || has_function(result) {
                return Err(error(at, "operator parameters must take and return values"));
            }
            for a in args {
                check_type(a, enums, at)?;
            }
            check_type(result, enums, at)?;
        }
        _ => {}
    }
    Ok(())
}
pub fn validate(spec: &Spec) -> Result<(), Error> {
    let mut names = BTreeSet::new();
    declare(&spec.name, &mut names)?;
    let mut env = Environment::new();
    let enums = spec.enums.iter().map(|e| e.name.text.clone()).collect();
    for enumeration in &spec.enums {
        declare(&enumeration.name, &mut names)?;
        if enumeration.variants.is_empty() {
            return Err(error(
                enumeration.name.at,
                "an enum must have at least one variant",
            ));
        }
        let ty = Type::Enum(enumeration.name.text.clone());
        env.insert(
            enumeration.name.text.clone(),
            (Type::Set(Box::new(ty.clone())), 0),
        );
        for variant in &enumeration.variants {
            declare(variant, &mut names)?;
            env.insert(variant.text.clone(), (ty.clone(), 0));
        }
    }
    records::declare_records(spec, &mut names, &mut env, &enums)?;
    for r in &spec.refinements {
        declare(&r.name, &mut names)?;
    }
    for binding in spec.constants.iter().chain(&spec.states) {
        if !(binding.name.at.line == 0
            && spec
                .refinements
                .iter()
                .any(|r| r.name.text == binding.name.text))
        {
            declare(&binding.name, &mut names)?;
        }
        check_type(&binding.ty, &enums, binding.name.at)?;
        if has_function(&binding.ty) {
            return Err(error(
                binding.name.at,
                "operators cannot be stored in constants or state",
            ));
        }
    }
    for function in &spec.functions {
        declare(&function.name, &mut names)?;
    }
    for action in &spec.actions {
        declare(&action.name, &mut names)?;
    }
    for property in &spec.properties {
        declare(&property.name, &mut names)?;
    }
    for &(function, index) in &spec.definition_order {
        if function {
            let f = &spec.functions[index];
            if !f.stateful {
                check_function(f, &mut env, &enums, &names)?;
            }
        } else {
            let c = &spec.constants[index];
            expect(&c.value, &env, &c.ty, 0)?;
            env.insert(c.name.text.clone(), (crate::refinements::erase(&c.ty), 0));
        }
    }
    for assumption in &spec.assumptions {
        expect(assumption, &env, &Type::Bool, 0)?;
    }
    for state in &spec.states {
        if spec.init.is_some() == state.initialized {
            return Err(error(
                state.name.at,
                "use either declaration initializers or an explicit init block for all state",
            ));
        }
        if !state.initialized {
            continue;
        }
        let ty = if state.choice {
            Type::Set(Box::new(state.ty.clone()))
        } else {
            state.ty.clone()
        };
        expect(&state.value, &env, &ty, 0)?;
    }
    for state in &spec.states {
        env.insert(
            state.name.text.clone(),
            (crate::refinements::erase(&state.ty), 1),
        );
    }
    for &(function, index) in &spec.definition_order {
        if function && spec.functions[index].stateful {
            check_function(&spec.functions[index], &mut env, &enums, &names)?;
        }
    }
    check_entrypoints(spec, &env, &names)?;
    if spec.library {
        if !spec.states.is_empty()
            || !spec.actions.is_empty()
            || !spec.properties.is_empty()
            || !spec.fairness.is_empty()
        {
            return Err(error(
                spec.name.at,
                "modules may contain only imports, enums, constants, functions, and assumptions",
            ));
        }
    } else if spec.states.is_empty() || spec.actions.is_empty() {
        return Err(error(
            spec.name.at,
            "a specification needs at least one state and one action",
        ));
    }
    for action in &spec.actions {
        check_action(action, &env, &names)?;
    }
    for property in &spec.properties {
        let maximum = if property.temporal {
            3
        } else if !property.frame.is_empty() {
            2
        } else {
            1
        };
        expect(&property.body, &env, &Type::Bool, maximum)?;
        for variable in &property.frame {
            if !spec.states.iter().any(|s| s.name.text == variable.text) {
                return Err(error(
                    variable.at,
                    "transition frame must name state variables",
                ));
            }
        }
        if property.temporal && infer(&property.body, &env)?.1 != 3 {
            return Err(error(
                property.body.at,
                "property requires always(...) or eventually(...); use invariant for state predicates",
            ));
        }
    }
    for predicate in &spec.refinement_checks {
        expect(predicate, &env, &Type::Bool, 1)?;
    }
    fairness(spec)
}
fn has_function(ty: &Type) -> bool {
    match ty {
        Type::Function(..) => true,
        Type::Tuple(ts) => ts.iter().any(has_function),
        Type::Record(_, fields) => fields.iter().any(|(_, t)| has_function(t)),
        Type::Set(t) | Type::Seq(t) | Type::Refined(t, _) => has_function(t),
        Type::Map(k, v) => has_function(k) || has_function(v),
        _ => false,
    }
}
fn check_function(
    function: &Function,
    env: &mut Environment,
    enums: &BTreeSet<String>,
    names: &BTreeSet<String>,
) -> Result<(), Error> {
    check_type(&function.result, enums, function.name.at)?;
    if has_function(&function.result) {
        return Err(error(
            function.name.at,
            "functions must return values, not operators",
        ));
    }
    let signature = (
        Type::Function(
            function
                .params
                .iter()
                .map(|(_, t)| crate::refinements::erase(t))
                .collect(),
            Box::new(crate::refinements::erase(&function.result)),
        ),
        u8::from(function.stateful),
    );
    let mut local = env.clone();
    let mut names = names.clone();
    if function.recursive {
        local.insert(function.name.text.clone(), signature.clone());
    }
    for (param, ty) in &function.params {
        declare(param, &mut names)?;
        check_type(ty, enums, param.at)?;
        if matches!(ty, Type::Function(..)) && crate::refinements::erase(ty) != *ty {
            return Err(error(
                param.at,
                "refinements inside operator signatures are not supported; use a value-taking wrapper",
            ));
        }
        if function.recursive && has_function(ty) {
            return Err(error(
                param.at,
                "recursive functions cannot take operator parameters",
            ));
        }
        if has_function(ty) && !matches!(ty, Type::Function(..)) {
            return Err(error(param.at, "operators cannot be nested in collections"));
        }
        local.insert(param.text.clone(), (crate::refinements::erase(ty), 0));
    }
    expect(
        &function.body,
        &local,
        &function.result,
        u8::from(function.stateful),
    )?;
    env.insert(function.name.text.clone(), signature);
    Ok(())
}
fn check_action(
    action: &Action,
    globals: &Environment,
    names: &BTreeSet<String>,
) -> Result<(), Error> {
    let mut env = globals.clone();
    let mut names = names.clone();
    for (index, (name, domain)) in action.params.iter().chain(&action.picks).enumerate() {
        declare(name, &mut names)?;
        let (ty, level) = infer(domain, &env)?;
        let Type::Set(element) = ty else {
            return Err(error(
                domain.at,
                "choice domain must be a set or inclusive range",
            ));
        };
        let parameter = index < action.params.len();
        if level > u8::from(!parameter) {
            return Err(error(
                domain.at,
                "action parameter domains must be constant; picks may read current state",
            ));
        }
        env.insert(name.text.clone(), (*element, u8::from(!parameter)));
    }
    for guard in &action.guards {
        expect(guard, &env, &Type::Bool, 1)?;
    }
    let mut writes = BTreeSet::new();
    for (name, value) in &action.updates {
        let Some((ty, 1)) = globals.get(&name.text) else {
            return Err(error(
                name.at,
                format!("next must update a state variable, found {:?}", name.text),
            ));
        };
        if !writes.insert(&name.text) {
            return Err(error(
                name.at,
                format!(
                    "duplicate update to {:?}; updates are simultaneous",
                    name.text
                ),
            ));
        }
        expect(value, &env, ty, 1)?;
    }
    Ok(())
}
fn fairness(spec: &Spec) -> Result<(), Error> {
    let mut used = BTreeSet::new();
    for fair in &spec.fairness {
        let mut parameters = None;
        for name in &fair.actions {
            let action = spec
                .actions
                .iter()
                .find(|a| a.name.text == name.text)
                .ok_or_else(|| {
                    error(
                        name.at,
                        format!("unknown action {:?} in fairness assumption", name.text),
                    )
                })?;
            if !used.insert(&name.text) {
                return Err(error(name.at, "duplicate fairness assumption for action"));
            }
            let signature = action
                .params
                .iter()
                .map(|(n, d)| (n.text.clone(), crate::emit::expr(d)))
                .collect::<Vec<_>>();
            if parameters.as_ref().is_some_and(|p| p != &signature) {
                return Err(error(
                    name.at,
                    "fairness group actions must have identical parameter names and domains",
                ));
            }
            parameters = Some(signature);
        }
    }
    Ok(())
}

fn check_entrypoints(
    spec: &Spec,
    env: &Environment,
    names: &BTreeSet<String>,
) -> Result<(), Error> {
    if spec.library
        && (spec.init.is_some() || spec.next.is_some() || spec.functions.iter().any(|f| f.stateful))
    {
        return Err(error(
            spec.name.at,
            "libraries cannot define init, next, or state views",
        ));
    }
    if let Some(next) = &spec.next {
        let mut seen = BTreeSet::new();
        for name in next {
            if !spec.actions.iter().any(|a| a.name.text == name.text) {
                return Err(error(name.at, "next must name declared actions"));
            }
            if !seen.insert(&name.text) {
                return Err(error(name.at, "duplicate action in next"));
            }
        }
        for fair in &spec.fairness {
            for name in &fair.actions {
                if !seen.contains(&name.text) {
                    return Err(error(
                        name.at,
                        "fairness must name an action selected by next",
                    ));
                }
            }
        }
    }
    if let Some(init) = &spec.init {
        let constants: Environment = env
            .iter()
            .filter(|(_, (_, l))| *l == 0)
            .map(|(n, v)| (n.clone(), v.clone()))
            .collect();
        let mut local = constants;
        let mut names = names.clone();
        for (name, domain) in &init.picks {
            declare(name, &mut names)?;
            let (ty, level) = infer(domain, &local)?;
            let Type::Set(element) = ty else {
                return Err(error(domain.at, "init pick requires a set"));
            };
            if level != 0 {
                return Err(error(domain.at, "init domains must be constant"));
            }
            local.insert(name.text.clone(), (*element, 0));
        }
        for guard in &init.guards {
            expect(guard, &local, &Type::Bool, 0)?;
        }
        let mut writes = BTreeSet::new();
        for (name, value) in &init.updates {
            let state = spec
                .states
                .iter()
                .find(|s| s.name.text == name.text)
                .ok_or_else(|| error(name.at, "init must assign a state variable"))?;
            if !writes.insert(&name.text) {
                return Err(error(name.at, "duplicate init assignment"));
            }
            expect(value, &local, &state.ty, 0)?;
        }
        if writes.len() != spec.states.len() {
            return Err(error(
                init.name.at,
                "init must assign every state variable exactly once",
            ));
        }
    }
    Ok(())
}
