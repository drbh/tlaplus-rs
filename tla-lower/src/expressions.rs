use crate::{Binding, Lower, Scope};
use std::collections::BTreeMap;
use tla_ir::{Builtin as B, Expr as I, ExprId, Level, Literal, Operator, Quantifier};
use tla_modules::Error;
use tla_syntax::{Definition, Expr};

impl Lower<'_> {
    pub(crate) fn expr(&mut self, expression: &Expr) -> Result<ExprId, Error> {
        // Inside ENABLED, an expression the action primes acts as a fresh
        // next-state variable (see the ENABLED arm below); it lowers to its
        // current value paired with that variable.
        if let Some(&(_, slot)) = self.enabled_scope.iter().find(|(e, _)| e == expression) {
            let scope = std::mem::take(&mut self.enabled_scope);
            let current = self.expr(expression);
            self.enabled_scope = scope;
            let current = current?;
            let next = self
                .program
                .push(I::Variable { slot, primed: true }, Level::Action);
            let level = self.program.nodes[current].level;
            return Ok(self.program.push(I::Scoped { current, next }, level));
        }
        Ok(match expression {
            Expr::Name(name) if name == "@" => self.program.push(I::At, Level::Constant),
            Expr::Let(definitions, body) => self.let_in(definitions, body)?,
            Expr::Record(fields) => {
                let mut pairs = Vec::new();
                for (key, value) in fields {
                    let key = self.expr(&Expr::String(key.clone()))?;
                    let value = self.expr(value)?;
                    pairs.push(self.apply(B::Tuple, vec![key, value]));
                }
                self.apply(B::Record, pairs)
            }
            Expr::Stutter(action, variables) => {
                let action = self.expr(action)?;
                let variables = self.expr(variables)?;
                let unchanged = self.apply(B::Unchanged, vec![variables]);
                self.apply(B::Or, vec![action, unchanged])
            }
            Expr::Integer(value) => self.literal(Literal::Int(
                i32::try_from(*value).map_err(|_| Error("integer outside TLC range".into()))?,
            )),
            Expr::String(text) => {
                let ordinal = self.symbol(text);
                self.literal(Literal::String {
                    ordinal,
                    text: text.clone(),
                })
            }
            Expr::Name(name) if name == "TRUE" || name == "FALSE" => {
                self.literal(Literal::Bool(name == "TRUE"))
            }
            Expr::Name(name) if name == "BOOLEAN" => self.literal(Literal::Set(vec![
                Literal::Bool(false),
                Literal::Bool(true),
            ])),
            Expr::Name(name) => match self.locals.get(name) {
                Some(Binding::Slot(slot)) => self.program.push(I::Local(slot), Level::Constant),
                Some(Binding::Node(id)) => id,
                None => {
                    if let Some(slot) = self.module.variables.iter().position(|v| v == name) {
                        let variable = I::Variable {
                            slot,
                            primed: false,
                        };
                        self.program.push(variable, Level::State)
                    } else if let Some(slot) = self.module.constants.iter().position(|v| v == name)
                    {
                        self.program.push(I::Constant(slot), Level::Constant)
                    } else {
                        self.call(name, &[])?
                    }
                }
            },
            Expr::Call(name, args) => self.call(name, args)?,
            Expr::Unary(op, child) if op == "'" => self.prime(child)?,
            Expr::Unary(op, child) if matches!(op.as_str(), "[]" | "<>") => {
                let child = self.expr(child)?;
                self.program
                    .temporal(op.clone(), vec![child], Vec::new(), Level::Temporal)
            }
            Expr::Unary(op, child) if op == "ENABLED" => self.enabled(child)?,
            Expr::Unary(op, child) => {
                let op = match op.as_str() {
                    "~" => B::Not,
                    "-" => B::Neg,
                    "SUBSET" => B::PowerSet,
                    "UNION" => B::UnionAll,
                    "DOMAIN" => B::Domain,
                    "UNCHANGED" => B::Unchanged,
                    other => return self.call(other, &[child.as_ref().clone()]),
                };
                let child = self.expr(child)?;
                self.apply(op, vec![child])
            }
            Expr::Binary(op, left, right) if matches!(op.as_str(), "~>" | "-+->") => {
                let left = self.expr(left)?;
                let right = self.expr(right)?;
                self.program
                    .temporal(op.clone(), vec![left, right], Vec::new(), Level::Temporal)
            }
            Expr::Binary(op, left, right) => {
                self.call(op, &[left.as_ref().clone(), right.as_ref().clone()])?
            }
            Expr::Set(values) | Expr::Tuple(values) => {
                let args = values
                    .iter()
                    .map(|v| self.expr(v))
                    .collect::<Result<_, _>>()?;
                let op = if matches!(expression, Expr::Set(_)) {
                    B::Set
                } else {
                    B::Tuple
                };
                self.apply(op, args)
            }
            Expr::Index(function, key) => {
                let function = self.expr(function)?;
                let key = self.expr(key)?;
                self.apply(B::Apply, vec![function, key])
            }
            Expr::If(condition, yes, no) => {
                let condition = self.expr(condition)?;
                let then_expr = self.expr(yes)?;
                let else_expr = self.expr(no)?;
                let level = self.level_of(&[condition, then_expr, else_expr]);
                self.program.push(
                    I::If {
                        condition,
                        then_expr,
                        else_expr,
                    },
                    level,
                )
            }
            Expr::Bound {
                kind,
                names,
                tuple,
                domain,
                body,
            } => self.bound(kind, names, *tuple, domain.as_deref(), body)?,
        })
    }

    fn bound(
        &mut self,
        kind: &str,
        names: &[String],
        tuple: bool,
        domain: Option<&Expr>,
        body: &Expr,
    ) -> Result<ExprId, Error> {
        // Only the outermost binder of a recursive function definition may
        // claim its name; any binder nested inside sees none.
        let this = self.function_name.take();
        let domain = domain.map(|d| self.expr(d)).transpose()?;
        let saved = self.locals.clone();
        let slot = self.fresh();
        let domain = match (tuple, domain) {
            (true, Some(domain)) => {
                for (index, name) in names.iter().enumerate() {
                    let component = self.program.tuple_local(slot, index, names.len());
                    self.locals.node(name, component);
                }
                // Binders that test their elements restrict the domain to the
                // tuples the pattern accepts; quantifiers require an exact
                // arity, as TLC does, while CHOOSE and filters allow longer
                // tuples. Constructors (`map`, `function`) keep the whole
                // domain, because every element must be mapped.
                let exact = matches!(kind, "\\E" | "\\A");
                match kind {
                    "\\E" | "\\A" | "CHOOSE" | "filter" => {
                        Some(self.program.tuple_domain(slot, domain, names.len(), exact))
                    }
                    _ => Some(domain),
                }
            }
            (_, domain) => {
                for name in names {
                    self.locals.slot(name, slot);
                }
                domain
            }
        };
        let domain_level = domain.map_or(Level::Constant, |d| self.program.nodes[d].level);
        if kind == "recursive function" {
            let operator = self.program.operators.len();
            self.program.operators.push(Operator {
                name: format!("{}[]", this.as_deref().unwrap_or("function")),
                parameters: vec![slot],
                body: 0,
            });
            let local = self.program.push(I::Local(slot), Level::Constant);
            let apply = self.program.push(
                I::Call {
                    operator,
                    args: vec![local],
                },
                Level::Constant,
            );
            let value = self.program.push(
                I::RecursiveFunction {
                    slot,
                    domain,
                    body: apply,
                },
                domain_level,
            );
            if let Some(name) = &this {
                self.locals.node(name, value);
            }
            let body = self.expr(body);
            self.locals = saved;
            let body = body?;
            self.program.operators[operator].body = body;
            let level = domain_level.max(self.program.nodes[body].level);
            self.program.nodes[apply].level = level;
            self.program.nodes[value].level = level;
            return Ok(value);
        }
        let body = self.expr(body);
        self.locals = saved;
        let body = body?;
        let level = domain_level.max(self.program.nodes[body].level);
        let expr = match kind {
            "CHOOSE" => I::Choose { slot, domain, body },
            "function" => I::Function { slot, domain, body },
            "\\EE" | "\\AA" => {
                // Temporal quantifiers have no domain; the binding records the
                // slot with the body standing in, since the temporal layer
                // only needs to know which local the formula closes over.
                return Ok(self.program.temporal(
                    kind.into(),
                    vec![body],
                    vec![(slot, body)],
                    Level::Temporal,
                ));
            }
            "\\E" | "\\A" | "filter" | "map" => I::Quantify {
                kind: match kind {
                    "\\E" => Quantifier::Exists,
                    "\\A" => Quantifier::ForAll,
                    "filter" => Quantifier::SetFilter,
                    _ => Quantifier::SetMap,
                },
                slot,
                domain,
                body,
            },
            _ => return Err(Error(format!("unsupported binder {kind}"))),
        };
        Ok(self.program.push(expr, level))
    }
}

