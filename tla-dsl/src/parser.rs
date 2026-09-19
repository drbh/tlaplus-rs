//! Declarations and bounded recursive-descent parsing of the modern syntax.
mod expressions;
use crate::{
    Error,
    ast::*,
    lexer::{self, Token},
};

pub fn parse(source: &str) -> Result<Spec, Error> {
    Parser {
        tokens: lexer::lex(source)?,
        index: 0,
        depth: 0,
        nodes: 0,
    }
    .spec()
}
struct Parser {
    tokens: Vec<Token>,
    index: usize,
    depth: usize,
    nodes: usize,
}
impl Parser {
    fn token(&self) -> &Token {
        &self.tokens[self.index]
    }
    fn error(&self, message: impl Into<String>) -> Error {
        Error {
            location: self.token().at,
            message: message.into(),
        }
    }
    fn eat(&mut self, text: &str) -> bool {
        if !text.is_empty() && self.token().text == text {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, text: &str) -> Result<(), Error> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error(format!("expected {text:?}, found {:?}", self.token().text)))
        }
    }
    fn name(&mut self) -> Result<Name, Error> {
        let token = self.token();
        if !token
            .text
            .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        {
            return Err(self.error("expected an identifier"));
        }
        let name = Name {
            text: token.text.clone(),
            at: token.at,
        };
        self.index += 1;
        Ok(name)
    }
    fn spec(&mut self) -> Result<Spec, Error> {
        let library = self.eat("module");
        if !library {
            self.expect("spec")?;
        }
        let name = self.name()?;
        self.expect("{")?;
        let mut spec = Spec {
            name,
            library,
            imports: vec![],
            enums: vec![],
            records: vec![],
            refinements: vec![],
            domains: vec![],
            definition_order: vec![],
            init: None,
            next: None,
            refinement_checks: vec![],
            constants: vec![],
            states: vec![],
            functions: vec![],
            assumptions: vec![],
            actions: vec![],
            properties: vec![],
            fairness: vec![],
        };
        while !self.eat("}") {
            self.declaration(&mut spec)?;
        }
        if !self.token().text.is_empty() {
            return Err(self.error("unexpected text after specification"));
        }
        Ok(spec)
    }
    fn declaration(&mut self, spec: &mut Spec) -> Result<(), Error> {
        match self.token().text.as_str() {
            "import" => { self.index += 1; spec.imports.push(self.name()?); self.expect(";")?; }
            "enum" => {
                self.index += 1; let name = self.name()?; self.expect("{")?;
                let mut variants = vec![];
                while !self.eat("}") {
                    variants.push(self.name()?);
                    if !self.eat(",") { self.expect("}")?; break; }
                }
                spec.enums.push(Enumeration { name, variants });
            }
            "struct" => {
                self.index += 1; let name = self.name()?; self.expect("{")?;
                let mut fields = vec![];
                while !self.eat("}") {
                    let field = self.name()?; self.expect(":")?; fields.push((field, self.ty(0)?));
                    if !self.eat(",") { self.expect("}")?; break; }
                }
                spec.records.push((name, fields));
            }
            "type" => {
                self.index += 1; let name = self.name()?; self.expect("=")?;
                let base = self.ty(0)?; self.expect("where")?; let binding = self.name()?;
                let predicate = self.block()?;
                spec.refinements.push(Refinement { name, base, binding, predicate });
            }
            "domain" => {
                self.index += 1; let name = self.name()?; self.expect("in")?;
                let value = self.expr(0)?; self.expect(";")?; spec.domains.push((name, value));
            }
            "init" => {
                if spec.init.is_some() { return Err(self.error("duplicate init block")); }
                let name = self.name()?; spec.init = Some(self.action_body(name, vec![])?);
            }
            "next" => {
                if spec.next.is_some() { return Err(self.error("duplicate next block")); }
                self.index += 1; self.expect("{")?; let mut actions = vec![];
                loop { actions.push(self.name()?); if !self.eat(",") { break; } }
                self.expect("}")?; spec.next = Some(actions);
            }
            "state" | "const" => {
                let state = self.eat("state"); if !state { self.expect("const")?; }
                let name = self.name()?; self.expect(":")?; let ty = self.ty(0)?;
                let initialized = !(state && self.token().text == ";");
                let choice = state && self.eat("in");
                let value = if initialized {
                    if !choice { self.expect("=")?; } self.expr(0)?
                } else { Expr { at: name.at, kind: Kind::Bool(false) } };
                self.expect(";")?;
                let binding = Binding { name, ty, value, choice, initialized };
                if state { spec.states.push(binding); } else { spec.constants.push(binding); }
            }
            "fn" | "recursive" | "view" => spec.functions.push(self.function()?),
            "assume" => { self.index += 1; spec.assumptions.push(self.expr(0)?); self.expect(";")?; }
            "action" => spec.actions.push(self.action()?),
            "invariant" | "property" | "transition" => {
                let transition = self.eat("transition"); let temporal = self.eat("property");
                if !transition && !temporal { self.expect("invariant")?; }
                let name = self.name()?; let mut frame = vec![];
                if transition {
                    self.expect("over")?; self.expect("(")?;
                    loop { frame.push(self.name()?); if !self.eat(",") { break; } }
                    self.expect(")")?;
                }
                let body = self.block()?;
                spec.properties.push(Property { name, body, temporal, frame });
            }
            "fair" => {
                self.index += 1; let strong = self.eat("strong"); if !strong { self.expect("weak")?; }
                let mut actions = vec![];
                if self.eat("{") {
                    loop { actions.push(self.name()?); if !self.eat(",") { break; } }
                    self.expect("}")?;
                } else { actions.push(self.name()?); }
                self.expect(";")?; spec.fairness.push(Fairness { strong, actions });
            }
            _ => return Err(self.error("expected type, enum, const, state, fn, import, assume, action, invariant, property, transition, or fair")),
        }
        Ok(())
    }
    fn ty(&mut self, depth: usize) -> Result<Type, Error> {
        if depth > 32 {
            return Err(self.error("type nesting exceeds 32 levels"));
        }
        let name = self.name()?;
        Ok(match name.text.as_str() {
            "Int" => Type::Int,
            "Bool" => Type::Bool,
            "String" => Type::String,
            "Set" | "Seq" | "Map" | "Fn" => {
                self.expect("<")?;
                let first = Box::new(self.ty(depth + 1)?);
                let ty = match name.text.as_str() {
                    "Set" => Type::Set(first),
                    "Seq" => Type::Seq(first),
                    _ => {
                        self.expect(",")?;
                        let second = Box::new(self.ty(depth + 1)?);
                        if name.text == "Map" {
                            Type::Map(first, second)
                        } else {
                            Type::Function(vec![*first], second)
                        }
                    }
                };
                self.expect(">")?;
                ty
            }
            _ => Type::Enum(name.text),
        })
    }
    fn function(&mut self) -> Result<Function, Error> {
        let recursive = self.eat("recursive");
        let stateful = self.eat("view");
        if !stateful {
            self.expect("fn")?;
        }
        let name = self.name()?;
        self.expect("(")?;
        let mut params = vec![];
        if !self.eat(")") {
            loop {
                let name = self.name()?;
                self.expect(":")?;
                params.push((name, self.ty(0)?));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        self.expect("->")?;
        let result = self.ty(0)?;
        let body = self.block()?;
        Ok(Function {
            name,
            params,
            result,
            body,
            recursive,
            stateful,
        })
    }
    fn domain_binding(&mut self) -> Result<(Name, Expr), Error> {
        let name = self.name()?;
        self.expect("in")?;
        Ok((name, self.expr(0)?))
    }
    fn action(&mut self) -> Result<Action, Error> {
        self.expect("action")?;
        let name = self.name()?;
        let mut params = vec![];
        if self.eat("(") {
            loop {
                params.push(self.domain_binding()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        self.action_body(name, params)
    }
    fn action_body(&mut self, name: Name, params: Vec<(Name, Expr)>) -> Result<Action, Error> {
        self.expect("{")?;
        let mut action = Action {
            name,
            params,
            picks: vec![],
            guards: vec![],
            updates: vec![],
            calls: vec![],
        };
        while !self.eat("}") {
            if self.eat("pick") {
                if !action.guards.is_empty()
                    || !action.updates.is_empty()
                    || !action.calls.is_empty()
                {
                    return Err(self.error("pick declarations must precede guards and updates"));
                }
                action.picks.push(self.domain_binding()?);
            } else if self.eat("call") {
                let callee = self.name()?;
                self.expect("(")?;
                action.calls.push((callee, self.items(")")?));
            } else if self.eat("require") {
                action.guards.push(self.expr(0)?);
            } else if self.eat("next") {
                let name = self.name()?;
                self.expect("=")?;
                action.updates.push((name, self.expr(0)?));
            } else {
                return Err(self.error("expected pick, require, or next in action"));
            }
            self.expect(";")?;
        }
        Ok(action)
    }
    fn block(&mut self) -> Result<Expr, Error> {
        self.expect("{")?;
        let body = self.expr(0)?;
        self.expect("}")?;
        Ok(body)
    }
}
