//! Lexer. Turns source text into tokens.
//!
//! Statements end at a newline, using the same rule as Go: a newline ends
//! the statement when the last token on the line could end one (a name, a
//! literal, `)`, `]`, `}`, `return`, `break`, `continue`). Inside `( )` and
//! `[ ]` newlines never end a statement, so long calls and lists can be
//! split over several lines.

use crate::diag::{Diag, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(u64),
    Float(f64),
    Str(Vec<StrTok>),
    // keywords
    Fn,
    Let,
    Var,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Break,
    Continue,
    Struct,
    Use,
    Pub,
    Enum,
    Match,
    NoneKw,
    True,
    False,
    And,
    Or,
    Not,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    DotDot,
    Arrow,
    FatArrow,
    Question,
    Bang,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Assign,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    /// End of a statement (a newline, or the end of the file).
    Semi,
    Eof,
}

/// A piece of a string literal: plain text, or the tokens of a `{...}`
/// interpolation.
#[derive(Debug, Clone, PartialEq)]
pub enum StrTok {
    Lit(String),
    Code(Vec<Token>, Span),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(n) => format!("name `{}`", n),
            Tok::Int(_) => "a number".into(),
            Tok::Float(_) => "a number".into(),
            Tok::Str(_) => "a string".into(),
            Tok::Semi => "end of line".into(),
            Tok::Eof => "end of file".into(),
            other => format!("`{}`", other.text()),
        }
    }
    fn text(&self) -> &'static str {
        match self {
            Tok::Fn => "fn",
            Tok::Let => "let",
            Tok::Var => "var",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::While => "while",
            Tok::For => "for",
            Tok::In => "in",
            Tok::Return => "return",
            Tok::Break => "break",
            Tok::Continue => "continue",
            Tok::Struct => "struct",
            Tok::Enum => "enum",
            Tok::Use => "use",
            Tok::Pub => "pub",
            Tok::Bang => "!",
            Tok::Match => "match",
            Tok::NoneKw => "none",
            Tok::FatArrow => "=>",
            Tok::Question => "?",
            Tok::True => "true",
            Tok::False => "false",
            Tok::And => "and",
            Tok::Or => "or",
            Tok::Not => "not",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Dot => ".",
            Tok::DotDot => "..",
            Tok::Arrow => "->",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::Assign => "=",
            Tok::PlusEq => "+=",
            Tok::MinusEq => "-=",
            Tok::StarEq => "*=",
            Tok::SlashEq => "/=",
            Tok::PercentEq => "%=",
            _ => "?",
        }
    }
}

fn keyword(s: &str) -> Option<Tok> {
    Some(match s {
        "fn" => Tok::Fn,
        "let" => Tok::Let,
        "var" => Tok::Var,
        "if" => Tok::If,
        "else" => Tok::Else,
        "while" => Tok::While,
        "for" => Tok::For,
        "in" => Tok::In,
        "return" => Tok::Return,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "struct" => Tok::Struct,
        "enum" => Tok::Enum,
        "use" => Tok::Use,
        "pub" => Tok::Pub,
        "match" => Tok::Match,
        "none" => Tok::NoneKw,
        "true" => Tok::True,
        "false" => Tok::False,
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        _ => return None,
    })
}

const MAX_STR_NEST: u32 = 8;

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    /// Nesting of ( and [ ; newlines inside them don't end statements.
    depth: u32,
    toks: Vec<Token>,
    /// true for the sub-lexer of a `{...}` inside a string.
    nested: bool,
    /// For `nexac fmt` (0.6): each token's characters (start, end) in the
    /// source, and each comment's. Same length as `toks`.
    ranges: Vec<(usize, usize)>,
    comments: Vec<(usize, usize)>,
}

/// The tokens of a file plus where each one and each comment is (as
/// character offsets), for the formatter.
pub struct Layout {
    pub toks: Vec<Token>,
    pub ranges: Vec<(usize, usize)>,
    pub comments: Vec<(usize, usize)>,
    pub chars: Vec<char>,
}

/// Lex `src` exactly as `lex` does, also keeping where tokens and
/// comments are.
pub fn lex_layout(src: &str) -> Result<Layout, Diag> {
    let mut lx = Lexer::new(src.chars().collect(), 1, 1, false);
    lx.run()?;
    Ok(Layout { toks: lx.toks, ranges: lx.ranges, comments: lx.comments, chars: lx.chars })
}