impl Lower<'_> {
    /// `LET defs IN body`: the definitions are visible to the body and to
    /// each other, as a scope over the enclosing locals.
    fn let_in(&mut self, definitions: &[Definition], body: &Expr) -> Result<ExprId, Error> {
        let scope: BTreeMap<_, _> = definitions
            .iter()
            .map(|d| (d.name.clone(), d.clone()))
            .collect();
        if scope.len() != definitions.len() {
            return Err(Error("duplicate LET definition".into()));
        }
        self.scoped.push(Scope {
            definitions: scope,
            locals: self.locals.clone(),
        });
        let body = self.expr(body);
        self.scoped.pop();
        body
    }

    /// `e'`: a state variable primes to its next-state slot; anything else is
    /// lowered and then primed structurally, which raises it to action level.
    fn prime(&mut self, child: &Expr) -> Result<ExprId, Error> {
        if let Expr::Name(name) = child
            && self.locals.get(name).is_none()
            && let Some(slot) = self.module.variables.iter().position(|v| v == name)
        {
            return Ok(self
                .program
                .push(I::Variable { slot, primed: true }, Level::Action));
        }
        let current = self.expr(child)?;
        let next = self
            .program
            .prime(current)
            .map_err(|e| Error(e.to_string()))?;
        self.program.nodes[next].level = self.program.nodes[next].level.max(Level::Action);
        Ok(next)
    }

    /// `ENABLED A`.
    fn enabled(&mut self, child: &Expr) -> Result<ExprId, Error> {
        // Like TLC, every primed compound expression inside ENABLED (an
        // instance variable substituted by an expression, say) acts as
        // a fresh variable of the next state.
        let depth = self.enabled_scope.len();
        let mut primed = Vec::new();
        self.primed_compounds(child, &mut primed, &mut Vec::new());
        for compound in primed {
            if !self.enabled_scope.iter().any(|(e, _)| *e == compound) {
                let slot = self.program.variables.len() + self.program.enabled_variables;
                self.program.enabled_variables += 1;
                self.enabled_scope.push((compound, slot));
            }
        }
        let action = self.expr(child);
        self.enabled_scope.truncate(depth);
        let action = action?;
        let enabled = I::Apply {
            op: B::Enabled,
            args: vec![action],
        };
        Ok(self.program.push(enabled, Level::State))
    }

    /// Operands of primes that are not names, such as `(f[k])'`, looking
    /// through the definitions the expression calls.
    fn primed_compounds(
        &self,
        expression: &Expr,
        output: &mut Vec<Expr>,
        active: &mut Vec<String>,
    ) {
        let mut work = vec![expression.clone()];
        while let Some(expression) = work.pop() {
            if let Expr::Name(name) | Expr::Call(name, _) = &expression
                && !active.contains(name)
                && let Some(definition) = self.module.definitions.iter().find(|d| &d.name == name)
                && !definition.standard
            {
                let args = match &expression {
                    Expr::Call(_, args) => args.clone(),
                    _ => Vec::new(),
                };
                let replacements = definition
                    .parameters
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect();
                if let Ok(body) = tla_syntax::substitute(&definition.body, &replacements) {
                    active.push(name.clone());
                    self.primed_compounds(&body, output, active);
                    active.pop();
                }
            }
            primed_operands(&expression, output, &mut work);
        }
    }
}

