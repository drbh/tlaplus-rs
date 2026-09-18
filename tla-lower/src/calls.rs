use crate::{Binding, Env, Lower};
use tla_ir::{Builtin as B, Expr as I, ExprId, Level, Operator};
use tla_modules::Error;
use tla_syntax::{Definition, Expr};

/// Where a called definition lives: a LET scope, or the module.
#[derive(Clone, Copy, PartialEq)]
enum Home {
    Scope(usize),
    Module(usize),
}

impl Lower<'_> {
    pub(crate) fn call(&mut self, name: &str, args: &[Expr]) -> Result<ExprId, Error> {
        match (name, args) {
            ("$Error", [Expr::String(message)]) => return Err(Error(message.clone())),
            ("$Lambda", _) => return Err(Error("LAMBDA is only an operator argument".into())),
            ("$AngleAct", [action, variables]) => {
                let action = self.expr(action)?;
                let variables = self.expr(variables)?;
                let unchanged = self.apply(B::Unchanged, vec![variables]);
                let changed = self.apply(B::Not, vec![unchanged]);
                return Ok(self.apply(B::And, vec![action, changed]));
            }
            ("WF_" | "SF_", _) => {
                let args = args
                    .iter()
                    .map(|a| self.expr(a))
                    .collect::<Result<_, _>>()?;
                return Ok(self
                    .program
                    .temporal(name.into(), args, Vec::new(), Level::Temporal));
            }
            _ => {}
        }
        let literal_message = matches!(args.first(), Some(Expr::String(_)));
        if let Some(Binding::Node(function)) = self.locals.get(name) {
            let args = self.arguments(args, &[])?;
            let level = self.level_of(&args);
            return self
                .program
                .invoke(function, args, literal_message, level)
                .map_err(|e| Error(e.to_string()));
        }
        let home = self.home(name);
        let definition = match home {
            Some(Home::Scope(index)) => Some(self.scoped[index].definitions[name].clone()),
            Some(Home::Module(index)) => Some(self.module.definitions[index].clone()),
            None => None,
        };
        let native = match home {
            None => true,
            Some(Home::Module(index)) => self.native(index).is_some(),
            Some(Home::Scope(_)) => false,
        };
        if native && let Some(op) = builtin(crate::base(name)) {
            let args = self.arguments(args, &[])?;
            if matches!(op, B::AssertError | B::TlcCache) {
                let level = self.level_of(&args);
                return self
                    .program
                    .checked_apply(op, args, literal_message, level)
                    .map_err(|e| Error(e.to_string()));
            }
            return Ok(self.apply(op, args));
        }
        let (home, definition) = match (home, definition) {
            (Some(home), Some(definition)) => (home, definition),
            _ => return Err(Error(format!("unresolved operator {name}"))),
        };
        if definition.parameters.len() != args.len() {
            return Err(Error(format!("wrong number of arguments for {name}")));
        }
        let higher_order = definition.arities.iter().any(|arity| *arity > 0);
        // Within ENABLED over substituted instance variables, bodies are lowered
        // at the call so that the substitution's temporary variables apply.
        if let Home::Module(index) = home
            && !higher_order
            && self.enabled_scope.is_empty()
        {
            self.definition(index)?;
            if self.statuses[index] == 2 && is_function(&definition) {
                return Ok(self.program.operators[index].body);
            }
            let args = self.arguments(args, &[])?;
            let body = (self.statuses[index] == 2).then(|| self.program.operators[index].body);
            return Ok(self.invoke(index, name, body, args));
        }
        self.expand(name, home, &definition, args)
    }

    fn home(&self, name: &str) -> Option<Home> {
        let scoped = self
            .scoped
            .iter()
            .rposition(|scope| scope.definitions.contains_key(name));
        scoped.map(Home::Scope).or_else(|| {
            self.module
                .definitions
                .iter()
                .position(|d| d.name == name)
                .map(Home::Module)
        })
    }

    /// Lowers a LET or higher-order operator at its call site, with operator
    /// parameters bound to their arguments and the others as fresh slots.
    fn expand(
        &mut self,
        name: &str,
        home: Home,
        definition: &Definition,
        args: &[Expr],
    ) -> Result<ExprId, Error> {
        let args = self.arguments(args, &definition.arities)?;
        if let Some(&(_, _, operator)) = self
            .active
            .iter()
            .find(|(active, n, _)| *active == key(home) && n == name)
        {
            if !definition.recursive {
                return Err(Error(format!("recursive local operator {name}")));
            }
            let ordinary = ordinary(definition, args);
            return Ok(self.invoke(operator, name, None, ordinary));
        }
        if is_function(definition) && args.is_empty() {
            let (env, shadowed) = self.enter(home);
            self.function_name = Some(name.into());
            let value = self.expr(&definition.body);
            self.leave(env, shadowed);
            return value;
        }
        let operator = self.program.operators.len();
        self.program.operators.push(Operator {
            name: format!("{name}@{operator}"),
            parameters: Vec::new(),
            body: 0,
        });
        self.active.push((key(home), name.into(), operator));
        let (env, shadowed) = self.enter(home);
        let mut parameters = Vec::new();
        for ((parameter, arity), argument) in definition
            .parameters
            .iter()
            .zip(&definition.arities)
            .zip(&args)
        {
            if *arity > 0 {
                self.locals.node(parameter, *argument);
            } else {
                let slot = self.fresh();
                self.locals.slot(parameter, slot);
                parameters.push(slot);
            }
        }
        self.program.operators[operator].parameters = parameters;
        let body = self.expr(&definition.body);
        self.leave(env, shadowed);
        self.active.pop();
        let body = body?;
        if definition.recursive {
            self.observe(name, body);
        }
        self.program.operators[operator].body = body;
        let ordinary = ordinary(definition, args);
        Ok(self.invoke(operator, name, Some(body), ordinary))
    }

    /// Enters the lexical scope of a definition: a LET scope's environment, or the
    /// module's, which sees no LET definitions.
    fn enter(&mut self, home: Home) -> (Env, Vec<crate::Scope>) {
        let (env, keep) = match home {
            Home::Scope(index) => (self.scoped[index].locals.clone(), index + 1),
            Home::Module(_) => (Env::default(), 0),
        };
        (
            std::mem::replace(&mut self.locals, env),
            self.scoped.split_off(keep),
        )
    }

    fn leave(&mut self, env: Env, shadowed: Vec<crate::Scope>) {
        self.locals = env;
        self.scoped.extend(shadowed);
    }

    /// Lowers call arguments; operators passed as arguments become lambdas.
    fn arguments(&mut self, args: &[Expr], arities: &[usize]) -> Result<Vec<ExprId>, Error> {
        args.iter()
            .enumerate()
            .map(|(index, arg)| {
                let arity = arities.get(index).copied().unwrap_or(0);
                if arity > 0 || self.is_operator(arg) {
                    self.operator_argument(arg, arity)
                } else {
                    self.expr(arg)
                }
            })
            .collect()
    }

    fn is_operator(&self, arg: &Expr) -> bool {
        match arg {
            Expr::Call(name, _) => name == "$Lambda",
            Expr::Name(name) => match self.locals.get(name) {
                Some(Binding::Node(id)) => matches!(
                    self.program.nodes[id].expr,
                    I::Lambda { .. } | I::BuiltinOperator(_)
                ),
                Some(Binding::Slot(_)) => false,
                None => match self.home(name) {
                    Some(Home::Scope(index)) => {
                        !self.scoped[index].definitions[name].parameters.is_empty()
                    }
                    Some(Home::Module(index)) => {
                        !self.module.definitions[index].parameters.is_empty()
                    }
                    None => {
                        name != "@"
                            && !name.starts_with(|c: char| c.is_alphanumeric() || c == '_')
                            && builtin(name).is_some()
                    }
                },
            },
            _ => false,
        }
    }

    fn operator_argument(&mut self, arg: &Expr, arity: usize) -> Result<ExprId, Error> {
        if let Expr::Call(name, parts) = arg
            && name == "$Lambda"
            && let [Expr::Tuple(names), body] = parts.as_slice()
        {
            let saved = self.locals.clone();
            let mut parameters = Vec::new();
            for name in names {
                let Expr::Name(name) = name else {
                    return Err(Error("invalid LAMBDA parameter".into()));
                };
                let slot = self.fresh();
                self.locals.slot(name, slot);
                parameters.push(slot);
            }
            let body = self.expr(body);
            self.locals = saved;
            let body = body?;
            let level = self.program.nodes[body].level;
            return Ok(self.program.push(I::Lambda { parameters, body }, level));
        }
        let Expr::Name(name) = arg else {
            return Err(Error("operator argument must be an operator".into()));
        };
        if let Some(Binding::Node(id)) = self.locals.get(name) {
            return Ok(id);
        }
        let parameters = match self.home(name) {
            Some(Home::Scope(index)) => self.scoped[index].definitions[name].parameters.len(),
            Some(Home::Module(index)) => self.module.definitions[index].parameters.len(),
            None => arity,
        };
        let native = self
            .home(name)
            .is_none_or(|home| matches!(home, Home::Module(index) if self.native(index).is_some()));
        if native && let Some(op) = builtin(crate::base(name)) {
            return Ok(self.program.push(I::BuiltinOperator(op), Level::Constant));
        }
        let slots: Vec<_> = (0..parameters).map(|_| self.fresh()).collect();
        let saved = self.locals.clone();
        let names: Vec<_> = slots.iter().map(|slot| format!("$arg{slot}")).collect();
        for (name, slot) in names.iter().zip(&slots) {
            self.locals.slot(name, *slot);
        }
        let args: Vec<_> = names.into_iter().map(Expr::Name).collect();
        let body = self.call(name, &args);
        self.locals = saved;
        let body = body?;
        let level = self.program.nodes[body].level;
        Ok(self.program.push(
            I::Lambda {
                parameters: slots,
                body,
            },
            level,
        ))
    }

    pub(crate) fn level_of(&self, args: &[ExprId]) -> Level {
        args.iter()
            .map(|id| self.program.nodes[*id].level)
            .max()
            .unwrap_or_default()
    }

    /// A call whose level is its arguments' and its body's (or the assumed level of
    /// a recursive operator whose body is still being lowered).
    fn invoke(
        &mut self,
        operator: usize,
        name: &str,
        body: Option<ExprId>,
        args: Vec<ExprId>,
    ) -> ExprId {
        let level = self.level_of(&args).max(self.body_level(name, body));
        self.program.push(I::Call { operator, args }, level)
    }
}