pub fn lex(src: &str) -> Result<Vec<Token>, Diag> {
    let mut lx = Lexer::new(src.chars().collect(), 1, 1, false);
    lx.run()?;
    Ok(lx.toks)
}

impl Lexer {
    fn new(chars: Vec<char>, line: u32, col: u32, nested: bool) -> Lexer {
        Lexer { chars, pos: 0, line, col, depth: 0, toks: Vec::new(), nested, ranges: Vec::new(), comments: Vec::new() }
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }
    fn here(&self) -> Span {
        Span::new(self.line, self.col)
    }
    fn push(&mut self, tok: Tok, span: Span) {
        // tokens never span lines, so the start is on the current line
        let start = if span.line == self.line && span.col <= self.col {
            self.pos - (self.col - span.col) as usize
        } else {
            self.pos
        };
        self.ranges.push((start, self.pos));
        self.toks.push(Token { tok, span });
    }

    fn ends_statement(&self) -> bool {
        matches!(
            self.toks.last().map(|t| &t.tok),
            Some(
                Tok::Ident(_)
                    | Tok::Int(_)
                    | Tok::Float(_)
                    | Tok::Str(_)
                    | Tok::True
                    | Tok::False
                    | Tok::NoneKw
                    | Tok::Return
                    | Tok::Break
                    | Tok::Continue
                    | Tok::RParen
                    | Tok::RBracket
                    | Tok::RBrace
                    | Tok::Question
            )
        )
    }

