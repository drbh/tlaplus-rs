//! Small ASCII syntax with Unicode-aware diagnostic columns and line comments.
use crate::{Error, Location};

#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub at: Location,
}

pub fn lex(source: &str) -> Result<Vec<Token>, Error> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    let mut at = Location { line: 1, column: 1 };
    while let Some(c) = chars.next() {
        let start = at;
        at.column += 1;
        if c == '\n' {
            at.line += 1;
            at.column = 1;
        }
        if c.is_whitespace() {
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            for next in chars.by_ref() {
                if next == '\n' {
                    at.line += 1;
                    at.column = 1;
                    break;
                }
                at.column += 1;
            }
            continue;
        }
        let mut text = c.to_string();
        if c == '"' {
            let mut closed = false;
            while let Some(next) = chars.next() {
                at.column += 1;
                if next == '\n' || next == '\r' {
                    break;
                }
                text.push(next);
                if next == '"' {
                    closed = true;
                    break;
                }
                if next == '\\' {
                    let escaped = chars.next().ok_or_else(|| Error {
                        location: start,
                        message: "unterminated string escape".into(),
                    })?;
                    at.column += 1;
                    if !matches!(escaped, '"' | '\\' | 'n' | 't' | 'r') {
                        return Err(Error {
                            location: start,
                            message: "unsupported string escape".into(),
                        });
                    }
                    text.push(escaped);
                }
            }
            if !closed {
                return Err(Error {
                    location: start,
                    message: "unterminated string".into(),
                });
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                text.push(chars.next().unwrap());
                at.column += 1;
            }
        } else if c.is_ascii_digit() {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                text.push(chars.next().unwrap());
                at.column += 1;
            }
        } else if let Some(&next) = chars.peek()
            && matches!(
                (c, next),
                ('=', '=')
                    | ('!', '=')
                    | ('<', '=')
                    | ('>', '=')
                    | ('&', '&')
                    | ('|', '|')
                    | ('.', '.')
                    | ('=', '>')
                    | ('-', '>')
            )
        {
            text.push(chars.next().unwrap());
            at.column += 1;
            if text == ".." && chars.peek() == Some(&'=') {
                text.push(chars.next().unwrap());
                at.column += 1;
            }
        } else if !"{}[]():;,=+-*!<>%.".contains(c) {
            return Err(Error {
                location: start,
                message: format!("unexpected character {c:?}"),
            });
        }
        tokens.push(Token { text, at: start });
    }
    tokens.push(Token {
        text: String::new(),
        at,
    });
    Ok(tokens)
}
