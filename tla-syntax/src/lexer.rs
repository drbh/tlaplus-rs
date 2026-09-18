use crate::Diagnostic;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut tokens = Vec::new();
    let (mut at, mut line, mut column) = (0, 1, 1);
    let bytes = source.as_bytes();
    while at < bytes.len() {
        let start = at;
        let (start_line, start_column) = (line, column);
        let tail = &source[at..];
        if tail.starts_with("\\*") {
            at += tail.find('\n').unwrap_or(tail.len());
        } else if tail.starts_with("(*") {
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth > 0 {
                if source[at..].starts_with("(*") {
                    depth += 1;
                    at += 2;
                } else if source[at..].starts_with("*)") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += source[at..].chars().next().unwrap().len_utf8();
                }
            }
            if depth > 0 {
                return Err(error(start, "unterminated comment"));
            }
        } else if bytes[at].is_ascii_whitespace() {
            at += 1;
        } else {
            if bytes[at] == b'"' {
                at += 1;
                let mut closed = false;
                while at < bytes.len() {
                    match bytes[at] {
                        b'"' => {
                            at += 1;
                            closed = true;
                            break;
                        }
                        b'\\' => {
                            at += 1;
                            if at < bytes.len() {
                                at += 1;
                            }
                        }
                        b'\n' | b'\r' => return Err(error(at, "newline in string")),
                        _ => at += source[at..].chars().next().unwrap().len_utf8(),
                    }
                }
                if !closed {
                    return Err(error(start, "unterminated string"));
                }
            } else if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' {
                at += 1;
                while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                    at += 1;
                }
            } else if tail.starts_with("----") || tail.starts_with("====") {
                let byte = bytes[at];
                while at < bytes.len() && bytes[at] == byte {
                    at += 1;
                }
            } else if let Some(symbol) = [
                "<=>",
                "|->",
                "\\notin",
                "\\subseteq",
                "\\union",
                "\\intersect",
                "\\cup",
                "\\cap",
                "\\div",
                "\\o",
                "\\in",
                "\\A",
                "\\E",
                "/\\",
                "\\/",
                "==",
                "=>",
                "<=",
                ">=",
                "/=",
                "#",
                "..",
                "<<",
                ">>",
                "[]",
                "<>",
                "->",
                "~>",
            ]
            .iter()
            .find(|symbol| tail.starts_with(**symbol))
            {
                at += symbol.len();
            } else if "()[]{},:!'+-*/%^=<>~\\.@".contains(bytes[at] as char) {
                at += 1;
            } else {
                return Err(error(at, "unexpected character"));
            }
            tokens.push(Token {
                text: source[start..at].to_owned(),
                offset: start,
                line: start_line,
                column: start_column,
            });
        }
        for ch in source[start..at].chars() {
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
    }
    Ok(tokens)
}

pub(crate) fn error(offset: usize, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        offset,
        message: message.into(),
    }
}

pub(crate) fn module_tokens(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let mut offset = 0;
    let mut start = None;
    let mut end = source.len();
    let mut depth = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("----") && trimmed.contains("MODULE") {
            start.get_or_insert(offset);
            depth += 1;
        } else if start.is_some() && trimmed.starts_with("====") {
            depth -= 1;
            if depth == 0 {
                end = offset + line.len();
                break;
            }
        }
        offset += line.len();
    }
    let start = start.ok_or_else(|| error(0, "expected module header"))?;
    let mut tokens = lex(&source[start..end])?;
    let line_offset = source[..start].bytes().filter(|b| *b == b'\n').count();
    for token in &mut tokens {
        token.offset += start;
        token.line += line_offset;
    }
    Ok(tokens)
}