    fn run(&mut self) -> Result<(), Diag> {
        while let Some(c) = self.peek() {
            let start = self.here();
            match c {
                '\n' => {
                    if self.depth == 0 && !self.nested && self.ends_statement() {
                        self.push(Tok::Semi, start);
                    }
                    self.bump();
                }
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '/' if self.peek2() == Some('/') => {
                    let from = self.pos;
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                    self.comments.push((from, self.pos));
                }
                '"' => self.string()?,
                c if c.is_ascii_digit() => self.number()?,
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let mut s = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            s.push(c);
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    let tok = keyword(&s).unwrap_or(Tok::Ident(s));
                    self.push(tok, start);
                }
                _ => self.punct(start)?,
            }
        }
        if !self.nested && self.ends_statement() {
            let s = self.here();
            self.push(Tok::Semi, s);
        }
        let s = self.here();
        self.push(Tok::Eof, s);
        Ok(())
    }

    fn punct(&mut self, start: Span) -> Result<(), Diag> {
        let c = self.bump().unwrap();
        let next = self.peek();
        let two = |lx: &mut Lexer, t: Tok| {
            lx.bump();
            t
        };
        let tok = match (c, next) {
            ('(', _) => {
                self.depth += 1;
                Tok::LParen
            }
            (')', _) => {
                self.depth = self.depth.saturating_sub(1);
                Tok::RParen
            }
            ('[', _) => {
                self.depth += 1;
                Tok::LBracket
            }
            (']', _) => {
                self.depth = self.depth.saturating_sub(1);
                Tok::RBracket
            }
            ('{', _) => Tok::LBrace,
            ('}', _) => Tok::RBrace,
            (',', _) => Tok::Comma,
            (':', _) => Tok::Colon,
            ('.', Some('.')) => two(self, Tok::DotDot),
            ('.', _) => Tok::Dot,
            ('-', Some('>')) => two(self, Tok::Arrow),
            ('+', Some('=')) => two(self, Tok::PlusEq),
            ('-', Some('=')) => two(self, Tok::MinusEq),
            ('*', Some('=')) => two(self, Tok::StarEq),
            ('/', Some('=')) => two(self, Tok::SlashEq),
            ('%', Some('=')) => two(self, Tok::PercentEq),
            ('=', Some('=')) => two(self, Tok::EqEq),
            ('=', Some('>')) => two(self, Tok::FatArrow),
            ('?', _) => Tok::Question,
            ('!', Some('=')) => two(self, Tok::NotEq),
            ('<', Some('=')) => two(self, Tok::Le),
            ('>', Some('=')) => two(self, Tok::Ge),
            ('+', _) => Tok::Plus,
            ('-', _) => Tok::Minus,
            ('*', _) => Tok::Star,
            ('/', _) => Tok::Slash,
            ('%', _) => Tok::Percent,
            ('<', _) => Tok::Lt,
            ('>', _) => Tok::Gt,
            ('=', _) => Tok::Assign,
            ('!', _) => Tok::Bang,
            ('&', Some('&')) => return Err(Diag::new(start, "unexpected `&&`").hint("write `and`")),
            ('|', Some('|')) => return Err(Diag::new(start, "unexpected `||`").hint("write `or`")),
            (';', _) => {
                return Err(Diag::new(start, "unexpected `;`").hint("statements end at the end of the line; no `;` needed"))
            }
            _ => {
                let d = Diag::new(start, format!("unexpected character `{}`", c));
                return Err(if c.is_alphabetic() {
                    d.hint("names may only use the letters a-z, A-Z, digits and `_`")
                } else {
                    d
                });
            }
        };
        self.push(tok, start);
        Ok(())
    }

    /// After a `{` in a string: the code up to the matching `}` (which is
    /// consumed), copied exactly. Strings inside are allowed, up to
    /// MAX_STR_NEST levels deep.
    fn capture_code(&mut self, open: Span, depth: u32) -> Result<String, Diag> {
        let mut code = String::new();
        let mut braces = 0u32;
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(Diag::new(open, "this `{` in a string is never closed")
                        .hint("write `\\{` for a literal brace"))
                }
                Some('"') => {
                    if depth + 1 > MAX_STR_NEST {
                        return Err(Diag::new(self.here(), "too many strings inside strings")
                            .hint("put the inner value in a variable first"));
                    }
                    self.capture_string(&mut code, depth + 1)?;
                    continue;
                }
                Some('{') => braces += 1,
                Some('}') if braces == 0 => {
                    self.bump();
                    return Ok(code);
                }
                Some('}') => braces -= 1,
                _ => {}
            }
            code.push(self.bump().unwrap());
        }
    }

    /// Copy a whole string literal (at `"`) into `code`, unchanged.
    fn capture_string(&mut self, code: &mut String, depth: u32) -> Result<(), Diag> {
        let start = self.here();
        code.push(self.bump().unwrap());
        loop {
            match self.peek() {
                None | Some('\n') => return Err(Diag::new(start, "this string never ends").hint("add a closing `\"`")),
                Some('\\') => {
                    code.push(self.bump().unwrap());
                    if let Some(c) = self.peek() {
                        if c != '\n' {
                            code.push(c);
                            self.bump();
                        }
                    }
                }
                Some('"') => {
                    code.push(self.bump().unwrap());
                    return Ok(());
                }
                Some('{') => {
                    let open = self.here();
                    code.push(self.bump().unwrap());
                    let inner = self.capture_code(open, depth)?;
                    code.push_str(&inner);
                    code.push('}');
                }
                Some(_) => code.push(self.bump().unwrap()),
            }
        }
    }

    fn number(&mut self) -> Result<(), Diag> {
        let start = self.here();
        let mut s = String::new();
        if self.peek() == Some('0') && matches!(self.peek2(), Some('x') | Some('X')) {
            self.bump();
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    s.push(c);
                } else if c != '_' {
                    break;
                }
                self.bump();
            }
            if s.is_empty() {
                return Err(Diag::new(start, "`0x` needs hex digits after it"));
            }
            let v = u64::from_str_radix(&s, 16)
                .map_err(|_| Diag::new(start, "this number is too big for an `int` (64 bits)"))?;
            self.push(Tok::Int(v), start);
            return Ok(());
        }
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.bump();
            } else if c == '_' {
                self.bump();
            } else if c == '.' && !is_float && self.peek2().is_some_and(|d| d.is_ascii_digit()) {
                is_float = true;
                s.push(c);
                self.bump();
            } else if (c == 'e' || c == 'E')
                && (self.peek2().is_some_and(|d| d.is_ascii_digit())
                    || (matches!(self.peek2(), Some('+') | Some('-'))
                        && self.chars.get(self.pos + 2).is_some_and(|d| d.is_ascii_digit())))
            {
                is_float = true;
                s.push('e');
                self.bump();
                if let Some(sign @ ('+' | '-')) = self.peek() {
                    s.push(sign);
                    self.bump();
                }
            } else {
                break;
            }
        }
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            return Err(Diag::new(self.here(), "a number can't be followed directly by a letter"));
        }
        if is_float {
            let v: f64 = s.parse().map_err(|_| Diag::new(start, "this isn't a valid number"))?;
            if !v.is_finite() {
                return Err(Diag::new(start, "this number is too big for a `float`"));
            }
            self.push(Tok::Float(v), start);
        } else {
            let v: u64 = s
                .parse()
                .map_err(|_| Diag::new(start, "this number is too big for an `int` (64 bits)"))?;
            self.push(Tok::Int(v), start);
        }
        Ok(())
    }

    fn string(&mut self) -> Result<(), Diag> {
        let start = self.here();
        self.bump(); // opening "
        let mut parts: Vec<StrTok> = Vec::new();
        let mut cur = String::new();
        loop {
            let here = self.here();
            let c = match self.peek() {
                None => return Err(Diag::new(start, "this string never ends").hint("add a closing `\"`")),
                Some(c) => c,
            };
            match c {
                '"' => {
                    self.bump();
                    break;
                }
                '\n' => {
                    return Err(Diag::new(start, "this string never ends on its line")
                        .hint("strings can't span lines; use `\\n` for a line break"))
                }
                '\\' => {
                    self.bump();
                    let e = self.bump();
                    cur.push(match e {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('0') => '\0',
                        Some('\\') => '\\',
                        Some('"') => '"',
                        Some('{') => '{',
                        Some('}') => '}',
                        _ => {
                            return Err(Diag::new(here, "unknown escape in string")
                                .hint("known escapes: \\n \\t \\r \\0 \\\\ \\\" \\{ \\}"))
                        }
                    });
                }
                '{' => {
                    self.bump();
                    let code_start = self.here();
                    let code = self.capture_code(here, 0)?;
                    if code.trim().is_empty() {
                        return Err(Diag::new(here, "empty `{}` in a string")
                            .hint("put a value inside, like {name}, or write `\\{` for a literal brace"));
                    }
                    let mut sub = Lexer::new(code.chars().collect(), code_start.line, code_start.col, true);
                    sub.run()?;
                    if !cur.is_empty() {
                        parts.push(StrTok::Lit(std::mem::take(&mut cur)));
                    }
                    parts.push(StrTok::Code(sub.toks, code_start));
                }
                _ => {
                    cur.push(c);
                    self.bump();
                }
            }
        }
        if !cur.is_empty() || parts.is_empty() {
            parts.push(StrTok::Lit(cur));
        }
        self.push(Tok::Str(parts), start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn newline_ends_statement() {
        assert_eq!(
            toks("let x = 1\nx"),
            vec![
                Tok::Let,
                Tok::Ident("x".into()),
                Tok::Assign,
                Tok::Int(1),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::Semi,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn no_semi_inside_parens_or_after_operator() {
        assert_eq!(
            toks("f(1,\n2)\na +\nb"),
            vec![
                Tok::Ident("f".into()),
                Tok::LParen,
                Tok::Int(1),
                Tok::Comma,
                Tok::Int(2),
                Tok::RParen,
                Tok::Semi,
                Tok::Ident("a".into()),
                Tok::Plus,
                Tok::Ident("b".into()),
                Tok::Semi,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn range_is_not_float() {
        assert_eq!(toks("0..10")[..3], [Tok::Int(0), Tok::DotDot, Tok::Int(10)]);
        assert_eq!(toks("1.5")[0], Tok::Float(1.5));
        assert_eq!(toks("2e3")[0], Tok::Float(2000.0));
        assert_eq!(toks("1_000")[0], Tok::Int(1000));
        assert_eq!(toks("0xff")[0], Tok::Int(255));
    }

    #[test]
    fn interpolation() {
        let t = toks("\"a{x}b\"");
        match &t[0] {
            Tok::Str(p) => {
                assert_eq!(p.len(), 3);
                assert_eq!(p[0], StrTok::Lit("a".into()));
                assert!(matches!(&p[1], StrTok::Code(c, _) if c[0].tok == Tok::Ident("x".into())));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn errors() {
        assert!(lex("\"abc").is_err());
        assert!(lex("a && b").is_err());
        assert!(lex("99999999999999999999").is_err());
        assert!(lex("\"{\"x\"}\"").is_ok());
        assert!(lex("\"{f(\"a{g(\"b\")}c\")}\"").is_ok());
        let deep = "\"{".repeat(9) + &"}\"".repeat(9);
        assert!(lex(&deep).is_err());
        let ok = "\"{".repeat(8) + "1" + &"}\"".repeat(8);
        assert!(lex(&ok).is_ok());
        assert!(lex("\"{\"x}\"").is_err());
        assert!(lex("\"\\q\"").is_err());
        assert!(lex("café").is_err());
    }
}
