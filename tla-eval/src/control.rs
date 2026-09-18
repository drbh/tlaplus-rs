//! The structured forms of the IR: operator calls, binders over enumerated
//! domains, `EXCEPT`, `CASE` and the string forms of sequence operators.
use crate::{Error, Evaluator, bind, deferred, functional};
use tla_ir::{Builtin, Expr, ExprId, Quantifier};
use tla_values::Value;

type Locals = Vec<Option<Value>>;

impl Evaluator<'_> {
    /// Applies a user operator to evaluated arguments, binding its parameters.
    pub(crate) fn call(
        &self,
        operator: usize,
        args: &[ExprId],
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        let op = self
            .program
            .operators
            .get(operator)
            .ok_or_else(|| Error("unknown operator".into()))?;
        if op.parameters.len() != args.len() {
            return Err(Error("operator arity mismatch".into()));
        }
        let values = args
            .iter()
            .map(|id| self.eval(*id, locals, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        let mut nested = locals.clone();
        for (&slot, value) in op.parameters.iter().zip(values) {
            bind(&mut nested, slot, value);
        }
        self.eval(op.body, &mut nested, depth + 1)
    }

    /// `[f EXCEPT ![path] = value, ...]`: each update is a `<<path, value>>`
    /// pair, and `@` inside a value names the entry being replaced. Updates
    /// to keys outside the domain are ignored, as TLC does.
    pub(crate) fn except_updates(
        &self,
        args: &[ExprId],
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        let (&base, updates) = args
            .split_first()
            .ok_or_else(|| Error("EXCEPT without base".into()))?;
        let mut function = self.eval(base, locals, depth + 1)?;
        if !matches!(function, Value::Function(_)) {
            return Err(Error("EXCEPT requires a function".into()));
        }
        for update in updates {
            let (path, value) = self.pair(*update, "EXCEPT update")?;
            let path = self.eval(path, locals, depth + 1)?.sequence()?;
            let mut previous = function.clone();
            let mut found = true;
            for key in &path {
                match previous.apply(key) {
                    Ok(value) => previous = value,
                    Err(_) => {
                        found = false;
                        break;
                    }
                }
            }
            if found {
                let evaluator = Evaluator {
                    at: Some(previous),
                    ..self.clone()
                };
                let replacement = evaluator.eval(value, locals, depth + 1)?;
                function = function.except(&path, replacement)?;
            }
        }
        Ok(function)
    }

    /// `CASE c1 -> e1 [] c2 -> e2 ...`, each branch a `<<condition, value>>` pair.
    pub(crate) fn case(
        &self,
        branches: &[ExprId],
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        for branch in branches {
            let (condition, value) = self.pair(*branch, "CASE branch")?;
            if self.eval(condition, locals, depth + 1)?.boolean()? {
                return self.eval(value, locals, depth + 1);
            }
        }
        Err(Error("CASE has no true condition".into()))
    }

    /// The two operands of a tuple node the lowering uses as a pair.
    fn pair(&self, id: ExprId, what: &str) -> Result<(ExprId, ExprId), Error> {
        let Expr::Apply {
            op: Builtin::Tuple,
            args,
        } = &self.program.nodes[id].expr
        else {
            return Err(Error(format!("invalid {what}")));
        };
        if args.len() != 2 {
            return Err(Error(format!("invalid {what} arity")));
        }
        Ok((args[0], args[1]))
    }

    /// Sequence operators applied to a string act on its UTF-16 units, as in TLC.
    pub(crate) fn string_sequence(
        &self,
        op: Builtin,
        values: &[Value],
    ) -> Result<Option<Value>, Error> {
        let Some(Value::String(first)) = values.first() else {
            return Ok(None);
        };
        let units = || first.text.encode_utf16().collect::<Vec<_>>();
        let from_units =
            |units: &[u16]| String::from_utf16(units).map_err(|e| Error(e.to_string()));
        match op {
            Builtin::Concat if values.len() == 2 => {
                let Value::String(second) = &values[1] else {
                    return Err(Error("string concatenation expects strings".into()));
                };
                self.string(format!("{}{}", first.text, second.text))
                    .map(Some)
            }
            Builtin::Tail if values.len() == 1 => {
                let units = units();
                if units.is_empty() {
                    return Err(Error("Tail of empty sequence".into()));
                }
                self.string(from_units(&units[1..])?).map(Some)
            }
            Builtin::Subsequence if values.len() == 3 => {
                let units = units();
                let range = tla_library::subsequence_range(
                    &values[0],
                    &values[1],
                    &values[2],
                    units.len(),
                )?;
                self.string(from_units(&units[range])?).map(Some)
            }
            Builtin::Len if values.len() == 1 => i32::try_from(units().len())
                .map(|n| Some(Value::Int(n)))
                .map_err(|e| Error(e.to_string())),
            _ => Ok(None),
        }
    }

    /// The elements a binder ranges over.
    fn domain(
        &self,
        domain: Option<ExprId>,
        missing: &str,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Vec<Value>, Error> {
        functional::enumerate(
            self,
            domain.ok_or_else(|| Error(missing.into()))?,
            locals,
            depth,
        )
    }

    /// `\E`, `\A`, `{x \in S : P}` and `{e : x \in S}`.
    pub(crate) fn quantify(
        &self,
        kind: Quantifier,
        slot: usize,
        domain: Option<ExprId>,
        body: ExprId,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        let domain = self.domain(
            domain,
            "unbounded quantifier cannot be enumerated",
            locals,
            depth,
        )?;
        // Quantifiers reject tuples of the wrong arity, as TLC does; set
        // builders accept longer tuples and bind their prefix.
        let strict = matches!(kind, Quantifier::Exists | Quantifier::ForAll);
        let mut output = Vec::new();
        for value in &domain {
            deferred::validate_tuple_binding(self, body, slot, value, strict)?;
            let mut nested = locals.clone();
            bind(&mut nested, slot, value.clone());
            let result = self.eval(body, &mut nested, depth + 1)?;
            match kind {
                Quantifier::Exists if result.boolean()? => return Ok(Value::Bool(true)),
                Quantifier::ForAll if !result.boolean()? => return Ok(Value::Bool(false)),
                Quantifier::SetFilter if result.boolean()? => output.push(value.clone()),
                Quantifier::SetMap => output.push(result),
                _ => {}
            }
        }
        match kind {
            Quantifier::Exists => Ok(Value::Bool(false)),
            Quantifier::ForAll => Ok(Value::Bool(true)),
            _ => Ok(Value::set(output)?),
        }
    }

    /// `[x \in S |-> e]`.
    pub(crate) fn function(
        &self,
        slot: usize,
        domain: Option<ExprId>,
        body: ExprId,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        let domain = self.domain(domain, "function value requires a domain", locals, depth)?;
        let mut entries = Vec::new();
        for key in &domain {
            // Every element must be mapped, so a tuple pattern must fit exactly.
            deferred::validate_tuple_binding(self, body, slot, key, true)?;
            let mut nested = locals.clone();
            bind(&mut nested, slot, key.clone());
            entries.push((key.clone(), self.eval(body, &mut nested, depth + 1)?));
        }
        Ok(Value::function(entries)?)
    }

    /// `CHOOSE x \in S : P`, the first element in `S`'s order that satisfies `P`.
    pub(crate) fn choose(
        &self,
        slot: usize,
        domain: Option<ExprId>,
        body: ExprId,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        let domain = self.domain(
            domain,
            "unbounded CHOOSE cannot be evaluated",
            locals,
            depth,
        )?;
        for value in &domain {
            // The lowering narrowed the domain to tuples the pattern
            // destructures; an element that still does not fit is an error.
            deferred::validate_tuple_binding(self, body, slot, value, true)?;
            let mut nested = locals.clone();
            bind(&mut nested, slot, value.clone());
            if self.eval(body, &mut nested, depth + 1)?.boolean()? {
                return Ok(value.clone());
            }
        }
        Err(Error("CHOOSE has no satisfying element".into()))
    }

    /// A single-key `EXCEPT` already resolved by the lowering.
    pub(crate) fn except(
        &self,
        function: ExprId,
        key: ExprId,
        value: ExprId,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Value, Error> {
        let function = self.eval(function, locals, depth + 1)?;
        let key = self.eval(key, locals, depth + 1)?;
        if !matches!(function, Value::Function(_)) {
            return Err(Error("EXCEPT requires a function".into()));
        }
        let replacement = self.eval(value, locals, depth + 1)?;
        Ok(function.except(&[key], replacement)?)
    }
}
