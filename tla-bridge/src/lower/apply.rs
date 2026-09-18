//! Application lowering: operator applications, binders and operator
//! references, from the faithful semantic graph into IR expressions.

use super::Lower;
use crate::model::ModelError;
use std::collections::HashMap;
use tla_ir::{Builtin, Expr, ExprId, Level, Literal, Program, Quantifier};
use tla_sany_semantic::NodeId;
use tla_sany_semantic::nodes::{NodeData, OpApplData};
use tla_tlc_eval::context::Context;

impl<'a> Lower<'a> {
    fn enabled(
        &mut self,
        action: NodeId,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        let bindings = substitutions
            .iter()
            .filter(|(id, _)| {
                self.model.tool.graph().node(**id).kind
                    == tla_sany_semantic::kinds::VARIABLE_DECL_KIND
            })
            .map(|(&id, &expression)| (id, expression));
        let scope = self
            .program
            .enabled_scope(bindings, &mut self.enabled_variables);
        let cache = (scope != self.enabled_scope).then(|| std::mem::take(&mut self.cache));
        let saved = std::mem::replace(&mut self.enabled_scope, scope);
        let body = self.node(action, context, substitutions, depth + 1);
        self.enabled_scope = saved;
        if let Some(cache) = cache {
            self.cache = cache;
        }
        Ok(self.program.push(
            Expr::Apply {
                op: Builtin::Enabled,
                args: vec![body?],
            },
            Level::State,
        ))
    }
    pub(super) fn application(
        &mut self,
        id: NodeId,
        data: OpApplData,
        level: Level,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        let literal_message =
            data.operands.first().and_then(|id| *id).is_some_and(|id| {
                matches!(self.model.tool.graph().node(id).data, NodeData::String(_))
            });
        let expression = {
            let operator = data
                .operator
                .ok_or_else(|| self.error("missing operator"))?;
            if let Some(expr) =
                self.program
                    .scoped_binding(&operator, substitutions, &self.enabled_scope)
            {
                if data.operands.is_empty() {
                    return Ok(expr);
                }
                if self.program.nodes[expr].expr == Expr::BuiltinOperator(Builtin::Enabled)
                    && let [Some(action)] = data.operands.as_slice()
                {
                    return self.enabled(*action, context, substitutions, depth);
                }
                let args: Vec<_> = data
                    .operands
                    .iter()
                    .map(|id| {
                        self.node(
                            id.ok_or_else(|| self.error("missing lambda argument"))?,
                            context,
                            substitutions,
                            depth + 1,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                return self
                    .program
                    .invoke(expr, args, literal_message, level)
                    .map_err(|e| self.error(e));
            }
            let variables = &self.model.processed.variables;
            if let Some(slot) = variables.iter().position(|v| *v == operator) {
                return Ok(self.program.push(
                    Expr::Variable {
                        slot,
                        primed: false,
                    },
                    Level::State,
                ));
            }
            let name = self
                .model
                .tool
                .graph()
                .name(operator)
                .map_or_else(String::new, ToString::to_string);
            if name == "TRUE" || name == "FALSE" {
                return Ok(self.program.push(
                    Expr::Literal(Literal::Bool(name == "TRUE")),
                    Level::Constant,
                ));
            }
            if name == "ENABLED"
                && let [Some(action)] = data.operands.as_slice()
            {
                return self.enabled(*action, context, substitutions, depth);
            }
            if matches!(name.as_str(), "'" | "UNCHANGED") && !self.enabled_scope.is_empty() {
                let arg = data
                    .operands
                    .first()
                    .copied()
                    .flatten()
                    .ok_or_else(|| self.error("missing scoped operand"))?;
                let expr = self.node(arg, context, substitutions, depth + 1)?;
                let primed = self.program.prime(expr).map_err(|e| self.error(e))?;
                return if name == "'" {
                    Ok(primed)
                } else {
                    Ok(self.program.unchanged_between(expr, primed))
                };
            }
            let mut args = Vec::new();
            let mut nested = substitutions.clone();
            let mut bounds = Vec::new();
            if data.bounded_bound_symbols.is_some() {
                bounds = self.binders(&data, context, substitutions, &mut nested, depth)?;
            }
            if data.bounded_bound_symbols.is_none()
                && let Some(symbols) = &data.unbounded_bound_symbols
            {
                let body = data
                    .operands
                    .first()
                    .copied()
                    .flatten()
                    .ok_or_else(|| self.error("missing unbounded body"))?;
                let tuple = data.tuple_ors.as_ref().is_some_and(|xs| xs[0]);
                let mut slots = Vec::new();
                if tuple {
                    let slot = self.next_local;
                    self.next_local += 1;
                    for (index, symbol) in symbols.iter().enumerate() {
                        let local = self.program.tuple_local(slot, index, symbols.len());
                        nested.insert(*symbol, local);
                    }
                    slots.push(slot);
                } else {
                    for symbol in symbols {
                        let slot = self.next_local;
                        self.next_local += 1;
                        let local = self.program.push(Expr::Local(slot), Level::Constant);
                        nested.insert(*symbol, local);
                        slots.push(slot);
                    }
                }
                let mut body = self.node(body, context, &nested, depth + 1)?;
                for slot in slots.into_iter().rev() {
                    body = self.program.push(
                        match name.as_str() {
                            "$UnboundedChoose" => Expr::Choose {
                                slot,
                                domain: None,
                                body,
                            },
                            "$UnboundedExists" => Expr::Quantify {
                                kind: Quantifier::Exists,
                                slot,
                                domain: None,
                                body,
                            },
                            "$UnboundedForall" => Expr::Quantify {
                                kind: Quantifier::ForAll,
                                slot,
                                domain: None,
                                body,
                            },
                            "$NonRecursiveFcnSpec" => Expr::Function {
                                slot,
                                domain: None,
                                body,
                            },
                            "$RecursiveFcnSpec" => Expr::RecursiveFunction {
                                slot,
                                domain: None,
                                body,
                            },
                            _ => {
                                return Err(
                                    self.error(format!("unsupported unbounded operator {name}"))
                                );
                            }
                        },
                        level,
                    );
                }
                return Ok(body);
            }
            if bounds.len() > 1
                && matches!(
                    name.as_str(),
                    "$FcnConstructor" | "$SetOfAll" | "$NonRecursiveFcnSpec" | "$RecursiveFcnSpec"
                )
            {
                let (slot, domain) = self.collapse(bounds, &mut nested);
                bounds = vec![(slot, domain)];
            }
            for arg in &data.operands {
                if arg.is_none() && name == "$Pair" {
                    args.push(
                        self.program
                            .push(Expr::Literal(Literal::Bool(true)), Level::Constant),
                    );
                    continue;
                }
                args.push(self.node(
                    arg.ok_or_else(|| self.error("missing operand"))?,
                    context,
                    &nested,
                    depth + 1,
                )?);
            }
            if name == "TLCEvalDefinition" {
                return self.eval_definition(&data, context, substitutions, depth);
            }
            if let Some(&operator) = self.cache.operators.get(&operator) {
                return Ok(self.program.push(Expr::Call { operator, args }, level));
            }
            if let Some(&operator) = self.cache.names.get(&name) {
                return Ok(self.program.push(Expr::Call { operator, args }, level));
            }
            if !bounds.is_empty() {
                if level == Level::Temporal {
                    return Ok(self.program.temporal(name, args, bounds, level));
                }
                let mut body = *args
                    .first()
                    .ok_or_else(|| self.error("missing bounded body"))?;
                for (slot, domain) in bounds.into_iter().rev() {
                    let domain = Some(domain);
                    let expr = match name.as_str() {
                        "$BoundedExists" => Expr::Quantify {
                            kind: Quantifier::Exists,
                            slot,
                            domain,
                            body,
                        },
                        "$BoundedForall" => Expr::Quantify {
                            kind: Quantifier::ForAll,
                            slot,
                            domain,
                            body,
                        },
                        "$SubsetOf" => Expr::Quantify {
                            kind: Quantifier::SetFilter,
                            slot,
                            domain,
                            body,
                        },
                        "$SetOfAll" => Expr::Quantify {
                            kind: Quantifier::SetMap,
                            slot,
                            domain,
                            body,
                        },
                        "$FcnConstructor" | "$NonRecursiveFcnSpec" => {
                            Expr::Function { slot, domain, body }
                        }
                        "$RecursiveFcnSpec" => Expr::RecursiveFunction { slot, domain, body },
                        "$BoundedChoose" => Expr::Choose { slot, domain, body },
                        _ => {
                            return Err(self.error(format!("unsupported bounded operator {name}")));
                        }
                    };
                    body = self.program.push(expr, level);
                }
                return Ok(body);
            }
            if args.is_empty() && Builtin::from_name(&name).is_none() {
                let resolved =
                    self.resolve(operator, context, substitutions, &mut nested, depth)?;
                if let Some(value) = &resolved.value
                    && let Ok(expression) = self.value(value)
                {
                    return Ok(expression);
                }
                if let Some((node, definition)) = resolved.definition {
                    if let Some(value) =
                        self.function_value(node, &definition, context, substitutions, depth)?
                    {
                        return Ok(value);
                    }
                    if definition.body.is_some() {
                        return self.define(
                            node,
                            name,
                            definition,
                            Vec::new(),
                            &mut nested,
                            context,
                            substitutions,
                            depth,
                            level,
                        );
                    }
                }
                if let Some(value) = resolved.value {
                    return self.value(&value);
                }
            }
            match name.as_str() {
                "'" => {
                    let arg = *args
                        .first()
                        .ok_or_else(|| self.error("missing prime operand"))?;
                    return self.program.prime(arg).map_err(|e| self.error(e));
                }
                "$IfThenElse" if args.len() == 3 => Expr::If {
                    condition: args[0],
                    then_expr: args[1],
                    else_expr: args[2],
                },
                "$SquareAct" | "$AngleAct" if args.len() == 2 => {
                    let mut unchanged = self.program.push(
                        Expr::Apply {
                            op: Builtin::Unchanged,
                            args: vec![args[1]],
                        },
                        Level::Action,
                    );
                    if name == "$AngleAct" {
                        unchanged = self.program.push(
                            Expr::Apply {
                                op: Builtin::Not,
                                args: vec![unchanged],
                            },
                            Level::Action,
                        );
                    }
                    Expr::Apply {
                        op: if name == "$SquareAct" {
                            Builtin::Or
                        } else {
                            Builtin::And
                        },
                        args: vec![args[0], unchanged],
                    }
                }
                _ if level == Level::Temporal => Program::temporal_expr(name, args, Vec::new()),
                _ if self.standard_builtin(operator, context).is_some() => {
                    let mut op = self.standard_builtin(operator, context).unwrap();
                    if op == Builtin::TlcCache {
                        args.push(self.cache_argument(data.operands[0].unwrap())?);
                        op = Builtin::Memo;
                    }
                    let expr = self.builtin(op, args, literal_message, level)?;
                    return Ok(self.method_origin(expr, operator, context));
                }
                _ => {
                    let resolved =
                        self.resolve(operator, context, substitutions, &mut nested, depth)?;
                    if let Some(value @ tla_tlc_value::Value::OpRcd(_)) = &resolved.value {
                        let operator = self.value(value)?;
                        return self
                            .program
                            .invoke(operator, args, literal_message, level)
                            .map_err(|e| self.error(e));
                    }
                    if args.is_empty()
                        && let Some(value) = &resolved.value
                        && let Ok(expression) = self.value(value)
                    {
                        return Ok(expression);
                    }
                    {
                        let Some((node, definition)) = resolved.definition else {
                            return Err(self.error(format!(
                                "unsupported standard operator {name} at {:?}",
                                self.model.tool.node_string(id)
                            )));
                        };
                        if let Some(op) = self.standard_builtin(node, context) {
                            return self.builtin(op, args, literal_message, level);
                        }
                        if let Some(value) =
                            self.function_value(node, &definition, context, substitutions, depth)?
                        {
                            let [argument] = args[..] else {
                                return Err(self.error(format!("arity mismatch for {name}")));
                            };
                            return Ok(self.program.push(
                                Expr::Apply {
                                    op: Builtin::Apply,
                                    args: vec![value, argument],
                                },
                                level,
                            ));
                        }
                        let params = definition.params.clone().unwrap_or_default();
                        if params.len() != args.len() {
                            return Err(self.error(format!("arity mismatch for {name}")));
                        }
                        return self.define(
                            node,
                            name,
                            definition,
                            args,
                            &mut nested,
                            context,
                            substitutions,
                            depth,
                            level,
                        );
                    }
                }
            }
        };
        Ok(self.program.push(expression, level))
    }

    #[rustfmt::skip]
    fn eval_definition(&mut self, data: &OpApplData, context: &Context, substitutions: &HashMap<NodeId, ExprId>, depth: usize) -> Result<ExprId, ModelError> { let Some(Some(id)) = data.operands.first() else { return self.invalid_tlc_eval(); }; let NodeData::String(text) = self.model.tool.graph().node(*id).data.clone() else { return self.invalid_tlc_eval(); }; let Some(def_id) = self.model.tool.module_info(self.model.tool.root_module()).and_then(|info| info.op_defs.iter().find(|&&d| self.model.tool.graph().name(d).is_some_and(|n| n.as_str() == text.value.as_str())).copied()) else { return self.invalid_tlc_eval(); }; let NodeData::OpDef(def) = self.model.tool.graph().node(def_id).data.clone() else { return self.invalid_tlc_eval(); }; if def.params.as_ref().is_some_and(|params| !params.is_empty()) { return self.invalid_tlc_eval(); } self.node(def.body.ok_or_else(|| self.error("definition has no body"))?, context, substitutions, depth + 1) }

    #[allow(clippy::too_many_arguments)]
    fn define(
        &mut self,
        node: NodeId,
        name: String,
        definition: tla_sany_semantic::nodes::OpDefData,
        args: Vec<ExprId>,
        nested: &mut HashMap<NodeId, ExprId>,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
        level: Level,
    ) -> Result<ExprId, ModelError> {
        let params = definition.params.clone().unwrap_or_default();
        if params.len() == args.len()
            && (definition.in_recursive
                || self.expanding.contains(&node)
                || self.cache.operators.contains_key(&node))
        {
            let mut scope = substitutions.clone();
            return self.recursive(
                node, name, definition, &params, args, &mut scope, context, depth, level,
            );
        }
        let Some(body) = definition.body else {
            return Err(self.error(format!("unsupported operator {name}")));
        };
        if params.len() != args.len() {
            return self
                .node(body, context, nested, depth + 1)
                .map(|id| self.program.at_least(id, level));
        }
        self.expanding.push(node);
        for (parameter, arg) in params.into_iter().zip(args) {
            nested.insert(parameter, arg);
        }
        let result = self.node(body, context, nested, depth + 1);
        self.expanding.pop();
        result.map(|id| self.program.at_least(id, level))
    }

    #[rustfmt::skip]
    pub(super) fn let_in(&mut self, data: tla_sany_semantic::nodes::LetInData, context: &Context, substitutions: &HashMap<NodeId, ExprId>, depth: usize) -> Result<ExprId, ModelError> {
        let saved_cache = self.cache.clone();
        let mut captures = substitutions.clone();
        for id in &data.op_defs {
            if let tla_sany_semantic::nodes::NodeData::OpDef(def) = self.model.tool.graph().node(*id).data.clone()
                && !def.in_recursive && def.params.as_ref().is_none_or(Vec::is_empty)
                && let Some(body) = def.body {
                let value = match self.function_value(*id, &def, context, &captures, depth)? {
                    Some(value) => value,
                    None => self.node(body, context, &captures, depth + 1)?,
                };
                captures.insert(*id, value);
            }
        }
        let substitutions = &captures;
        let mut defs = Vec::new();
        for id in data.op_defs {
            if self.model.tool.graph().node(id).kind != tla_sany_semantic::kinds::USER_DEFINED_OP_KIND { continue; }
            let tla_sany_semantic::nodes::NodeData::OpDef(def) = self.model.tool.graph().node(id).data.clone() else { return Err(self.error("invalid LET operator")); };
            if !def.in_recursive { continue; }
            let params = def.params.unwrap_or_default();
            let slots = (0..params.len()).map(|_| { let s = self.next_local; self.next_local += 1; s }).collect::<Vec<_>>();
            let op = self.program.operators.len(); self.cache.operators.insert(id, op);
            if let Some(name) = &def.name { self.cache.names.insert(name.as_str().to_owned(), op); }
            self.program.operators.push(tla_ir::Operator { name: def.name.map(|n| n.as_str().to_owned()).unwrap_or_default(), parameters: slots.clone(), body: 0 }); defs.push((op, params, slots, def.body));
        }
        for (op, params, slots, body) in defs {
            let mut nested = substitutions.clone(); for (param, slot) in params.into_iter().zip(slots) { nested.insert(param, self.program.push(Expr::Local(slot), Level::Constant)); }
            let body = self.node(body.ok_or_else(|| self.error(format!("missing LET body for {}", self.program.operators[op].name)))?, context, &nested, depth + 1)?; self.program.operators[op].body = body;
        }
        let body = data.body.ok_or_else(|| self.error("missing LET body"))?; let result = self.node(body, context, substitutions, depth + 1);
        self.cache = saved_cache;
        result
    }
}
