use crate::{Definition, Diagnostic, Expr, Module, Token, lex, lexer::error};

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &str {
        self.tokens.get(self.at).map_or("", |t| t.text.as_str())
    }
    fn take(&mut self) -> Result<String, Diagnostic> {
        let token = self
            .tokens
            .get(self.at)
            .ok_or_else(|| self.error("expected expression"))?;
        self.at += 1;
        Ok(token.text.clone())
    }
    fn error(&self, message: impl Into<String>) -> Diagnostic {
        error(
            self.tokens.get(self.at).map_or_else(
                || self.tokens.last().map_or(0, |t| t.offset + t.text.len()),
                |t| t.offset,
            ),
            message,
        )
    }
    fn expect(&mut self, expected: &str) -> Result<(), Diagnostic> {
        if self.peek() != expected {
            return Err(self.error(format!("expected {expected}, found {}", self.peek())));
        }
        self.at += 1;
        Ok(())
    }
    fn list(&mut self, close: &str) -> Result<Vec<Expr>, Diagnostic> {
        let mut values = Vec::new();
        if self.peek() != close {
            loop {
                values.push(self.expr(0)?);
                if self.peek() != "," {
                    break;
                }
                self.at += 1;
            }
        }
        self.expect(close)?;
        Ok(values)
    }
    fn expr(&mut self, minimum: u8) -> Result<Expr, Diagnostic> {
        let token = self.take()?;
        let mut left = match token.as_str() {
            "(" => {
                let value = self.expr(0)?;
                self.expect(")")?;
                value
            }
            "{" => self.set()?,
            "<<" => Expr::Tuple(self.list(">>")?),
            "IF" => {
                let condition = self.expr(0)?;
                self.expect("THEN")?;
                let yes = self.expr(0)?;
                self.expect("ELSE")?;
                Expr::If(Box::new(condition), Box::new(yes), Box::new(self.expr(0)?))
            }
            "\\E" | "\\A" | "CHOOSE" => {
                let name = self.take()?;
                self.expect("\\in")?;
                let domain = self.expr(0)?;
                self.expect(":")?;
                Expr::Bound {
                    kind: token,
                    names: vec![name],
                    tuple: false,
                    domain: Some(Box::new(domain)),
                    body: Box::new(self.expr(0)?),
                }
            }
            "[" => {
                if self
                    .tokens
                    .get(self.at + 1)
                    .is_some_and(|t| t.text == "|->")
                {
                    let mut fields = Vec::new();
                    loop {
                        let field = self.take()?;
                        if !identifier(&field) || fields.iter().any(|(name, _)| name == &field) {
                            return Err(self.error("invalid record field"));
                        }
                        self.expect("|->")?;
                        fields.push((field, self.expr(0)?));
                        if self.peek() != "," {
                            break;
                        }
                        self.at += 1;
                    }
                    self.expect("]")?;
                    Expr::Record(fields)
                } else if self
                    .tokens
                    .get(self.at + 1)
                    .is_some_and(|t| t.text == "\\in")
                {
                    let name = self.take()?;
                    self.expect("\\in")?;
                    let domain = self.expr(0)?;
                    self.expect("|->")?;
                    let body = self.expr(0)?;
                    self.expect("]")?;
                    Expr::Bound {
                        kind: "function".into(),
                        names: vec![name],
                        tuple: false,
                        domain: Some(Box::new(domain)),
                        body: Box::new(body),
                    }
                } else {
                    let action = self.expr(0)?;
                    if self.peek() == "->" {
                        self.at += 1;
                        let range = self.expr(0)?;
                        self.expect("]")?;
                        return Ok(Expr::Binary("->".into(), Box::new(action), Box::new(range)));
                    }
                    self.expect("]")?;
                    let subscript = self.take()?;
                    let variables = if subscript == "_" {
                        self.expr(12)?
                    } else if let Some(name) = subscript.strip_prefix('_') {
                        Expr::Name(name.into())
                    } else {
                        return Err(self.error("action brackets require a subscript"));
                    };
                    Expr::Stutter(Box::new(action), Box::new(variables))
                }
            }
            "~" | "[]" | "<>" => Expr::Unary(token, Box::new(self.expr(5)?)),
            "-" | "SUBSET" | "UNION" | "DOMAIN" | "UNCHANGED" => {
                Expr::Unary(token, Box::new(self.expr(12)?))
            }
            "/\\" | "\\/" => self.expr(if token == "/\\" { 5 } else { 4 })?,
            _ if token.starts_with('"') => {
                Expr::String(string(&token).map_err(|message| self.error(message))?)
            }
            _ if token.bytes().all(|b| b.is_ascii_digit()) => Expr::Integer(
                token
                    .parse()
                    .map_err(|_| self.error("integer out of range"))?,
            ),
            _ if identifier(&token) => {
                if self.peek() == "(" {
                    self.at += 1;
                    Expr::Call(token, self.list(")")?)
                } else {
                    Expr::Name(token)
                }
            }
            _ => return Err(self.error(format!("unexpected expression token {token}"))),
        };
        loop {
            if self.peek() == "." {
                self.at += 1;
                let field = self.take()?;
                if !identifier(&field) {
                    return Err(self.error("expected record field"));
                }
                left = Expr::Index(Box::new(left), Box::new(Expr::String(field)));
                continue;
            }
            if self.peek() == "'" {
                self.at += 1;
                left = Expr::Unary("'".into(), Box::new(left));
                continue;
            }
            if self.peek() == "[" {
                self.at += 1;
                let index = self.expr(0)?;
                self.expect("]")?;
                left = Expr::Index(Box::new(left), Box::new(index));
                continue;
            }
            let Some((precedence, right_associative)) = precedence(self.peek()) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            let operator = self.take()?;
            let right = self.expr(precedence + u8::from(!right_associative))?;
            left = Expr::Binary(operator, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn set(&mut self) -> Result<Expr, Diagnostic> {
        if self.peek() == "}" {
            self.at += 1;
            return Ok(Expr::Set(Vec::new()));
        }
        if self
            .tokens
            .get(self.at + 1)
            .is_some_and(|t| t.text == "\\in")
        {
            let name = self.take()?;
            self.expect("\\in")?;
            let domain = self.expr(0)?;
            self.expect(":")?;
            let body = self.expr(0)?;
            self.expect("}")?;
            return Ok(Expr::Bound {
                kind: "filter".into(),
                names: vec![name],
                tuple: false,
                domain: Some(Box::new(domain)),
                body: Box::new(body),
            });
        }
        let first = self.expr(0)?;
        if self.peek() == ":" {
            self.at += 1;
            let name = self.take()?;
            self.expect("\\in")?;
            let domain = self.expr(0)?;
            self.expect("}")?;
            return Ok(Expr::Bound {
                kind: "map".into(),
                names: vec![name],
                tuple: false,
                domain: Some(Box::new(domain)),
                body: Box::new(first),
            });
        }
        let mut values = vec![first];
        while self.peek() == "," {
            self.at += 1;
            values.push(self.expr(0)?);
        }
        self.expect("}")?;
        Ok(Expr::Set(values))
    }
}

fn identifier(text: &str) -> bool {
    text.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn precedence(operator: &str) -> Option<(u8, bool)> {
    Some((
        match operator {
            "<=>" => 1,
            "=>" | "~>" => 2,
            "\\/" => 3,
            "/\\" => 4,
            "=" | "#" | "/=" | "<" | "<=" | ">" | ">=" | "\\in" | "\\notin" | "\\subseteq" => 5,
            "\\cup" | "\\union" | "\\cap" | "\\intersect" | "\\" => 6,
            ".." => 7,
            "+" | "-" => 8,
            "*" | "/" | "\\div" | "%" | "\\o" => 9,
            "^" => 10,
            _ => return None,
        },
        matches!(operator, "=>" | "^"),
    ))
}

fn string(token: &str) -> Result<String, &'static str> {
    let mut result = String::new();
    let mut chars = token[1..token.len() - 1].chars();
    while let Some(ch) = chars.next() {
        result.push(if ch == '\\' {
            match chars.next() {
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('f') => '\x0c',
                Some('"') => '"',
                Some('\\') => '\\',
                _ => return Err("invalid string escape"),
            }
        } else {
            ch
        });
    }
    Ok(result)
}

pub fn expression(source: &str) -> Result<Expr, Diagnostic> {
    parse_expression(&lex(source)?)
}

fn parse_expression(tokens: &[Token]) -> Result<Expr, Diagnostic> {
    let mut parser = Parser { tokens, at: 0 };
    let result = parser.expr(0)?;
    if parser.at != tokens.len() {
        return Err(parser.error("unexpected token after expression"));
    }
    Ok(result)
}

pub fn module(source: &str) -> Result<Module, Diagnostic> {
    let tokens = crate::lexer::module_tokens(source)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
    };
    if !parser.peek().starts_with("----") {
        return Err(parser.error("expected module header"));
    }
    parser.at += 1;
    parser.expect("MODULE")?;
    let name = parser.take()?;
    if !identifier(&name) {
        return Err(parser.error("invalid module name"));
    }
    if !parser.peek().starts_with("----") {
        return Err(parser.error("expected module header terminator"));
    }
    parser.at += 1;
    let mut result = Module {
        name,
        extends: Vec::new(),
        variables: Vec::new(),
        constants: Vec::new(),
        definitions: Vec::new(),
        assumptions: Vec::new(),
        instances: Vec::new(),
        local_names: Vec::new(),
        modules: Vec::new(),
        declared_at: Vec::new(),
    };
    while !parser.peek().starts_with("====") {
        let keyword = parser.take()?;
        if matches!(
            keyword.as_str(),
            "EXTENDS" | "VARIABLE" | "VARIABLES" | "CONSTANT" | "CONSTANTS"
        ) {
            let destination = match keyword.as_str() {
                "EXTENDS" => &mut result.extends,
                "VARIABLE" | "VARIABLES" => &mut result.variables,
                _ => &mut result.constants,
            };
            loop {
                let name = parser.take()?;
                if !identifier(&name) {
                    return Err(parser.error("expected identifier"));
                }
                if destination.contains(&name) {
                    return Err(parser.error(format!("duplicate declaration {name}")));
                }
                destination.push(name);
                if parser.peek() != "," {
                    break;
                }
                parser.at += 1;
            }
            continue;
        }
        let assumption = keyword == "ASSUME" || keyword == "ASSUMPTION";
        let mut parameters = Vec::new();
        if !assumption {
            if !identifier(&keyword) {
                return Err(parser.error("expected operator definition"));
            }
            if parser.peek() == "(" {
                parser.at += 1;
                loop {
                    let parameter = parser.take()?;
                    if !identifier(&parameter) || parameters.contains(&parameter) {
                        return Err(parser.error("invalid operator parameter"));
                    }
                    parameters.push(parameter);
                    if parser.peek() != "," {
                        break;
                    }
                    parser.at += 1;
                }
                parser.expect(")")?;
            }
            parser.expect("==")?;
        }
        let start = parser.at;
        while parser.at < tokens.len() && !boundary(&tokens, parser.at, start) {
            parser.at += 1;
        }
        let body = parse_expression(&tokens[start..parser.at])?;
        if assumption {
            result.assumptions.push(body);
        } else {
            if result.definitions.iter().any(|d| d.name == keyword) {
                return Err(parser.error(format!("duplicate operator {keyword}")));
            }
            result.definitions.push(Definition {
                name: keyword,
                arities: vec![0; parameters.len()],
                parameters,
                body,
                recursive: false,
                standard: false,
            });
        }
    }
    parser.at += 1;
    if parser.at != tokens.len() {
        return Err(parser.error("unexpected text after module"));
    }
    Ok(result)
}

fn boundary(tokens: &[Token], at: usize, start: usize) -> bool {
    let token = &tokens[at];
    if token.text.starts_with("====") {
        return true;
    }
    if at == start || token.line == tokens[at - 1].line {
        return false;
    }
    if matches!(
        token.text.as_str(),
        "ASSUME" | "ASSUMPTION" | "EXTENDS" | "VARIABLE" | "VARIABLES" | "CONSTANT" | "CONSTANTS"
    ) {
        return true;
    }
    if !identifier(&token.text) {
        return false;
    }
    if tokens.get(at + 1).is_some_and(|t| t.text == "==") {
        return true;
    }
    if tokens.get(at + 1).is_some_and(|t| t.text == "(") {
        return tokens[at + 2..]
            .iter()
            .take_while(|t| t.line == token.line)
            .any(|t| t.text == "==");
    }
    false
}
