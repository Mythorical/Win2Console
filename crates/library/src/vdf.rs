//! Minimal parser for Valve's KeyValues text format (`.vdf` / `.acf`).
//!
//! Supports quoted and unquoted tokens, `\\ \" \n \t` escapes, `//` comments, nested objects and
//! platform conditionals (`[$WIN32]`), which are skipped. Keys are matched case-insensitively,
//! as Steam does.

/// A parsed KeyValues node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Str(String),
    Obj(Vec<(String, Node)>),
}

impl Node {
    /// First child named `key` (case-insensitive).
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Obj(children) => children.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Node::Str(_) => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Node::Str(s) => Some(s),
            Node::Obj(_) => None,
        }
    }

    /// Child entries of an object (empty for strings).
    pub fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Obj(children) => children,
            Node::Str(_) => &[],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VDF parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parses a document and returns its top-level `(key, node)` pair (e.g. `("AppState", {...})`).
pub fn parse(text: &str) -> Result<(String, Node), ParseError> {
    let mut lexer = Lexer { chars: text.trim_start_matches('\u{feff}').chars().peekable() };
    let key = match lexer.next()? {
        Some(Tok::Str(k)) => k,
        other => return Err(ParseError(format!("expected root key, got {other:?}"))),
    };
    let value = parse_value(&mut lexer)?;
    Ok((key, value))
}

fn parse_value(lx: &mut Lexer<'_>) -> Result<Node, ParseError> {
    match lx.next()? {
        Some(Tok::Str(s)) => Ok(Node::Str(s)),
        Some(Tok::Open) => parse_object(lx),
        other => Err(ParseError(format!("expected value, got {other:?}"))),
    }
}

fn parse_object(lx: &mut Lexer<'_>) -> Result<Node, ParseError> {
    let mut children = Vec::new();
    loop {
        match lx.next()? {
            Some(Tok::Close) => return Ok(Node::Obj(children)),
            Some(Tok::Str(key)) => children.push((key, parse_value(lx)?)),
            other => return Err(ParseError(format!("expected key or '}}', got {other:?}"))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Lexer<'_> {
    fn next(&mut self) -> Result<Option<Tok>, ParseError> {
        loop {
            let Some(&c) = self.chars.peek() else { return Ok(None) };
            match c {
                c if c.is_whitespace() => {
                    self.chars.next();
                }
                '/' => {
                    self.chars.next();
                    if self.chars.peek() == Some(&'/') {
                        self.chars.by_ref().take_while(|&c| c != '\n').for_each(drop);
                    } else {
                        return Err(ParseError("stray '/'".into()));
                    }
                }
                '[' => {
                    // Conditional like [$WIN32]: ignore it.
                    self.chars.by_ref().take_while(|&c| c != ']').for_each(drop);
                }
                '{' => {
                    self.chars.next();
                    return Ok(Some(Tok::Open));
                }
                '}' => {
                    self.chars.next();
                    return Ok(Some(Tok::Close));
                }
                '"' => {
                    self.chars.next();
                    return self.quoted().map(|s| Some(Tok::Str(s)));
                }
                _ => {
                    let mut s = String::new();
                    while let Some(&c) = self.chars.peek() {
                        if c.is_whitespace() || matches!(c, '{' | '}' | '"') {
                            break;
                        }
                        s.push(c);
                        self.chars.next();
                    }
                    return Ok(Some(Tok::Str(s)));
                }
            }
        }
    }

    fn quoted(&mut self) -> Result<String, ParseError> {
        let mut s = String::new();
        loop {
            match self.chars.next() {
                None => return Err(ParseError("unterminated string".into())),
                Some('"') => return Ok(s),
                Some('\\') => match self.chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(c @ ('\\' | '"')) => s.push(c),
                    // Unknown escape: keep it literally (Windows paths in old files).
                    Some(c) => {
                        s.push('\\');
                        s.push(c);
                    }
                    None => return Err(ParseError("unterminated string".into())),
                },
                Some(c) => s.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_with_comments_and_escapes() {
        let text = r#"
            // comment
            "Root"
            {
                "Path"   "C:\\Program Files (x86)\\Steam"
                "Quote"  "say \"hi\""
                unquoted value
                "Child" [$WIN32]
                {
                    "a" "1"
                }
            }
        "#;
        let (key, root) = parse(text).unwrap();
        assert_eq!(key, "Root");
        assert_eq!(root.get_str("path"), Some(r"C:\Program Files (x86)\Steam"));
        assert_eq!(root.get_str("Quote"), Some(r#"say "hi""#));
        assert_eq!(root.get_str("unquoted"), Some("value"));
        assert_eq!(root.get("child").unwrap().get_str("A"), Some("1"));
        assert_eq!(root.entries().len(), 4);
    }

    #[test]
    fn handles_bom() {
        let (key, _) = parse("\u{feff}\"k\" { }").unwrap();
        assert_eq!(key, "k");
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse(r#""Root" { "a" "#).is_err());
        assert!(parse(r#""Root" { "a" "1" "#).is_err());
        assert!(parse("").is_err());
    }
}
