use crate::model::Stage;
use crate::{Model, ModelError};
use std::collections::HashMap;
use tla_ir::{Builtin, Expr, ExprId, Level, Program};
use tla_model::BoundModel;
use tla_sany_semantic::{NodeId, nodes::NodeData};
use tla_tlc_eval::{TlcState, context::Context, context::Obj, spec::Action};

impl Model {
    pub fn lower(&self) -> Result<BoundModel, ModelError> {
        let spec = &self.processed;
        let mut lower = Lower::new(self);
        let init = spec
            .init_preds
            .iter()
            .map(|a| lower.action(a))
            .collect::<Result<_, _>>()?;
        let actions = spec
            .actions
            .iter()
            .map(|a| Ok((a.name_of_default(&self.tool), lower.action(a)?)))
            .collect::<Result<_, ModelError>>()?;
        let mut properties = |items: &[(Action, String)]| {
            items
                .iter()
                .map(|(a, name)| Ok((name.clone(), lower.action(a)?)))
                .collect::<Result<Vec<_>, ModelError>>()
        };
        let invariants = properties(&spec.invariants)?;
        let initial_properties = properties(&spec.implied_inits)?;
        let action_properties = properties(&spec.implied_actions)?;
        let temporal_properties = properties(&spec.implied_temporals)?;
        let temporal_assumptions = properties(&spec.temporals)?;
        let context = Context::empty();
        let mut expression = |id| lower.node(id, &context, &HashMap::new(), 0);
        let mut expressions = |ids: &[NodeId]| {
            ids.iter()
                .map(|id| expression(*id))
                .collect::<Result<Vec<_>, _>>()
        };
        let state_constraints = expressions(&spec.model_constraints)?;
        let action_constraints = expressions(&spec.action_constraints)?;
        let view = spec.view.map(expression).transpose()?;
        let program = lower.finish()?;
        program
            .validate()
            .map_err(|e| ModelError::message(Stage::Load, e))?;
        let mut model = BoundModel::new(program);
        self.replay_random.set(model.randomized);
        self.seed_context(&mut model)?;
        model.init = init;
        model.actions = actions;
        model.invariants = invariants;
        model.initial_properties = initial_properties;
        model.action_properties = action_properties;
        (model.temporal_properties, model.temporal_assumptions) =
            (temporal_properties, temporal_assumptions);
        model.state_constraints = state_constraints;
        model.action_constraints = action_constraints;
        model.view = view;
        model.symmetry = self.lower_symmetry()?;
        model.check_deadlock = spec.config_check_deadlock;
        Ok(model)
    }
}
struct Resolved {
    value: Option<tla_tlc_value::Value>,
    definition: Option<(NodeId, tla_sany_semantic::nodes::OpDefData)>,
}

pub(crate) mod apply;