/// Collects one expression's primed compound operands and queues its children.
fn primed_operands(expression: &Expr, output: &mut Vec<Expr>, work: &mut Vec<Expr>) {
    {
        match expression {
            Expr::Unary(op, child) if op == "'" && !matches!(**child, Expr::Name(_)) => {
                if !output.contains(child) {
                    output.push(child.as_ref().clone());
                }
            }
            Expr::Unary(_, a) => work.push(a.as_ref().clone()),
            Expr::Binary(_, a, b) | Expr::Index(a, b) | Expr::Stutter(a, b) => {
                work.extend([a.as_ref().clone(), b.as_ref().clone()])
            }
            Expr::If(a, b, c) => work.extend([a, b, c].map(|e| e.as_ref().clone())),
            Expr::Call(_, args) | Expr::Set(args) | Expr::Tuple(args) => {
                work.extend(args.iter().cloned())
            }
            Expr::Record(fields) => work.extend(fields.iter().map(|(_, v)| v.clone())),
            Expr::Let(definitions, body) => {
                work.push(body.as_ref().clone());
                work.extend(definitions.iter().map(|d| d.body.clone()));
            }
            Expr::Bound { domain, body, .. } => {
                work.extend(domain.as_deref().cloned());
                work.push(body.as_ref().clone());
            }
            Expr::Integer(_) | Expr::String(_) | Expr::Name(_) => {}
        }
    }
}