fn key(home: Home) -> usize {
    match home {
        Home::Scope(index) => index,
        Home::Module(_) => usize::MAX,
    }
}

/// The arguments bound to slots, excluding operator arguments.
fn ordinary(definition: &Definition, args: Vec<ExprId>) -> Vec<ExprId> {
    args.into_iter()
        .zip(&definition.arities)
        .filter(|(_, arity)| **arity == 0)
        .map(|(arg, _)| arg)
        .collect()
}

pub(crate) fn is_function(definition: &Definition) -> bool {
    matches!(&definition.body, Expr::Bound { kind, .. } if kind == "recursive function")
}

pub(crate) fn builtin(name: &str) -> Option<B> {
    Some(match name {
        "$Case" => B::Case,
        "$Except" => B::Except,
        "/\\" => B::And,
        "\\/" => B::Or,
        "=>" => B::Implies,
        "<=>" => B::Equivalent,
        "=" => B::Eq,
        "#" | "/=" => B::Ne,
        "<" => B::Lt,
        "<=" => B::Le,
        ">" => B::Gt,
        ">=" => B::Ge,
        "+" => B::Add,
        "-" => B::Sub,
        "*" => B::Mul,
        "\\div" => B::Div,
        "%" => B::Mod,
        "^" => B::Pow,
        "\\in" => B::In,
        "\\notin" => B::NotIn,
        "\\subseteq" => B::SubsetEq,
        "\\cup" | "\\union" => B::Union,
        "\\cap" | "\\intersect" => B::Intersection,
        "\\" => B::Difference,
        ".." => B::Range,
        "->" => B::FunctionSet,
        "\\X" => B::Cartesian,
        "(+)" => B::BagAdd,
        "(-)" => B::BagDifference,
        "Cardinality" => B::Cardinality,
        "Len" => B::Len,
        "Head" => B::Head,
        "Tail" => B::Tail,
        "Append" => B::Append,
        "SubSeq" => B::Subsequence,
        "SelectSeq" => B::SelectSeq,
        _ => return B::from_name(name),
    })
}
