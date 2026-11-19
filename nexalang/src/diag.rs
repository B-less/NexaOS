//! Diagnostics: one error with a position, printed with the source line
//! and a caret under the column.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(line: u32, col: u32) -> Self {
        Span { line, col }
    }
}

#[derive(Debug, Clone)]
pub struct Diag {
    /// Index of the source file (module); None until the checker stamps it.
    pub file: Option<usize>,
    pub msg: String,
    pub span: Span,
    pub hint: Option<String>,
}

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Self {
        Diag { file: None, msg: msg.into(), span, hint: None }
    }
    pub fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }

    /// Render like:
    /// error: unknown name `cout`
    ///   --> hello.nx:3:5
    ///    |
    ///  3 |     cout = 1
    ///    |     ^
    ///    = hint: ...
    pub fn render(&self, file: &str, src: &str) -> String {
        let mut s = format!("error: {}\n", self.msg);
        s += &format!("  --> {}:{}:{}\n", file, self.span.line, self.span.col);
        if self.span.line >= 1 {
            if let Some(text) = src.lines().nth(self.span.line as usize - 1) {
                let num = self.span.line.to_string();
                let pad = " ".repeat(num.len());
                s += &format!(" {} |\n", pad);
                s += &format!(" {} | {}\n", num, text);
                let caret_pad: String = text
                    .chars()
                    .take(self.span.col.saturating_sub(1) as usize)
                    .map(|c| if c == '\t' { '\t' } else { ' ' })
                    .collect();
                s += &format!(" {} | {}^\n", pad, caret_pad);
            }
        }
        if let Some(h) = &self.hint {
            s += &format!("  = hint: {}\n", h);
        }
        s
    }
}

/// A finished report of one mistake: everything needed to show it as text
/// (exactly as `Diag::render` does) or as JSON for editors (0.6).
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub file: String,
    /// 1-based; 0 when the problem isn't about one place (a file that
    /// can't be read).
    pub line: u32,
    /// 1-based, counted in characters (Unicode scalar values); a tab is one.
    pub col: u32,
    pub msg: String,
    pub hint: Option<String>,
    /// How many characters from `col` the mistake covers, for editors to
    /// underline: the whole word or number there, else 1 (0 when `line`
    /// is 0).
    pub length: u32,
    /// The rendered text, made once so text output can't drift from 0.5's.
    pub text: String,
}

impl Report {
    pub fn at(d: &Diag, file: &str, src: &str) -> Report {
        Report {
            file: file.to_string(),
            line: d.span.line,
            col: d.span.col,
            msg: d.msg.clone(),
            hint: d.hint.clone(),
            length: word_length(src, d.span),
            text: d.render(file, src),
        }
    }
    /// A problem with no position. `msg` is the message without `error: `.
    pub fn general(file: &str, msg: impl Into<String>) -> Report {
        let msg = msg.into();
        Report { file: file.to_string(), line: 0, col: 0, length: 0, text: format!("error: {}\n", msg), msg, hint: None }
    }
}

/// The length in characters of the word (letters, digits, `_`) starting
/// at `span`, or 1 for anything else that's there, or 0 past the line.
fn word_length(src: &str, span: Span) -> u32 {
    if span.line == 0 || span.col == 0 {
        return 0;
    }
    let Some(line) = src.lines().nth(span.line as usize - 1) else { return 0 };
    let mut chars = line.chars().skip(span.col as usize - 1).peekable();
    match chars.peek() {
        None => 0,
        Some(c) if c.is_alphanumeric() || *c == '_' => chars.take_while(|c| c.is_alphanumeric() || *c == '_').count() as u32,
        Some(_) => 1,
    }
}

/// All reports as text, the way nexac has always printed them.
pub fn render_reports(rs: &[Report]) -> String {
    let mut s = rs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("\n");
    if rs.len() > 1 {
        s += &format!("\n{} errors\n", rs.len());
    }
    s
}
