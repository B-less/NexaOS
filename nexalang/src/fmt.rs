//! `nexac fmt` (0.6): one canonical layout for NexaLang source.
//!
//! The formatter never adds, removes or reorders a token: it only changes
//! the spaces between tokens on a line, the indentation, and the number of
//! empty lines (at most one in a row). Where the program has a line break
//! between two tokens, the formatted one has one too, and where it has
//! none, neither does the formatted one. Since a line break is the only
//! whitespace NexaLang gives meaning to (it ends a statement), the
//! formatted program means exactly the same: tested by comparing the
//! generated C of every example, test and app before and after.
//!
//! Comments are kept as written (trailing spaces removed), on their own
//! line or after code, as they were. Comments after code on lines next to
//! each other are lined up, two spaces after the longest code. Strings are kept exactly, including
//! the code inside `{...}`.
//!
//! Layout rules:
//! - 4 spaces per level; a line is one level deeper than the line that
//!   opened the innermost unclosed `{`, `(` or `[`; a line starting with a
//!   closing bracket lines up with the line that opened it.
//! - One space between tokens, except: none inside `( )` and `[ ]` next to
//!   the brackets, before `,` `:` `)` `]`, around `.` and `..`, between a
//!   name and its `(` or `[`, after a unary `-`, around the `?` and `!` of
//!   types (`?int`, `!void`) and before the `?` that passes on errors.
//! - At most one empty line in a row; none at the start or end of the file,
//!   after a line whose code ends in an opening bracket, or before a line starting
//!   with a closing one. The file ends with one line break.
//! - Long lines are not split (a line break could change the meaning).
//! - Line ends are `\n`, or `\r\n` when the file's first line ends that
//!   way (0.9: files from Windows).

use crate::diag::Diag;
use crate::lexer::{lex_layout, Tok};
use crate::parser;

#[derive(Clone, Copy)]
enum Item {
    Tok(usize),
    Comment,
}

/// Ends an expression, so a following `?` is postfix, `-` is binary and
/// `[` indexes.
fn ends_expr(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Ident(_)
            | Tok::Int(_)
            | Tok::Float(_)
            | Tok::Str(_)
            | Tok::True
            | Tok::False
            | Tok::NoneKw
            | Tok::RParen
            | Tok::RBracket
            | Tok::Question
    )
}

fn is_open(t: &Tok) -> bool {
    matches!(t, Tok::LParen | Tok::LBracket | Tok::LBrace)
}
fn is_close(t: &Tok) -> bool {
    matches!(t, Tok::RParen | Tok::RBracket | Tok::RBrace)
}

/// The formatted text, or the first mistake that stops the file from
/// being read (formatting needs a file that parses; type mistakes are
/// fine).
pub fn format(src: &str) -> Result<String, Diag> {
    let out = format_lf(src)?;
    // Keep the file's line ends (0.9): a file from Windows (`\r\n`, as
    // Notepad, VS Code and git there write it) stays one, so `fmt --check`
    // agrees on every computer. The first line end decides; strings and
    // comments can't contain a line break, so only line ends change.
    match src.find('\n') {
        Some(i) if i > 0 && src.as_bytes()[i - 1] == b'\r' => Ok(out.replace('\n', "\r\n")),
        _ => Ok(out),
    }
}

