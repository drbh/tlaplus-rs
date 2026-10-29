//! Application lowering: operator applications, binders and operator
//! references, from the faithful semantic graph into IR expressions.

use super::Lower;
use crate::model::ModelError;
use std::collections::HashMap;
use tlaplus_ir::{Builtin, Expr, ExprId, Level, Literal, Program, Quantifier};
use tla_sany_semantic::NodeId;
use tla_sany_semantic::nodes::{NodeData, OpApplData};
use tla_tlc_eval::context::Context;

/// How an operator application may be read; see `operator_application`.
#[derive(Clone, Copy)]
enum Reading {
    Primitive,
    Definition,
}

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
    /// Lowers an operator application. The phases are ordered by what the
    /// operator can be: a name bound in scope or a state variable, a keyword
    /// the semantic graph keeps as an operator, an unbounded binder,
    /// `TLCEvalDefinition`, a definition already lowered, a bounded binder,
    /// and finally a definition or builtin applied to lowered operands.
    pub(super) fn application(
        &mut self,
        id: NodeId,
        data: OpApplData,
        level: Level,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        let operator = data
            .operator
            .ok_or_else(|| self.error("missing operator"))?;
        // The flag tells `checked_apply` that the lowered first operand came
        // from a source string literal, so `AssertError` may take its text as
        // the expected message.
        let literal_message =
            data.operands.first().and_then(|id| *id).is_some_and(|id| {
                matches!(self.model.tool.graph().node(id).data, NodeData::String(_))
            });
        // Operator parameters, instance substitutions, and state variables
        // rebound inside `ENABLED`; other state variables resolve by slot below.
        if let Some(expr) =
            self.program
                .scoped_binding(&operator, substitutions, &self.enabled_scope)
        {
            return self.scoped_invocation(
                expr,
                &data,
                literal_message,
                level,
                context,
                substitutions,
                depth,
            );
        }
        if let Some(slot) = self
            .model
            .processed
            .variables
            .iter()
            .position(|v| *v == operator)
        {
            let variable = Expr::Variable {
                slot,
                primed: false,
            };
            return Ok(self.program.push(variable, Level::State));
        }
        let name = self
            .model
            .tool
            .graph()
            .name(operator)
            .map_or_else(|| "<unnamed>".to_owned(), ToString::to_string);
        if let Some(expr) = self.keyword(&name, &data, context, substitutions, depth)? {
            return Ok(expr);
        }
        // Binders introduce locals that the operands are lowered under.
        let mut nested = substitutions.clone();
        if data.bounded_bound_symbols.is_none()
            && let Some(symbols) = &data.unbounded_bound_symbols
        {
            return self.unbounded_binder(
                &name,
                &data,
                symbols,
                &mut nested,
                context,
                depth,
                level,
            );
        }
        // `TLCEvalDefinition("Op")` names a definition in its string operand.
        if name == "TLCEvalDefinition" {
            return self.eval_definition(&data, context, substitutions, depth);
        }
        let bounds =
            self.bound_symbols(&name, &data, context, substitutions, &mut nested, depth)?;
        let args = self.operands(&name, &data, context, &nested, depth)?;
        // What the operator resolves to: a definition already lowered, a
        // binder, a definition or constant, or a builtin.
        let known = self
            .cache
            .operators
            .get(&operator)
            .or_else(|| self.cache.names.get(&name))
            .copied();
        if let Some(operator) = known {
            return Ok(self.program.push(Expr::Call { operator, args }, level));
        }
        if !bounds.is_empty() {
            return self.bounded_binder(&name, args, bounds, level);
        }
        self.operator_application(
            id,
            operator,
            &name,
            &data,
            args,
            literal_message,
            &mut nested,
            context,
            substitutions,
            depth,
            level,
        )
    }

    /// `TRUE`, `FALSE`, `ENABLED`, and priming or `UNCHANGED` inside `ENABLED`,
    /// which the semantic graph presents as ordinary operators.
    fn keyword(
        &mut self,
        name: &str,
        data: &OpApplData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<Option<ExprId>, ModelError> {
        Ok(match name {
            "TRUE" | "FALSE" => Some(self.program.push(
                Expr::Literal(Literal::Bool(name == "TRUE")),
                Level::Constant,
            )),
            "ENABLED" => match data.operands.as_slice() {
                [Some(action)] => Some(self.enabled(*action, context, substitutions, depth)?),
                _ => None,
            },
            "'" | "UNCHANGED" if !self.enabled_scope.is_empty() => {
                Some(self.scoped_step(name, data, context, substitutions, depth)?)
            }
            _ => None,
        })
    }

    /// The bound symbols of a binder as locals, collapsed to one tuple local
    /// for the constructors TLC evaluates over a product domain.
    fn bound_symbols(
        &mut self,
        name: &str,
        data: &OpApplData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        nested: &mut HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<Vec<(usize, ExprId)>, ModelError> {
        if data.bounded_bound_symbols.is_none() {
            return Ok(Vec::new());
        }
        let bounds = self.binders(data, context, substitutions, nested, depth)?;
        let product = matches!(
            name,
            "$FcnConstructor" | "$SetOfAll" | "$NonRecursiveFcnSpec" | "$RecursiveFcnSpec"
        );
        if bounds.len() > 1 && product {
            let (slot, domain) = self.collapse(bounds, nested);
            return Ok(vec![(slot, domain)]);
        }
        Ok(bounds)
    }

    /// The operands lowered under the binder's locals. A missing operand of
    /// `$Pair` is the `OTHER` branch of a `CASE`, whose guard is `TRUE`.
    fn operands(
        &mut self,
        name: &str,
        data: &OpApplData,
        context: &Context,
        nested: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<Vec<ExprId>, ModelError> {
        let mut args = Vec::with_capacity(data.operands.len());
        for arg in &data.operands {
            args.push(match arg {
                None if name == "$Pair" => self
                    .program
                    .push(Expr::Literal(Literal::Bool(true)), Level::Constant),
                None => return Err(self.error("missing operand")),
                Some(arg) => self.node(*arg, context, nested, depth + 1)?,
            });
        }
        Ok(args)
    }

    /// An operator applied to lowered operands. Two readings are tried in
    /// order: the operator as a primitive the IR has a node for or a builtin
    /// the faithful graph identifies, and the operator as a user definition.
    /// A bare name that is not a builtin by name tries the definition first,
    /// so a parameterless user definition, a temporal one in particular, is
    /// expanded rather than kept as a symbolic `Temporal` node; any other
    /// application tries the primitive reading first.
    #[allow(clippy::too_many_arguments)]
    fn operator_application(
        &mut self,
        id: NodeId,
        operator: NodeId,
        name: &str,
        data: &OpApplData,
        args: Vec<ExprId>,
        literal_message: bool,
        nested: &mut HashMap<NodeId, ExprId>,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
        level: Level,
    ) -> Result<ExprId, ModelError> {
        let bare = args.is_empty() && Builtin::from_name(name).is_none();
        let order = if bare {
            [Reading::Definition, Reading::Primitive]
        } else {
            [Reading::Primitive, Reading::Definition]
        };
        for reading in order {
            let found = match reading {
                Reading::Primitive => self.primitive_application(
                    operator,
                    name,
                    data,
                    args.clone(),
                    literal_message,
                    context,
                    level,
                )?,
                Reading::Definition => self.user_application(
                    operator,
                    name,
                    data,
                    args.clone(),
                    literal_message,
                    nested,
                    context,
                    substitutions,
                    depth,
                    level,
                )?,
            };
            if let Some(expr) = found {
                return Ok(expr);
            }
        }
        Err(self.error(format!(
            "operator {name} resolves to neither a value nor a definition at {:?}",
            self.model.tool.node_string(id)
        )))
    }

    /// The forms the IR has nodes for, temporal operators, and the builtins
    /// the faithful evaluator implements natively. `None` for anything else.
    #[allow(clippy::too_many_arguments)]
    fn primitive_application(
        &mut self,
        operator: NodeId,
        name: &str,
        data: &OpApplData,
        args: Vec<ExprId>,
        literal_message: bool,
        context: &Context,
        level: Level,
    ) -> Result<Option<ExprId>, ModelError> {
        let expression = match name {
            "'" => {
                let arg = *args
                    .first()
                    .ok_or_else(|| self.error("missing prime operand"))?;
                return self.program.prime(arg).map(Some).map_err(|e| self.error(e));
            }
            "$IfThenElse" => match args[..] {
                [condition, then_expr, else_expr] => Expr::If {
                    condition,
                    then_expr,
                    else_expr,
                },
                _ => return Err(self.error("IF expects a condition and two branches")),
            },
            "$SquareAct" | "$AngleAct" => {
                if args.len() != 2 {
                    return Err(self.error(format!("{name} expects an action and a subscript")));
                }
                self.step_action(name, &args)
            }
            // Temporal-level applications with operands, user-defined ones
            // included, stay symbolic as named `Expr::Temporal` nodes: the
            // bridge never evaluates temporal formulas, it only compares their
            // lowered shape with the native front end's, which expands them
            // itself. Bare non-builtin references were already tried as
            // definitions.
            _ if level == Level::Temporal => {
                Program::temporal_expr(name.to_owned(), args, Vec::new())
            }
            _ => {
                return self.builtin_application(
                    operator,
                    data,
                    args,
                    literal_message,
                    context,
                    level,
                );
            }
        };
        Ok(Some(self.program.push(expression, level)))
    }

    /// Inside `ENABLED`, priming and `UNCHANGED` relate an expression to its
    /// stand-in for the next state.
    fn scoped_step(
        &mut self,
        name: &str,
        data: &OpApplData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        let arg = data
            .operands
            .first()
            .copied()
            .flatten()
            .ok_or_else(|| self.error("missing scoped operand"))?;
        let expr = self.node(arg, context, substitutions, depth + 1)?;
        let primed = self.program.prime(expr).map_err(|e| self.error(e))?;
        if name == "'" {
            Ok(primed)
        } else {
            Ok(self.program.unchanged_between(expr, primed))
        }
    }

    /// A bound operator applied to operands: `ENABLED` of an action, or an
    /// invocation of the bound expression.
    #[allow(clippy::too_many_arguments)]
    fn scoped_invocation(
        &mut self,
        expr: ExprId,
        data: &OpApplData,
        literal_message: bool,
        level: Level,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
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
        self.program
            .invoke(expr, args, literal_message, level)
            .map_err(|e| self.error(e))
    }

    /// `\\E x : F`, `CHOOSE x : F` and function specs without a domain bind
    /// their symbols to fresh locals, one per symbol or one tuple for all.
    #[allow(clippy::too_many_arguments)]
    fn unbounded_binder(
        &mut self,
        name: &str,
        data: &OpApplData,
        symbols: &[NodeId],
        nested: &mut HashMap<NodeId, ExprId>,
        context: &Context,
        depth: usize,
        level: Level,
    ) -> Result<ExprId, ModelError> {
        let body = data
            .operands
            .first()
            .copied()
            .flatten()
            .ok_or_else(|| self.error("missing unbounded body"))?;
        // An unbounded binder has exactly one group of symbols, so one flag.
        let tuple = data
            .tuple_ors
            .as_ref()
            .is_some_and(|flags| flags.first().copied().unwrap_or(false));
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
        let mut body = self.node(body, context, nested, depth + 1)?;
        for slot in slots.into_iter().rev() {
            body = self.program.push(
                match name {
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
                        return Err(self.error(format!("unsupported unbounded operator {name}")));
                    }
                },
                level,
            );
        }
        Ok(body)
    }

    /// Binders over domains: quantifiers, set builders, function constructors
    /// and `CHOOSE`, nested innermost-last.
    fn bounded_binder(
        &mut self,
        name: &str,
        args: Vec<ExprId>,
        bounds: Vec<(usize, ExprId)>,
        level: Level,
    ) -> Result<ExprId, ModelError> {
        if level == Level::Temporal {
            return Ok(self.program.temporal(name.to_owned(), args, bounds, level));
        }
        let mut body = *args
            .first()
            .ok_or_else(|| self.error("missing bounded body"))?;
        for (slot, domain) in bounds.into_iter().rev() {
            let domain = Some(domain);
            let expr = match name {
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
                "$FcnConstructor" | "$NonRecursiveFcnSpec" => Expr::Function { slot, domain, body },
                "$RecursiveFcnSpec" => Expr::RecursiveFunction { slot, domain, body },
                "$BoundedChoose" => Expr::Choose { slot, domain, body },
                _ => {
                    return Err(self.error(format!("unsupported bounded operator {name}")));
                }
            };
            body = self.program.push(expr, level);
        }
        Ok(body)
    }

    /// `[A]_v` is `A \\/ UNCHANGED v`; `<<A>>_v` is `A /\\ ~UNCHANGED v`.
    fn step_action(&mut self, name: &str, args: &[ExprId]) -> Expr {
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

    /// Applies `node` as a builtin the faithful evaluator implements natively,
    /// recording which native method it was, or `None` when it is not one.
    /// `TLCCache` is lowered as `Memo` with the model's cache seed appended
    /// as the argument the cache keys on.
    fn builtin_application(
        &mut self,
        node: NodeId,
        data: &OpApplData,
        mut args: Vec<ExprId>,
        literal_message: bool,
        context: &Context,
        level: Level,
    ) -> Result<Option<ExprId>, ModelError> {
        let Some(mut op) = self.standard_builtin(node, context) else {
            return Ok(None);
        };
        if op == Builtin::TlcCache {
            let source = data
                .operands
                .first()
                .copied()
                .flatten()
                .ok_or_else(|| self.error("TLCCache is missing its expression"))?;
            args.push(self.cache_argument(source)?);
            op = Builtin::Memo;
        }
        let expr = self.builtin(op, args, literal_message, level)?;
        Ok(Some(self.method_origin(expr, node, context)))
    }

    /// A user-defined or module operator, resolved through the faithful
    /// context. In order: an operator record is invoked; a bare name whose
    /// value the evaluator already holds is that value; a definition is a
    /// native builtin, a function value, a body lowered in place, or a
    /// recursive operator called by index. `None` when the name has neither
    /// a usable value nor a definition to lower.
    /// `nested` starts as this site's substitutions and gains the INSTANCE
    /// substitutions `resolve` walks through.
    #[allow(clippy::too_many_arguments)]
    fn user_application(
        &mut self,
        operator: NodeId,
        name: &str,
        data: &OpApplData,
        args: Vec<ExprId>,
        literal_message: bool,
        nested: &mut HashMap<NodeId, ExprId>,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
        level: Level,
    ) -> Result<Option<ExprId>, ModelError> {
        let resolved = self.resolve(operator, context, substitutions, nested, depth)?;
        if let Some(value @ tla_tlc_value::Value::OpRcd(_)) = &resolved.value {
            let operator = self.value(value)?;
            return self
                .program
                .invoke(operator, args, literal_message, level)
                .map(Some)
                .map_err(|e| self.error(e));
        }
        // A value the faithful evaluator cannot hand over as a literal may
        // still be reachable through its definition, so that conversion
        // error is only reported once the definition path fails too.
        let converted = if args.is_empty() {
            resolved.value.as_ref().map(|value| self.value(value))
        } else {
            None
        };
        if let Some(Ok(expression)) = converted {
            return Ok(Some(expression));
        }
        let Some((node, definition)) = resolved.definition else {
            return converted.transpose();
        };
        // `node` is the resolved definition, which may be a standard operator
        // (through an INSTANCE, say) even when `operator` itself was not.
        if let Some(expr) =
            self.builtin_application(node, data, args.clone(), literal_message, context, level)?
        {
            return Ok(Some(expr));
        }
        // A parameterless definition may itself be a function value, which the
        // operands then index; definitions with parameters are expanded below.
        let params = definition.params.clone().unwrap_or_default();
        if params.is_empty()
            && let Some(value) =
                self.function_value(node, &definition, context, substitutions, depth)?
        {
            if args.is_empty() {
                return Ok(Some(value));
            }
            let [argument] = args[..] else {
                return Err(self.error(format!(
                    "function {name} applied to {} arguments, expected 1",
                    args.len()
                )));
            };
            let applied = Expr::Apply {
                op: Builtin::Apply,
                args: vec![value, argument],
            };
            return Ok(Some(self.program.push(applied, level)));
        }
        if args.is_empty() && !params.is_empty() {
            // A bare reference to an operator with parameters. Without a
            // body there is nothing to lower. With one, the body is lowered
            // as it stands: nothing binds the parameters, so only bodies that
            // do not use them survive, and one that does fails to resolve.
            let Some(body) = definition.body else {
                return converted.transpose();
            };
            return self
                .node(body, context, nested, depth + 1)
                .map(|id| Some(self.program.at_least(id, level)));
        }
        if args.is_empty() && definition.body.is_none() {
            return converted.transpose();
        }
        if params.len() != args.len() {
            return Err(self.error(format!(
                "{name} applied to {} arguments, expected {}",
                args.len(),
                params.len()
            )));
        }
        self.define(
            node,
            name.to_owned(),
            definition,
            &params,
            args,
            nested,
            context,
            substitutions,
            depth,
            level,
        )
        .map(Some)
    }

    fn eval_definition(
        &mut self,
        data: &OpApplData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        let Some(Some(id)) = data.operands.first() else {
            return self.invalid_tlc_eval();
        };
        let NodeData::String(text) = self.model.tool.graph().node(*id).data.clone() else {
            return self.invalid_tlc_eval();
        };
        let Some(def_id) = self
            .model
            .tool
            .module_info(self.model.tool.root_module())
            .and_then(|info| {
                info.op_defs
                    .iter()
                    .find(|&&d| {
                        self.model
                            .tool
                            .graph()
                            .name(d)
                            .is_some_and(|n| n.as_str() == text.value.as_str())
                    })
                    .copied()
            })
        else {
            return self.invalid_tlc_eval();
        };
        let NodeData::OpDef(def) = self.model.tool.graph().node(def_id).data.clone() else {
            return self.invalid_tlc_eval();
        };
        if def.params.as_ref().is_some_and(|params| !params.is_empty()) {
            return self.invalid_tlc_eval();
        }
        self.node(
            def.body
                .ok_or_else(|| self.error("definition has no body"))?,
            context,
            substitutions,
            depth + 1,
        )
    }

    /// Lowers a definition applied to `args`, one per parameter: recursive
    /// definitions become IR operators called by index; others are expanded
    /// in place with the parameters bound to the arguments.
    #[allow(clippy::too_many_arguments)]
    fn define(
        &mut self,
        node: NodeId,
        name: String,
        definition: tla_sany_semantic::nodes::OpDefData,
        params: &[NodeId],
        args: Vec<ExprId>,
        nested: &mut HashMap<NodeId, ExprId>,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
        level: Level,
    ) -> Result<ExprId, ModelError> {
        debug_assert_eq!(params.len(), args.len());
        // A definition SANY marked recursive, one re-entered while its own
        // body is being expanded (recursion SANY did not flag), or one already
        // lowered as an IR operator, is called by index rather than expanded.
        if definition.in_recursive
            || self.expanding.contains(&node)
            || self.cache.operators.contains_key(&node)
        {
            // `node` is the resolved definition and may differ from the
            // operator `application` looked up, so the cache is checked again.
            // The IR operator is shared by every call site, so its body is
            // lowered under the module substitutions alone, without the
            // INSTANCE substitutions `resolve` added to `nested` for this
            // site; the recursive operator handles those itself.
            let mut scope = substitutions.clone();
            return self.recursive(
                node, name, definition, params, args, &mut scope, context, depth, level,
            );
        }
        // Applied with arguments, so a body is required.
        let Some(body) = definition.body else {
            return Err(self.error(format!("operator {name} has no body to lower")));
        };
        self.expanding.push(node);
        for (&parameter, arg) in params.iter().zip(args) {
            nested.insert(parameter, arg);
        }
        let result = self.node(body, context, nested, depth + 1);
        self.expanding.pop();
        result.map(|id| self.program.at_least(id, level))
    }

    pub(super) fn let_in(
        &mut self,
        data: tla_sany_semantic::nodes::LetInData,
        context: &Context,
        substitutions: &HashMap<NodeId, ExprId>,
        depth: usize,
    ) -> Result<ExprId, ModelError> {
        let saved_cache = self.cache.clone();
        let mut captures = substitutions.clone();
        for id in &data.op_defs {
            if let tla_sany_semantic::nodes::NodeData::OpDef(def) =
                self.model.tool.graph().node(*id).data.clone()
                && !def.in_recursive
                && def.params.as_ref().is_none_or(Vec::is_empty)
                && let Some(body) = def.body
            {
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
            if self.model.tool.graph().node(id).kind
                != tla_sany_semantic::kinds::USER_DEFINED_OP_KIND
            {
                continue;
            }
            let tla_sany_semantic::nodes::NodeData::OpDef(def) =
                self.model.tool.graph().node(id).data.clone()
            else {
                return Err(self.error("invalid LET operator"));
            };
            if !def.in_recursive {
                continue;
            }
            let params = def.params.unwrap_or_default();
            let slots = (0..params.len())
                .map(|_| {
                    let s = self.next_local;
                    self.next_local += 1;
                    s
                })
                .collect::<Vec<_>>();
            let op = self.program.operators.len();
            self.cache.operators.insert(id, op);
            if let Some(name) = &def.name {
                self.cache.names.insert(name.as_str().to_owned(), op);
            }
            self.program.operators.push(tlaplus_ir::Operator {
                name: def.name.map(|n| n.as_str().to_owned()).unwrap_or_default(),
                parameters: slots.clone(),
                body: 0,
            });
            defs.push((op, params, slots, def.body));
        }
        for (op, params, slots, body) in defs {
            let mut nested = substitutions.clone();
            for (param, slot) in params.into_iter().zip(slots) {
                nested.insert(param, self.program.push(Expr::Local(slot), Level::Constant));
            }
            let body = self.node(
                body.ok_or_else(|| {
                    self.error(format!(
                        "missing LET body for {}",
                        self.program.operators[op].name
                    ))
                })?,
                context,
                &nested,
                depth + 1,
            )?;
            self.program.operators[op].body = body;
        }
        let body = data.body.ok_or_else(|| self.error("missing LET body"))?;
        let result = self.node(body, context, substitutions, depth + 1);
        self.cache = saved_cache;
        result
    }
}
