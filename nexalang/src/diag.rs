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