fn format_lf(src: &str) -> Result<String, Diag> {
    let lay = lex_layout(src)?;
    parser::parse(lay.toks.clone())?;
    let toks = &lay.toks;

    // How each token is used, from the token before it (Semi included:
    // after a line break that ends a statement, `-1` is a new expression).
    let n = toks.len();
    let mut unary = vec![false; n]; // `-` that negates
    let mut prefix = vec![false; n]; // `?` or `!` of a type
    let mut postfix = vec![false; n]; // `?` after a value
    let mut index = vec![false; n]; // `[` after a value
    for i in 0..n {
        let prev_ends = i > 0 && ends_expr(&toks[i - 1].tok) && !prefix[i - 1];
        match toks[i].tok {
            Tok::Minus => unary[i] = !prev_ends,
            Tok::Question => {
                if prev_ends {
                    postfix[i] = true
                } else {
                    prefix[i] = true
                }
            }
            Tok::Bang => prefix[i] = true,
            Tok::LBracket => index[i] = prev_ends,
            _ => {}
        }
    }

    let mut items: Vec<(usize, usize, Item)> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if matches!(t.tok, Tok::Semi | Tok::Eof) {
            continue;
        }
        items.push((lay.ranges[i].0, lay.ranges[i].1, Item::Tok(i)));
    }
    for c in lay.comments.iter() {
        items.push((c.0, c.1, Item::Comment));
    }
    items.sort_by_key(|x| x.0);

    // group into lines, with the number of line breaks before each line
    let mut lines: Vec<(usize, Vec<(usize, usize, Item)>)> = Vec::new();
    let mut prev_end = 0;
    for it in items {
        let breaks = lay.chars[prev_end..it.0].iter().filter(|&&c| c == '\n').count();
        if breaks > 0 || lines.is_empty() {
            lines.push((breaks, Vec::new()));
        }
        lines.last_mut().unwrap().1.push(it);
        prev_end = it.1;
    }

    let text = |a: usize, b: usize| -> String { lay.chars[a..b].iter().collect() };
    let tok_of = |it: &Item| -> Option<usize> {
        match it {
            Item::Tok(i) => Some(*i),
            Item::Comment => None,
        }
    };

    // each output line: an empty line before it?, its code (with the
    // indentation), and the comment after the code
    let mut outl: Vec<(bool, String, Option<String>)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new(); // indent of the line each open bracket is on
    let mut last_opened = false; // the previous line's code ended with an opening bracket
    for (li, (breaks, line)) in lines.iter().enumerate() {
        let first = tok_of(&line[0].2);
        let starts_closed = first.is_some_and(|i| is_close(&toks[i].tok));
        let indent = if starts_closed {
            stack.last().copied().unwrap_or(0)
        } else {
            stack.last().map(|l| l + 1).unwrap_or(0)
        };
        let blank = li > 0 && *breaks > 1 && !starts_closed && !last_opened;
        let mut code = "    ".repeat(indent);
        let mut comment = None;
        let mut prev: Option<usize> = None;
        for (a, b, it) in line {
            match it {
                // a comment runs to the end of its line, so it's last
                Item::Comment => comment = Some(text(*a, *b).trim_end().to_string()),
                Item::Tok(i) => {
                    if let Some(p) = prev {
                        if space(&toks[p].tok, &toks[*i].tok, p, *i, &unary, &prefix, &postfix, &index) {
                            code.push(' ');
                        }
                    }
                    code.push_str(&text(*a, *b));
                    if is_open(&toks[*i].tok) {
                        stack.push(indent);
                    } else if is_close(&toks[*i].tok) {
                        stack.pop();
                    }
                    prev = Some(*i);
                }
            }
        }
        if prev.is_none() {
            // a line that is only a comment
            code.push_str(&comment.take().unwrap_or_default());
        }
        last_opened = line.iter().rev().find_map(|x| tok_of(&x.2)).is_some_and(|i| is_open(&toks[i].tok));
        outl.push((blank, code, comment));
    }

    // Comments after code on lines next to each other start in the same
    // column, two spaces after the longest code of the group.
    let mut col = vec![0usize; outl.len()];
    let mut i = 0;
    while i < outl.len() {
        if outl[i].2.is_none() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < outl.len() && outl[j].2.is_some() && !outl[j].0 {
            j += 1;
        }
        let width = outl[i..j].iter().map(|l| l.1.chars().count()).max().unwrap_or(0) + 2;
        for c in &mut col[i..j] {
            *c = width;
        }
        i = j;
    }

    let mut out = String::new();
    for (k, (blank, code, comment)) in outl.iter().enumerate() {
        if *blank {
            out.push('\n');
        }
        out.push_str(code);
        if let Some(c) = comment {
            let pad = col[k] - code.chars().count();
            out.push_str(&" ".repeat(pad));
            out.push_str(c);
        }
        out.push('\n');
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn space(p: &Tok, n: &Tok, pi: usize, ni: usize, unary: &[bool], prefix: &[bool], postfix: &[bool], index: &[bool]) -> bool {
    if matches!(p, Tok::LParen | Tok::LBracket) {
        return false;
    }
    if matches!(n, Tok::RParen | Tok::RBracket | Tok::Comma | Tok::Colon) {
        return false;
    }
    if matches!(p, Tok::Dot | Tok::DotDot) || matches!(n, Tok::Dot | Tok::DotDot) {
        return false;
    }
    if unary[pi] || prefix[pi] {
        return false;
    }
    if postfix[ni] {
        return false;
    }
    if matches!(n, Tok::LParen) {
        return !matches!(p, Tok::Ident(_) | Tok::RParen | Tok::RBracket);
    }
    if matches!(n, Tok::LBracket) && index[ni] {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> String {
        format(s).unwrap()
    }

    #[test]
    fn line_ends_from_windows_are_kept() {
        let crlf = "fn main(){\r\n// a note\r\nprint( \"a\rb\" )   // one\r\n\r\n\r\n}";
        let want = "fn main() {\r\n    // a note\r\n    print(\"a\rb\")  // one\r\n}\r\n";
        assert_eq!(f(crlf), want);
        assert_eq!(f(want), want);
        // the first line end decides, for a file with both
        assert_eq!(f("fn main() {\r\nprint(1)\n}\n"), "fn main() {\r\n    print(1)\r\n}\r\n");
        assert_eq!(f("fn main() {\nprint(1)\r\n}\r\n"), "fn main() {\n    print(1)\n}\n");
        assert_eq!(f("fn main() { }"), "fn main() { }\n");
    }

    #[test]
    fn spacing() {
        assert_eq!(f("fn  main( ) {\nlet x=-1+2*( 3-4 )\nprint( x )\n}"), "fn main() {\n    let x = -1 + 2 * (3 - 4)\n    print(x)\n}\n");
        assert_eq!(
            f("fn g(a:int,b : ?str)->!void{\nreturn ok( )\n}"),
            "fn g(a: int, b: ?str) -> !void {\n    return ok()\n}\n"
        );
        assert_eq!(f("fn main() {\nlet xs = [ 1,2 ]\nprint(xs [0])\nfor i in 0 .. 3 { }\n}"), "fn main() {\n    let xs = [1, 2]\n    print(xs[0])\n    for i in 0..3 { }\n}\n");
        assert_eq!(f("fn h() -> !int {\nlet v = files . read(\"x\") ?\nreturn ok(len(v))\n}"), "fn h() -> !int {\n    let v = files.read(\"x\")?\n    return ok(len(v))\n}\n");
        assert_eq!(f("fn main() {\nlet a: [?int] = []\nlet b = not(true)\n}"), "fn main() {\n    let a: [?int] = []\n    let b = not (true)\n}\n");
    }

    #[test]
    fn comments_and_blank_lines() {
        let src = "\n\n// top\nfn main() {   // trailing   \n\n\n    let x = 1\n\n\n\n  // inside\n    print(x)\n\n}\n\n\n";
        assert_eq!(f(src), "// top\nfn main() {  // trailing\n    let x = 1\n\n    // inside\n    print(x)\n}\n");
    }

    #[test]
    fn comments_after_code_line_up() {
        let src = "enum E {\n    A(int) // a\n    Bee   // b\n\n    C // c\n    // own line\n    D // d\n}\n";
        assert_eq!(f(src), "enum E {\n    A(int)  // a\n    Bee     // b\n\n    C  // c\n    // own line\n    D  // d\n}\n");
    }

    #[test]
    fn strings_are_kept_exactly() {
        let src = "fn main() {\n    print(\"a  {join( [\"x\" ,\"y\"], \", \")}\\t\\{ \")\n}\n";
        assert_eq!(f(src), src);
    }

    #[test]
    fn continued_lines() {
        let src = "fn main() {\nprint(add(1,\n2,\n3))\nlet xs = [\n1,\n2,\n]\n}\n";
        assert_eq!(f(src), "fn main() {\n    print(add(1,\n        2,\n        3))\n    let xs = [\n        1,\n        2,\n    ]\n}\n");
    }

    #[test]
    fn idempotent_and_refuses_broken_files() {
        let src = "fn main() {\nif a{\n} else{\n}\nmatch x {\nsome(v)=>{ print(v) }\nnone=>{}\n}\n}";
        let once = f(src);
        assert_eq!(f(&once), once);
        assert!(format("fn main() {").is_err());
        assert!(format("\"never ends").is_err());
        assert_eq!(f(""), "");
        assert_eq!(f("\n\n  \n"), "");
        assert_eq!(f("// only\n"), "// only\n");
        // 0.6 turned `\r\n` into `\n`; since 0.9 the file's line ends are kept
        assert_eq!(f("fn main() {\r\n    print(1)\r\n}\r\n"), "fn main() {\r\n    print(1)\r\n}\r\n");
    }
}
