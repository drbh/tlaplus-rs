//! Expressions, comprehensions, and calls. Collection indexing is one-based.
use super::*;
impl Parser {
    pub(super) fn expr(&mut self, minimum: u8) -> Result<Expr, Error> {
        if self.depth == 0 {
            self.nodes = 0;
        }
        self.node()?;
        if self.depth >= 128 {
            return Err(self.error("expression nesting exceeds 128 levels"));
        }
        self.depth += 1;
        let mut left = self.atom()?;
        loop {
            if self.eat(".") {
                self.node()?;
                let field = self.name()?;
                left = Expr {
                    at: left.at,
                    kind: Kind::Field(Box::new(left), field),
                };
                continue;
            }
            if self.eat("[") {
                let key = self.expr(0)?;
                self.expect("]")?;
                left = Expr {
                    at: left.at,
                    kind: Kind::Index(Box::new(left), Box::new(key)),
                };
                continue;
            }
            let Some(precedence) = precedence(&self.token().text) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            self.node()?;
            let op = self.token().text.clone();
            self.index += 1;
            let right = self.expr(precedence + u8::from(op != "=>"))?;
            left = Expr {
                at: left.at,
                kind: Kind::Binary(op, Box::new(left), Box::new(right)),
            };
        }
        self.depth -= 1;
        Ok(left)
    }
    fn node(&mut self) -> Result<(), Error> {
        self.nodes += 1;
        if self.nodes > 256 {
            return Err(self.error("expression exceeds the prototype limit of 256 syntax nodes"));
        }
        Ok(())
    }
    pub(super) fn items(&mut self, end: &str) -> Result<Vec<Expr>, Error> {
        let mut items = vec![];
        if !self.eat(end) {
            loop {
                items.push(self.expr(0)?);
                if !self.eat(",") {
                    self.expect(end)?;
                    break;
                }
                if self.eat(end) {
                    break;
                }
            }
        }
        Ok(items)
    }
    fn atom(&mut self) -> Result<Expr, Error> {
        let token = self.token().clone();
        let kind = match token.text.as_str() {
            "(" => {
                self.index += 1;
                let inner = self.expr(0)?;
                if self.eat(",") {
                    let mut items = vec![inner];
                    items.extend(self.items(")")?);
                    Kind::Tuple(items)
                } else {
                    self.expect(")")?;
                    return Ok(inner);
                }
            }
            "[" | "{" => {
                self.index += 1;
                let set = token.text == "{";
                Kind::Collection(set, self.items(if set { "}" } else { "]" })?)
            }
            "!" | "-" => {
                self.index += 1;
                Kind::Unary(token.text, Box::new(self.expr(8)?))
            }
            "always" | "eventually" | "after" => {
                self.index += 1;
                self.expect("(")?;
                let body = self.expr(0)?;
                self.expect(")")?;
                Kind::Unary(token.text, Box::new(body))
            }
            "if" => {
                self.index += 1;
                let condition = self.expr(0)?;
                let yes = self.block()?;
                self.expect("else")?;
                let no = self.block()?;
                Kind::If(Box::new(condition), Box::new(yes), Box::new(no))
            }
            "forall" | "exists" | "map" | "sequence" | "set_of" | "filter" | "choose" => {
                self.index += 1;
                self.expect("(")?;
                let (name, domain) = self.domain_binding()?;
                self.expect(")")?;
                let body = self.block()?;
                Kind::Binder(token.text, name, Box::new(domain), Box::new(body))
            }
            "lambda" => {
                self.index += 1;
                self.expect("(")?;
                let name = self.name()?;
                self.expect(":")?;
                let ty = self.ty(0)?;
                self.expect(")")?;
                Kind::Lambda(name, ty, Box::new(self.block()?))
            }
            "true" | "false" => {
                self.index += 1;
                Kind::Bool(token.text == "true")
            }
            text if text.starts_with('"') => {
                self.index += 1;
                Kind::String(token.text)
            }
            text if text.starts_with(|c: char| c.is_ascii_digit()) => {
                self.index += 1;
                Kind::Int(token.text)
            }
            _ => self.named_atom()?,
        };
        Ok(Expr { at: token.at, kind })
    }
    fn named_atom(&mut self) -> Result<Kind, Error> {
        Ok({
            let name = self.name()?;
            if self.token().text == "{"
                && self
                    .tokens
                    .get(self.index + 2)
                    .is_some_and(|t| t.text == ":")
            {
                self.index += 1;
                let mut fields = vec![];
                loop {
                    let field = self.name()?;
                    self.expect(":")?;
                    fields.push((field, self.expr(0)?));
                    if !self.eat(",") || self.token().text == "}" {
                        break;
                    }
                }
                self.expect("}")?;
                Kind::Record(name.text, fields)
            } else if self.eat("(") {
                Kind::Call(name.text, self.items(")")?)
            } else {
                Kind::Name(name.text)
            }
        })
    }
}
fn precedence(op: &str) -> Option<u8> {
    Some(match op {
        "=>" => 1,
        "||" => 2,
        "&&" => 3,
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "is" => 4,
        "..=" => 5,
        "+" | "-" => 6,
        "*" | "%" => 7,
        _ => return None,
    })
}
