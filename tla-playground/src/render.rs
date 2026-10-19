//! Lossless typed values and separate trace/graph projections of checker results.
use serde_json::{Value as Json, json};
use tla_checker::{BoundModel, Completion, Liveness, Outcome, State, Value, ViolationKind};

fn value(v: &Value) -> Json {
    let (kind, data) = match v {
        Value::Bool(v) => ("bool", json!(v)),
        Value::Int(v) => ("int", json!(v)),
        Value::String(v) => ("string", json!(v.text.as_ref())),
        Value::Model(v) => ("model", json!(v.text.as_ref())),
        Value::Set(v) => ("set", Json::Array(v.iter().map(value).collect())),
        Value::Function(v) => (
            "function",
            Json::Array(v.iter().map(|(k, v)| json!([value(k), value(v)])).collect()),
        ),
    };
    json!({"kind":kind,"data":data,"text":tla_value_expr::stringify(&tla_value_expr::Expr::atom(v.clone())).unwrap_or_else(|_| format!("{v:?}"))})
}

fn state(s: &State, model: &BoundModel) -> Json {
    Json::Array(
        model
            .program
            .variables
            .iter()
            .zip(s)
            .map(|(name, v)| json!({"name":name,"value":value(v)}))
            .collect(),
    )
}

pub fn outcome(model: &BoundModel, outcome: Outcome, artifact: Option<&tla_dsl::Artifact>) -> Json {
    let action = |index: usize| {
        let name = &model.checks.actions[index].0;
        let original = artifact.and_then(|a| {
            let name = if name == "Next" && a.actions.len() == 1 {
                &a.actions[0]
            } else {
                name
            };
            a.origins.iter().find(|o| &o.name == name)
        });
        json!({"name":original.map_or(name.as_str(), |o| o.name.as_str()),
            "line":original.map(|o|o.source.line),"column":original.map(|o|o.source.column)})
    };
    let mut status = match outcome.completion {
        Completion::Complete => "passed",
        Completion::StateLimit => "incomplete",
        Completion::Violated(_) => "violated",
    };
    let mut message = match status {
        "passed" => "Configured checks passed for this finite model.",
        "incomplete" => "State limit reached; verification is incomplete.",
        _ => "A counterexample was found.",
    }
    .to_string();
    let mut trace = Vec::new();
    let mut loop_back = None;
    if let Completion::Violated(v) = &outcome.completion {
        message = match &v.kind {
            ViolationKind::Deadlock => "Deadlock: no enabled next action.".into(),
            ViolationKind::Invariant(n) => format!("Invariant {n} is violated."),
            ViolationKind::InitialProperty(n) => format!("Initial property {n} is violated."),
            ViolationKind::ActionProperty(n) => format!("Transition property {n} is violated."),
        };
        for (i, s) in v.trace.states.iter().enumerate() {
            trace.push(json!({"values":state(s,model),"action":i.checked_sub(1).map(|i|action(v.trace.actions[i]))}));
        }
    }
    let mut properties = Vec::new();
    for (name, verdict) in &outcome.liveness {
        properties.push(json!({"name":name,"holds":matches!(verdict,Liveness::Holds)}));
        if let Liveness::Violated {
            steps,
            loop_back: back,
        } = verdict
        {
            status = "violated";
            message = format!(
                "Temporal property {name} is violated; the highlighted cycle repeats forever."
            );
            loop_back = Some(json!({"index":back.0,"stuttering":back.1}));
            trace = steps
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    json!({"values":state(s,model),
                "action":(i>0).then(||json!({"name":"Next / stutter","line":null,"column":null}))})
                })
                .collect();
        }
    }
    if let Some(name) = &outcome.unwitnessed {
        status = "violated";
        message = format!("Possible {name} unwitnessed.");
    }
    if let Some(name) = &outcome.postcondition_violated {
        status = "violated";
        message = format!("Postcondition {name} is violated.");
    }
    let graph = outcome.graph.map(|g| json!({
        "complete":matches!(outcome.completion,Completion::Complete),
        "quotient":!model.checks.symmetry.is_empty(),
        "nodes":g.nodes.iter().enumerate().map(|(i,n)|json!({"id":i,"depth":n.depth,"expanded":n.expanded,
            "parent":n.parent.map(|p|p.0),"values":state(&n.state,model)})).collect::<Vec<_>>(),
        "edges":g.edges.iter().map(|e|json!({"from":e.from,"to":e.to,"action":action(e.action)})).collect::<Vec<_>>()
    }));
    json!({"status":status,"message":message,"trace":trace,"loopBack":loop_back,"graph":graph,
        "statistics":{"states":outcome.statistics.distinct,"initial":outcome.statistics.initial,"depth":outcome.statistics.depth},
        "properties":properties,"generated":artifact.map(|a|json!({"source":a.tla,"config":a.config}))})
}
