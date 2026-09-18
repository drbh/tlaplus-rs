use tla_bridge::checker::intern_value;
use tla_values::{Error, SetExpr, Value};

pub fn enumeration(text: &str) -> Result<std::collections::HashSet<Value>, String> {
    let mut decoded = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next().ok_or("trailing escape")? {
                '\\' => decoded.push('\\'),
                'n' => decoded.push('\n'),
                c => return Err(format!("unknown escape {c}")),
            }
        } else {
            decoded.push(c);
        }
    }
    let mut reader = Reader {
        text: &decoded,
        cursor: 0,
    };
    reader.require("[")?;
    let mut values = Vec::new();
    if !reader.take("]") {
        loop {
            values.push(reader.value()?);
            if reader.take("]") {
                break;
            }
            reader.require(";")?;
        }
    }
    if !reader.rest().is_empty() {
        return Err(format!("trailing input {}", reader.rest()));
    }
    Ok(values.into_iter().collect())
}
struct Reader<'a> {
    text: &'a str,
    cursor: usize,
}
impl Reader<'_> {
    fn rest(&mut self) -> &str {
        let n = self.text[self.cursor..].len() - self.text[self.cursor..].trim_start().len();
        self.cursor += n;
        &self.text[self.cursor..]
    }
    fn take(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.cursor += token.len();
            true
        } else {
            false
        }
    }
    fn require(&mut self, token: &str) -> Result<(), String> {
        if self.take(token) {
            Ok(())
        } else {
            Err(format!("expected {token} at {}", self.rest()))
        }
    }
    fn word(&mut self) -> Result<String, String> {
        let text = self.rest();
        let n = text
            .find(|c: char| c.is_whitespace() || ",;]}):>".contains(c))
            .unwrap_or(text.len());
        if n == 0 {
            return Err(format!("expected token at {text}"));
        }
        let word = text[..n].to_owned();
        self.cursor += n;
        Ok(word)
    }
    fn value(&mut self) -> Result<Value, String> {
        if self.take("{") {
            let mut values = Vec::new();
            if !self.take("}") {
                loop {
                    values.push(self.value()?);
                    if self.take("}") {
                        break;
                    }
                    self.require(",")?;
                }
            }
            return Value::set(values).map_err(|e| e.to_string());
        }
        if self.take("<<") {
            let mut values = Vec::new();
            if !self.take(">>") {
                loop {
                    values.push(self.value()?);
                    if self.take(">>") {
                        break;
                    }
                    self.require(",")?;
                }
            }
            return Ok(Value::tuple(values));
        }
        if self.take("[") {
            let mut values = Vec::new();
            if !self.take("]") {
                loop {
                    let name = self.word()?;
                    self.require("|->")?;
                    let value = self.value()?;
                    values.push((
                        intern_value(&name, false).map_err(|e| e.to_string())?,
                        value,
                    ));
                    if self.take("]") {
                        break;
                    }
                    self.require(",")?;
                }
            }
            return Value::function(values).map_err(|e| e.to_string());
        }
        if self.take("(") {
            if self.take(")") {
                return Value::function(vec![]).map_err(|e| e.to_string());
            }
            let mut entries = Vec::new();
            loop {
                let key = self.value()?;
                self.require(":>")?;
                let value = self.value()?;
                entries.push((key, value));
                if self.take(")") {
                    break;
                }
                self.require("@@")?;
            }
            return Value::function(entries).map_err(|e| e.to_string());
        }
        if self.rest().starts_with('"') {
            let text = self.rest();
            let mut escaped = false;
            let mut end = None;
            for (i, c) in text.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    end = Some(i + 1);
                    break;
                }
            }
            let n = end.ok_or("unterminated string")?;
            let value: String = serde_json::from_str(&text[..n]).map_err(|e| e.to_string())?;
            self.cursor += n;
            return intern_value(&value, false).map_err(|e| e.to_string());
        }
        let token = self.word()?;
        match token.as_str() {
            "TRUE" => Ok(Value::Bool(true)),
            "FALSE" => Ok(Value::Bool(false)),
            _ => {
                if let Ok(n) = token.parse::<i32>() {
                    return Ok(Value::Int(n));
                }
                if let Some((lo, hi)) = token.split_once("..") {
                    return SetExpr::Interval(
                        lo.parse().map_err(|_| "interval low")?,
                        hi.parse().map_err(|_| "interval high")?,
                    )
                    .materialize(100_000)
                    .map_err(|e| e.to_string());
                }
                intern_value(&token, true).map_err(|e| e.to_string())
            }
        }
    }
}

pub fn observed(
    expr: &tla_value_expr::Expr,
    normalized: bool,
) -> Result<std::collections::HashSet<Value>, Error> {
    (if normalized {
        expr.sorted(100_000)?
    } else {
        expr.enumerate(100_000)?
    })
    .iter()
    .map(|v| v.normalize(100_000))
    .collect()
}