pub(crate) struct Lower<'a> {
    pub(crate) model: &'a Model,
    pub(crate) program: Program,
    next_local: usize,
    cache: crate::LowerCache,
    enabled_variables: HashMap<NodeId, ExprId>,
    expanding: Vec<NodeId>,
    enabled_scope: HashMap<NodeId, (ExprId, ExprId)>,
}
impl<'a> Lower<'a> {
    pub(crate) fn cache_argument(&mut self, source: NodeId) -> Result<ExprId, ModelError> {
        self.value(&self.model.cache_seed(source))
    }
    pub(crate) fn new(model: &'a Model) -> Self {
        Self {
            model,
            program: model.lowered_program(),
            next_local: 0,
            cache: crate::LowerCache::default(),
            enabled_variables: HashMap::new(),
            expanding: Vec::new(),
            enabled_scope: HashMap::new(),
        }
    }
    #[rustfmt::skip]    fn error(&self, message: impl ToString) -> ModelError { ModelError::message(Stage::Load, message) }
    #[rustfmt::skip]    pub(crate) fn invalid_tlc_eval(&mut self) -> Result<ExprId, ModelError> { self.builtin(Builtin::TlcEvalDefinition, Vec::new(), false, Level::Constant) }
    fn resolve(
        &mut self,
        node: NodeId,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        nested: &mut HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<Resolved, ModelError> {
        let mut value = None;
        let mut current = node;
        for _ in 0..16 {
            match self.model.tool.lookup(current, context, false) {
                Obj::Value(found) => {
                    if let tla_tlc_value::Value::Method(method) = &found
                        && let Some(id) =
                            tla_tlc_eval::modules::as_builtin(method).and_then(|m| m.op_def())
                    {
                        current = id;
                    }
                    value = Some(found);
                    break;
                }
                Obj::Node(inner) if inner != current => current = inner,
                Obj::Node(_) => break,
            }
        }
        let mut definition = None;
        loop {
            match self.model.tool.graph().node(current).data.clone() {
                NodeData::OpDef(found) => {
                    definition = Some((current, found));
                    break;
                }
                NodeData::ThmOrAssumpDef(found) if found.params.is_empty() => {
                    let Some(body) = found.body else { break };
                    definition = Some((
                        current,
                        tla_sany_semantic::nodes::OpDefData {
                            name: found.name,
                            arity: 0,
                            body: Some(body),
                            params: Some(Vec::new()),
                            ..Default::default()
                        },
                    ));
                    break;
                }
                NodeData::SubstIn(data) | NodeData::APSubstIn(data) => {
                    for subst in data.substs {
                        if let Some(expr) = subst.expr {
                            let bound = self.node(expr, context, substitutions, depth + 1)?;
                            nested.insert(subst.op, self.program.copy_expr(bound));
                        }
                    }
                    let Some(body) = data.body else { break };
                    current = body;
                }
                _ => break,
            }
        }
        Ok(Resolved { value, definition })
    }
    fn function_value(
        &mut self,
        node: NodeId,
        definition: &tla_sany_semantic::nodes::OpDefData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<Option<ExprId>, ModelError> {
        if let Some(&value) = self.cache.functions.get(&node) {
            return Ok(Some(value));
        }
        let Some(body) = definition.body else {
            return Ok(None);
        };
        let NodeData::OpAppl(data) = self.model.tool.graph().node(body).data.clone() else {
            return Ok(None);
        };
        let name = data
            .operator
            .and_then(|op| self.model.tool.graph().name(op))
            .map(|name| name.as_str().to_owned())
            .unwrap_or_default();
        let recursive = match name.as_str() {
            "$RecursiveFcnSpec" => true,
            "$NonRecursiveFcnSpec" => false,
            _ => return Ok(None),
        };
        let expression = data
            .operands
            .first()
            .copied()
            .flatten()
            .ok_or_else(|| self.error("missing function body"))?;
        let self_name = definition
            .name
            .as_ref()
            .map(|name| name.as_str().to_owned())
            .unwrap_or_default();
        let level = self.level_of(expression);
        let mut nested = substitutions.clone();
        let mut selfs = Vec::new();
        let (slot, domain) = if data.bounded_bound_symbols.is_some() {
            let bounds = self.binders(&data, context, substitutions, &mut nested, depth)?;
            selfs.extend(data.unbounded_bound_symbols.iter().flatten().copied());
            let (slot, domain) = self.collapse(bounds, &mut nested);
            (slot, Some(domain))
        } else {
            let mut symbols = data.unbounded_bound_symbols.clone().unwrap_or_default();
            symbols.retain(|symbol| {
                let is_self = self
                    .model
                    .tool
                    .graph()
                    .name(*symbol)
                    .is_some_and(|name| name.as_str() == self_name);
                if is_self {
                    selfs.push(*symbol);
                }
                !is_self
            });
            let [symbol] = symbols[..] else {
                return Err(self.error("multi-binder unbounded function spec"));
            };
            let slot = self.next_local;
            self.next_local += 1;
            let local = self.program.push(Expr::Local(slot), Level::Constant);
            nested.insert(symbol, local);
            (slot, None)
        };
        let operator = self.program.operators.len();
        self.program.operators.push(tla_ir::Operator {
            name: name.clone(),
            parameters: vec![slot],
            body: 0,
        });
        let local = self.program.push(Expr::Local(slot), Level::Constant);
        let apply = self.program.push(
            Expr::Call {
                operator,
                args: vec![local],
            },
            Level::Constant,
        );
        let value = self.program.push(
            if recursive {
                Expr::RecursiveFunction {
                    slot,
                    domain,
                    body: apply,
                }
            } else {
                Expr::Function {
                    slot,
                    domain,
                    body: apply,
                }
            },
            level,
        );
        self.cache.functions.insert(node, value);
        for symbol in selfs {
            nested.insert(symbol, value);
        }
        let body = self.node(expression, context, &nested, depth + 1)?;
        self.program.operators[operator].body = body;
        Ok(Some(value))
    }
    pub(super) fn binders(
        &mut self,
        data: &tla_sany_semantic::nodes::OpApplData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        nested: &mut HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<Vec<(usize, ExprId)>, ModelError> {
        let mut bounds = Vec::new();
        let groups = data.bounded_bound_symbols.as_deref().unwrap_or(&[]);
        for (group, (symbols, range)) in groups.iter().zip(&data.ranges).enumerate() {
            let tuple = data.tuple_ors.as_ref().is_some_and(|xs| xs[group]);
            let domain = self.node(
                range.ok_or_else(|| self.error("missing bound range"))?,
                context,
                substitutions,
                depth + 1,
            )?;
            if tuple {
                let slot = self.next_local;
                self.next_local += 1;
                for (index, symbol) in symbols.iter().enumerate() {
                    let local = self.program.tuple_local(slot, index, symbols.len());
                    nested.insert(*symbol, local);
                }
                let exact = data
                    .operator
                    .and_then(|op| self.model.tool.graph().name(op))
                    .is_some_and(|name| {
                        matches!(name.as_str(), "$BoundedExists" | "$BoundedForall")
                    });
                let domain = self
                    .program
                    .tuple_domain(slot, domain, symbols.len(), exact);
                bounds.push((slot, domain));
            } else {
                for symbol in symbols {
                    let slot = self.next_local;
                    self.next_local += 1;
                    let local = self.program.push(Expr::Local(slot), Level::Constant);
                    nested.insert(*symbol, local);
                    bounds.push((slot, domain));
                }
            }
        }
        Ok(bounds)
    }
    pub(super) fn collapse(
        &mut self,
        bounds: Vec<(usize, ExprId)>,
        nested: &mut HashMap<NodeId, ExprId>,
    ) -> (usize, ExprId) {
        if let [(slot, domain)] = bounds[..] {
            return (slot, domain);
        }
        let slot = self.next_local;
        self.next_local += 1;
        let domain = self.program.push(
            Expr::Apply {
                op: Builtin::Cartesian,
                args: bounds.iter().map(|(_, domain)| *domain).collect(),
            },
            Level::Constant,
        );
        for (index, (bound, _)) in bounds.iter().enumerate() {
            let value = self.program.tuple_local(slot, index, bounds.len());
            for expression in nested.values_mut() {
                *expression = self.program.replace_local(*expression, *bound, value);
            }
        }
        (slot, domain)
    }
    #[rustfmt::skip]    fn level_of(&self, id: NodeId) -> Level { match self.model.tool.graph().node(id).level.level { 1 => Level::State, 2 => Level::Action, 3 => Level::Temporal, _ => Level::Constant } }
    fn action(&mut self, action: &Action) -> Result<ExprId, ModelError> {
        let mut substitutions = HashMap::new();
        for wrapper in &action.pred.wrappers {
            let NodeData::SubstIn(data) = &self.model.tool.graph().node(*wrapper).data else {
                return Err(self.error("invalid action substitution wrapper"));
            };
            for subst in &data.substs {
                let expr = subst
                    .expr
                    .ok_or_else(|| self.error("missing substitution expression"))?;
                let id = self.node(expr, &action.con, &substitutions, 0)?;
                substitutions.insert(subst.op, self.program.copy_expr(id));
            }
        }
        self.node(action.pred.node, &action.con, &substitutions, 0)
    }
    #[allow(clippy::too_many_arguments)] #[rustfmt::skip]
    fn recursive(&mut self, id: NodeId, name: String, def: tla_sany_semantic::nodes::OpDefData, params: &[NodeId], args: Vec<ExprId>, nested: &mut HashMap<NodeId, ExprId>, context: &Context, depth: usize, level: Level) -> Result<ExprId, ModelError> {
        let op = if let Some(&op) = self.cache.operators.get(&id) { op } else {
            let slots = (0..params.len()).map(|_| { let s = self.next_local; self.next_local += 1; s }).collect::<Vec<_>>(); let op = self.program.operators.len(); self.cache.operators.insert(id, op);
            self.program.operators.push(tla_ir::Operator { name, parameters: slots.clone(), body: 0 }); for (param, slot) in params.iter().zip(slots) { nested.insert(*param, self.program.push(Expr::Local(slot), Level::Constant)); }
            self.expanding.push(id);
            let body = self.node(def.body.ok_or_else(|| self.error("missing recursive body"))?, context, nested, depth + 1);
            self.expanding.pop();
            self.program.operators[op].body = body?; op
        }; Ok(self.program.push(Expr::Call { operator: op, args }, level))
    }
    fn value(&mut self, value: &tla_tlc_value::Value) -> Result<ExprId, ModelError> {
        if let tla_tlc_value::Value::OpRcd(table) = value {
            let arity = table.domain().first().map_or(0, Vec::len);
            let parameters = (self.next_local..self.next_local + arity).collect();
            self.next_local += arity;
            return Ok(self
                .program
                .table_operator(crate::operator_table(table)?, parameters));
        }
        let value = crate::checker::from_faithful(value).map_err(|e| self.error(e))?;
        Ok(self.program.push(
            Expr::Literal(crate::checker::literal(&value)),
            Level::Constant,
        ))
    }
    pub(crate) fn node(
        &mut self,
        id: NodeId,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        if depth > 256 {
            return Err(self.error("lowering nested too deeply"));
        }
        if let Some(expr) = self
            .program
            .scoped_binding(&id, substitutions, &self.enabled_scope)
        {
            return Ok(expr);
        }
        let node = self.model.tool.graph().node(id).clone();
        let source = self.model.tool.graph().location(id).to_string();
        if let NodeData::Label(data) = &node.data {
            let body = data.body.ok_or_else(|| self.error("missing label body"))?;
            return self.node(body, context, substitutions, depth + 1);
        }
        let level = match node.level.level {
            0 => Level::Constant,
            1 => Level::State,
            2 => Level::Action,
            3 => Level::Temporal,
            _ => return Err(self.error("invalid semantic level")),
        };
        let expression = match node.data {
            NodeData::At(_) => Expr::At,
            NodeData::OpArg(arg) => {
                let operator = arg
                    .op
                    .ok_or_else(|| self.error("missing operator argument"))?;
                if let Some(expression) = substitutions.get(&operator) {
                    return Ok(*expression);
                }
                let mut nested = substitutions.clone();
                let resolved =
                    self.resolve(operator, context, substitutions, &mut nested, depth)?;
                if let Some(value) = &resolved.value
                    && let Ok(expression) = self.value(value)
                {
                    return Ok(expression);
                }
                let Some((node, mut definition)) = resolved.definition else {
                    return Err(self.error(format!(
                        "operator argument {:?} is not a definition",
                        self.model.tool.node_string(operator)
                    )));
                };
                if self.standard_builtin(node, context).is_some() {
                    definition.body = None;
                }
                if let Some(value) =
                    self.function_value(node, &definition, context, substitutions, depth)?
                {
                    return Ok(value);
                }
                let mut parameters = Vec::new();
                let lambda = definition.name.as_ref().map(|n| n.as_str().to_owned());
                for parameter in definition.params.clone().unwrap_or_default() {
                    let slot = self.next_local;
                    self.next_local += 1;
                    parameters.push(slot);
                    nested.insert(
                        parameter,
                        self.program.push(Expr::Local(slot), Level::Constant),
                    );
                }
                let Some(body) = definition.body else {
                    let op = self.standard_builtin(node, context).ok_or_else(|| {
                        self.error(format!("operator argument {lambda:?} has no definition"))
                    })?;
                    let expr = self.program.push(Expr::BuiltinOperator(op), level);
                    return Ok(self.method_origin(expr, node, context));
                };
                let body = self.node(body, context, &nested, depth + 1)?;
                Expr::Lambda { parameters, body }
            }
            NodeData::Numeral(_) | NodeData::String(_) => {
                let value = self
                    .model
                    .tool
                    .eval(id, context, &TlcState::Empty, &TlcState::Empty, 0)
                    .map_err(|e| self.error(e))?;
                return self
                    .value(&value)
                    .map(|expr| self.program.at_source(expr, source));
            }
            NodeData::LetIn(data) => {
                return self.let_in(data, context, substitutions, depth);
            }
            NodeData::SubstIn(data) | NodeData::APSubstIn(data) => {
                let mut nested = substitutions.clone();
                for subst in data.substs {
                    let value = self.node(
                        subst
                            .expr
                            .ok_or_else(|| self.error("missing substitution"))?,
                        context,
                        substitutions,
                        depth + 1,
                    )?;
                    nested.insert(subst.op, self.program.copy_expr(value));
                }
                return self.node(
                    data.body
                        .ok_or_else(|| self.error("missing substituted body"))?,
                    context,
                    &nested,
                    depth + 1,
                );
            }
            NodeData::OpAppl(data) => {
                let expr = self.application(id, data, level, context, substitutions, depth)?;
                return Ok(self.program.at_source(expr, source));
            }
            _ => return Err(self.error(format!("unsupported semantic node {}", node.kind))),
        };
        Ok(self.program.push(expression, level))
    }
}
