//! Parser: tokens -> syntax tree. Stops at the first error.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{StrTok, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// In `if`/`while`/`for` headers `Name {` starts the block, not a
    /// struct literal (same rule as Go and Rust).
    no_struct: bool,
    /// Nesting depth, so hostile input can't overflow the stack.
    depth: u32,
}

const MAX_DEPTH: u32 = 200;

type PResult<T> = Result<T, Diag>;

pub fn parse(toks: Vec<Token>) -> PResult<Program> {
    let mut p = Parser { toks, pos: 0, no_struct: false, depth: 0 };
    p.program()
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: Tok, what: &str) -> PResult<Span> {
        if self.at(&t) {
            Ok(self.bump().span)
        } else {
            Err(self.unexpected(what))
        }
    }
    fn unexpected(&self, what: &str) -> Diag {
        Diag::new(self.span(), format!("expected {}, found {}", what, self.peek().describe()))
    }
    fn ident(&mut self, what: &str) -> PResult<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(n) => {
                let s = self.bump().span;
                Ok((n, s))
            }
            _ => Err(self.unexpected(what)),
        }
    }
    fn skip_semis(&mut self) {
        while self.at(&Tok::Semi) {
            self.bump();
        }
    }

    fn program(&mut self) -> PResult<Program> {
        let mut prog = Program { uses: vec![], structs: vec![], enums: vec![], fns: vec![] };
        loop {
            self.skip_semis();
            if self.at(&Tok::Use) {
                let s = self.bump().span;
                if !prog.fns.is_empty() || !prog.structs.is_empty() || !prog.enums.is_empty() {
                    return Err(Diag::new(s, "`use` lines go at the top of the file"));
                }
                let (m, ms) = self.ident("a module name, like `use files`")?;
                if !m.starts_with(|c: char| c.is_ascii_lowercase()) {
                    return Err(Diag::new(ms, "module names start with a lowercase letter"));
                }
                if prog.uses.iter().any(|(u, _)| *u == m) {
                    return Err(Diag::new(ms, format!("`{}` is already used above", m)));
                }
                prog.uses.push((m, ms));
                if !self.at(&Tok::Eof) && !self.eat(&Tok::Semi) {
                    return Err(self.unexpected("a new line after the module name"));
                }
                continue;
            }
            let public = self.eat(&Tok::Pub);
            match self.peek() {
                Tok::Eof if !public => break,
                Tok::Fn => {
                    let mut f = self.function()?;
                    f.public = public;
                    prog.fns.push(f);
                }
                Tok::Struct => {
                    let mut s = self.structure()?;
                    s.public = public;
                    prog.structs.push(s);
                }
                Tok::Enum => {
                    let mut e = self.enumeration()?;
                    e.public = public;
                    prog.enums.push(e);
                }
                _ => return Err(self.unexpected("`fn`, `struct` or `enum`")),
            }
        }
        Ok(prog)
    }

    fn fields(&mut self, close: Tok, what: &str) -> PResult<Vec<Field>> {
        let mut out = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&close) {
                break;
            }
            let (name, span) = self.ident(what)?;
            self.expect(Tok::Colon, "`:` and a type")?;
            let ty = self.ty()?;
            out.push(Field { name, ty, span });
            let newline = self.at(&Tok::Semi);
            self.skip_semis();
            if !self.eat(&Tok::Comma) && !newline {
                self.skip_semis();
                break;
            }
        }
        Ok(out)
    }

    fn structure(&mut self) -> PResult<StructDecl> {
        let span = self.bump().span;
        let (name, nspan) = self.ident("a struct name")?;
        if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return Err(Diag::new(nspan, format!("struct names start with a capital letter: `{}`", capitalize(&name))));
        }
        self.expect(Tok::LBrace, "`{`")?;
        let fields = self.fields(Tok::RBrace, "a field name")?;
        self.expect(Tok::RBrace, "`,` or `}`")?;
        Ok(StructDecl { public: false, name, fields, span })
    }

    fn enumeration(&mut self) -> PResult<EnumDecl> {
        let span = self.bump().span;
        let (name, nspan) = self.ident("an enum name")?;
        if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return Err(Diag::new(nspan, format!("enum names start with a capital letter: `{}`", capitalize(&name))));
        }
        self.expect(Tok::LBrace, "`{`")?;
        let mut variants = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) {
                break;
            }
            let (vname, vspan) = self.ident("a variant name")?;
            if !vname.starts_with(|c: char| c.is_ascii_uppercase()) {
                return Err(Diag::new(vspan, format!("variant names start with a capital letter: `{}`", capitalize(&vname))));
            }
            let mut payload = Vec::new();
            if self.eat(&Tok::LParen) {
                while !self.at(&Tok::RParen) {
                    payload.push(self.ty()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen, "`,` or `)`")?;
                if payload.is_empty() {
                    return Err(Diag::new(vspan, "empty `()` after a variant").hint(format!("write just `{}`", vname)));
                }
            }
            variants.push(Variant { name: vname, payload, span: vspan });
            let newline = self.at(&Tok::Semi);
            self.skip_semis();
            if !self.eat(&Tok::Comma) && !newline {
                break;
            }
        }
        self.skip_semis();
        self.expect(Tok::RBrace, "`,`, a new line or `}`")?;
        Ok(EnumDecl { public: false, name, variants, span })
    }

    fn function(&mut self) -> PResult<FnDecl> {
        let span = self.bump().span;
        let (name, _) = self.ident("a function name")?;
        self.expect(Tok::LParen, "`(`")?;
        let params = self.fields(Tok::RParen, "a parameter name")?;
        self.expect(Tok::RParen, "`,` or `)`")?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.ty()?) } else { None };
        let (body, end) = self.block()?;
        Ok(FnDecl { public: false, name, params, ret, body, span, end })
    }

    fn ty(&mut self) -> PResult<TypeExpr> {
        let s = self.span();
        if self.eat(&Tok::LBracket) {
            let inner = self.ty()?;
            self.expect(Tok::RBracket, "`]`")?;
            return Ok(TypeExpr::Array(Box::new(inner), s));
        }
        if self.eat(&Tok::Bang) {
            if matches!(self.peek(), Tok::Question | Tok::Bang) {
                return Err(Diag::new(self.span(), "`!` and `?` can't be stacked; pick one"));
            }
            let inner = self.ty()?;
            return Ok(TypeExpr::Result(Box::new(inner), s));
        }
        if self.eat(&Tok::Question) {
            if matches!(self.peek(), Tok::Question | Tok::Bang) {
                return Err(Diag::new(self.span(), "`??T` isn't allowed; one `?` is enough"));
            }
            let inner = self.ty()?;
            return Ok(TypeExpr::Maybe(Box::new(inner), s));
        }
        let (n, s) = self.ident("a type")?;
        Ok(TypeExpr::Named(n, s))
    }

    /// `{ stmt* }`; returns the block and the span of the closing brace.
    fn block(&mut self) -> PResult<(Block, Span)> {
        self.enter()?;
        let r = self.block_inner();
        self.depth -= 1;
        r
    }

    fn block_inner(&mut self) -> PResult<(Block, Span)> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) {
                break;
            }
            if self.at(&Tok::Eof) {
                return Err(self.unexpected("`}`"));
            }
            stmts.push(self.stmt()?);
            if !self.at(&Tok::RBrace) && !self.eat(&Tok::Semi) {
                return Err(self.unexpected("a new line after the statement"));
            }
        }
        let end = self.bump().span;
        Ok((stmts, end))
    }

    fn header_expr(&mut self) -> PResult<Expr> {
        let old = std::mem::replace(&mut self.no_struct, true);
        let e = self.expr();
        self.no_struct = old;
        e
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let span = self.span();
        match self.peek() {
            Tok::Let | Tok::Var => {
                let mutable = self.bump().tok == Tok::Var;
                let (name, _) = self.ident("a name")?;
                let ty = if self.eat(&Tok::Colon) { Some(self.ty()?) } else { None };
                if !self.at(&Tok::Assign) {
                    return Err(self.unexpected("`=` and a starting value").hint(
                        "every variable needs a starting value, like `var count = 0`",
                    ));
                }
                self.bump();
                let value = self.expr()?;
                Ok(Stmt::Let { name, mutable, ty, value, span })
            }
            Tok::If => self.if_stmt(),
            Tok::While => {
                self.bump();
                let cond = self.header_expr()?;
                let (body, _) = self.block()?;
                Ok(Stmt::While { cond, body })
            }
            Tok::For => {
                self.bump();
                let (var, _) = self.ident("a loop variable name")?;
                self.expect(Tok::In, "`in`")?;
                let first = self.header_expr()?;
                if self.eat(&Tok::DotDot) {
                    let end = self.header_expr()?;
                    let (body, _) = self.block()?;
                    Ok(Stmt::ForRange { var, start: first, end, body, span })
                } else {
                    let (body, _) = self.block()?;
                    Ok(Stmt::ForEach { var, iter: first, body, span })
                }
            }
            Tok::Match => self.match_stmt(),
            Tok::Return => {
                self.bump();
                let value = if self.at(&Tok::Semi) || self.at(&Tok::RBrace) { None } else { Some(self.expr()?) };
                Ok(Stmt::Return { value, span })
            }
            Tok::Break => {
                self.bump();
                Ok(Stmt::Break(span))
            }
            Tok::Continue => {
                self.bump();
                Ok(Stmt::Continue(span))
            }
            _ => {
                let e = self.expr()?;
                let op = match self.peek() {
                    Tok::Assign => Some(AssignOp::Set),
                    Tok::PlusEq => Some(AssignOp::Add),
                    Tok::MinusEq => Some(AssignOp::Sub),
                    Tok::StarEq => Some(AssignOp::Mul),
                    Tok::SlashEq => Some(AssignOp::Div),
                    Tok::PercentEq => Some(AssignOp::Rem),
                    _ => None,
                };
                if let Some(op) = op {
                    let span = self.bump().span;
                    let value = self.expr()?;
                    return Ok(Stmt::Assign { target: e, op, value, span });
                }
                // a call, or `call()?` (which can leave the function)
                if !matches!(e.kind, ExprKind::Call(..) | ExprKind::Try(_)) {
                    let mut d = Diag::new(e.span, "this value is computed but never used");
                    if let ExprKind::Binary(BinOp::Eq, ..) = e.kind {
                        d = d.hint("to change a variable use `=`, not `==`");
                    }
                    return Err(d);
                }
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn match_stmt(&mut self) -> PResult<Stmt> {
        let span = self.bump().span;
        let value = self.header_expr()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut arms = Vec::new();
        loop {
            self.skip_semis();
            if self.at(&Tok::RBrace) {
                break;
            }
            let aspan = self.span();
            let pattern = self.pattern()?;
            self.expect(Tok::FatArrow, "`=>`")?;
            if !self.at(&Tok::LBrace) {
                return Err(self.unexpected("`{`").hint("each case's code goes in braces: `Name => { ... }`"));
            }
            let (body, _) = self.block()?;
            arms.push(Arm { pattern, body, span: aspan });
            if !self.at(&Tok::RBrace) && !self.eat(&Tok::Semi) && !self.eat(&Tok::Comma) {
                return Err(self.unexpected("a new line after the case"));
            }
        }
        self.bump();
        if arms.is_empty() {
            return Err(Diag::new(span, "a `match` needs at least one case"));
        }
        Ok(Stmt::Match { value, arms, span })
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        if self.eat(&Tok::NoneKw) {
            return Ok(Pattern::NoneLit);
        }
        let neg = self.at(&Tok::Minus) && matches!(self.peek_at(1), Tok::Int(_));
        if neg {
            self.bump();
        }
        if let Tok::Int(v) = *self.peek() {
            let s = self.bump().span;
            let max = if neg { (i64::MAX as u64) + 1 } else { i64::MAX as u64 };
            if v > max {
                return Err(Diag::new(s, "this number doesn't fit in an `int`"));
            }
            return Ok(Pattern::Int(if neg { (-(v as i128)) as i64 } else { v as i64 }));
        }
        if let Tok::Str(parts) = self.peek().clone() {
            let s = self.bump().span;
            return match parts.as_slice() {
                [StrTok::Lit(l)] => Ok(Pattern::Str(l.clone())),
                _ => Err(Diag::new(s, "a case can't contain `{...}`; it must be plain text")),
            };
        }
        let (first, _) = self.ident("a case, like `Name(x)`, `some(x)`, `none` or `_`")?;
        if first == "_" {
            return Ok(Pattern::Wildcard);
        }
        let (qual, name) = if self.eat(&Tok::Dot) {
            let (n, _) = self.ident("a variant name")?;
            (Some(first), n)
        } else {
            (None, first)
        };
        let binds = if self.eat(&Tok::LParen) {
            let mut b = Vec::new();
            while !self.at(&Tok::RParen) {
                b.push(self.ident("a name for the value")?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RParen, "`,` or `)`")?;
            Some(b)
        } else {
            None
        };
        Ok(Pattern::Case { qual, name, binds })
    }

    fn if_stmt(&mut self) -> PResult<Stmt> {
        self.bump();
        let cond = self.header_expr()?;
        let (then, _) = self.block()?;
        // Allow `}` newline `else`.
        if self.at(&Tok::Semi) && *self.peek_at(1) == Tok::Else {
            self.bump();
        }
        let els = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                Some(vec![self.if_stmt()?])
            } else {
                Some(self.block()?.0)
            }
        } else {
            None
        };
        Ok(Stmt::If { cond, then, els })
    }

    // ---- expressions, lowest precedence first ----

    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Diag::new(self.span(), "this is nested too deeply").hint("split it into smaller functions"));
        }
        Ok(())
    }

    pub fn expr(&mut self) -> PResult<Expr> {
        self.enter()?;
        let e = self.or();
        self.depth -= 1;
        e
    }

    fn or(&mut self) -> PResult<Expr> {
        let mut l = self.and()?;
        let mut levels = 0;
        while self.at(&Tok::Or) {
            self.enter()?;
            levels += 1;
            let s = self.bump().span;
            let r = self.and()?;
            l = Expr::new(ExprKind::Binary(BinOp::Or, Box::new(l), Box::new(r)), s);
        }
        self.depth -= levels;
        Ok(l)
    }

    fn and(&mut self) -> PResult<Expr> {
        let mut l = self.not()?;
        let mut levels = 0;
        while self.at(&Tok::And) {
            self.enter()?;
            levels += 1;
            let s = self.bump().span;
            let r = self.not()?;
            l = Expr::new(ExprKind::Binary(BinOp::And, Box::new(l), Box::new(r)), s);
        }
        self.depth -= levels;
        Ok(l)
    }

    fn not(&mut self) -> PResult<Expr> {
        if self.at(&Tok::Not) {
            let s = self.bump().span;
            self.enter()?;
            let e = self.not();
            self.depth -= 1;
            let e = e?;
            return Ok(Expr::new(ExprKind::Unary(UnOp::Not, Box::new(e)), s));
        }
        self.cmp()
    }

    fn cmp_op(&self) -> Option<BinOp> {
        Some(match self.peek() {
            Tok::EqEq => BinOp::Eq,
            Tok::NotEq => BinOp::Ne,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            _ => return None,
        })
    }

    fn cmp(&mut self) -> PResult<Expr> {
        let l = self.add()?;
        if let Some(op) = self.cmp_op() {
            let s = self.bump().span;
            let r = self.add()?;
            if self.cmp_op().is_some() {
                return Err(Diag::new(self.span(), "comparisons can't be chained")
                    .hint("write `a < b and b < c`"));
            }
            return Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), s));
        }
        Ok(l)
    }

    fn add(&mut self) -> PResult<Expr> {
        let mut l = self.mul()?;
        let mut levels = 0;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.enter()?;
            levels += 1;
            let s = self.bump().span;
            let r = self.mul()?;
            l = Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), s);
        }
        self.depth -= levels;
        Ok(l)
    }

    fn mul(&mut self) -> PResult<Expr> {
        let mut l = self.unary()?;
        let mut levels = 0;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Rem,
                _ => break,
            };
            self.enter()?;
            levels += 1;
            let s = self.bump().span;
            let r = self.unary()?;
            l = Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), s);
        }
        self.depth -= levels;
        Ok(l)
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.at(&Tok::Minus) {
            let s = self.bump().span;
            // Fold `-<int literal>` so the smallest int can be written.
            if let Tok::Int(v) = *self.peek() {
                if !matches!(self.peek_at(1), Tok::Dot | Tok::LBracket | Tok::LParen) {
                    self.bump();
                    if v > (i64::MAX as u64) + 1 {
                        return Err(Diag::new(s, "this number is too small for an `int` (64 bits)"));
                    }
                    let n = (-(v as i128)) as i64;
                    return Ok(Expr::new(ExprKind::Int(n), s));
                }
            }
            if let Tok::Float(v) = *self.peek() {
                self.bump();
                return Ok(Expr::new(ExprKind::Float(-v), s));
            }
            self.enter()?;
            let e = self.unary();
            self.depth -= 1;
            let e = e?;
            return Ok(Expr::new(ExprKind::Unary(UnOp::Neg, Box::new(e)), s));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        let mut levels = 0;
        loop {
            if matches!(self.peek(), Tok::LParen | Tok::LBracket | Tok::Dot | Tok::Question) {
                self.enter()?;
                levels += 1;
            }
            match self.peek() {
                Tok::Question => {
                    let s = self.bump().span;
                    e = Expr::new(ExprKind::Try(Box::new(e)), s);
                }
                // `module.function(...)`: methods don't exist, so a lowercase
                // name, a dot, a name and `(` is always a call into a module.
                Tok::Dot
                    if matches!(&e.kind, ExprKind::Var(n) if n.starts_with(|c: char| c.is_ascii_lowercase()))
                        && matches!(self.peek_at(1), Tok::Ident(_))
                        && *self.peek_at(2) == Tok::LParen =>
                {
                    self.bump();
                    let (f, fs) = self.ident("a function name")?;
                    let module = match &e.kind {
                        ExprKind::Var(n) => n.clone(),
                        _ => unreachable!(),
                    };
                    e = Expr::new(ExprKind::Var(format!("{}.{}", module, f)), fs);
                }
                Tok::LParen => {
                    let s = self.bump().span;
                    let name = match &e.kind {
                        ExprKind::Var(n) => n.clone(),
                        _ => return Err(Diag::new(s, "only functions can be called, by name")),
                    };
                    let old = std::mem::replace(&mut self.no_struct, false);
                    let mut args = Vec::new();
                    while !self.at(&Tok::RParen) {
                        args.push(self.expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.no_struct = old;
                    self.expect(Tok::RParen, "`,` or `)`")?;
                    e = Expr::new(ExprKind::Call(name, args), e.span);
                }
                Tok::LBracket => {
                    let s = self.bump().span;
                    let old = std::mem::replace(&mut self.no_struct, false);
                    let i = self.expr()?;
                    self.no_struct = old;
                    self.expect(Tok::RBracket, "`]`")?;
                    e = Expr::new(ExprKind::Index(Box::new(e), Box::new(i)), s);
                }
                Tok::Dot => {
                    self.bump();
                    // `Shape.Circle(...)`: names starting with a capital
                    // are types, never variables.
                    if let ExprKind::Var(tn) = &e.kind {
                        if tn.starts_with(|c: char| c.is_ascii_uppercase()) {
                            let tn = tn.clone();
                            let (v, vs) = self.ident("a variant name")?;
                            let args = if self.at(&Tok::LParen) {
                                self.bump();
                                let old = std::mem::replace(&mut self.no_struct, false);
                                let mut args = Vec::new();
                                while !self.at(&Tok::RParen) {
                                    args.push(self.expr()?);
                                    if !self.eat(&Tok::Comma) {
                                        break;
                                    }
                                }
                                self.no_struct = old;
                                self.expect(Tok::RParen, "`,` or `)`")?;
                                Some(args)
                            } else {
                                None
                            };
                            e = Expr::new(ExprKind::Variant(tn, v, args), vs);
                            continue;
                        }
                    }
                    let (f, s) = self.ident("a field name")?;
                    e = Expr::new(ExprKind::Field(Box::new(e), f), s);
                }
                _ => break,
            }
        }
        self.depth -= levels;
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let t = self.bump();
        let s = t.span;
        let kind = match t.tok {
            Tok::Int(v) => {
                if v > i64::MAX as u64 {
                    return Err(Diag::new(s, "this number is too big for an `int` (64 bits)"));
                }
                ExprKind::Int(v as i64)
            }
            Tok::Float(v) => ExprKind::Float(v),
            Tok::NoneKw => ExprKind::NoneLit,
            Tok::Bang => {
                return Err(Diag::new(s, "unexpected `!`").hint("NexaLang uses `not`, and `!=` for 'not equal'"));
            }
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::Str(parts) => {
                let mut pieces = Vec::new();
                for p in parts {
                    match p {
                        StrTok::Lit(l) => pieces.push(StrPiece::Lit(l)),
                        StrTok::Code(toks, cs) => {
                            let mut toks = toks;
                            if toks.is_empty() || toks.last().map(|t| &t.tok) != Some(&Tok::Eof) {
                                toks.push(Token { tok: Tok::Eof, span: cs });
                            }
                            let mut sub = Parser { toks, pos: 0, no_struct: false, depth: 0 };
                            let e = sub.expr()?;
                            if !sub.at(&Tok::Eof) {
                                return Err(sub.unexpected("`}` to end the value in the string"));
                            }
                            pieces.push(StrPiece::Expr(e));
                        }
                    }
                }
                ExprKind::Str(pieces)
            }
            Tok::Ident(name) => {
                let is_type_name = name.starts_with(|c: char| c.is_ascii_uppercase());
                if is_type_name && !self.no_struct && self.at(&Tok::LBrace) {
                    self.bump();
                    let mut fields = Vec::new();
                    loop {
                        self.skip_semis();
                        if self.at(&Tok::RBrace) {
                            break;
                        }
                        let (f, fs) = self.ident("a field name")?;
                        self.expect(Tok::Colon, "`:` and a value")?;
                        let old = std::mem::replace(&mut self.no_struct, false);
                        let v = self.expr()?;
                        self.no_struct = old;
                        fields.push((f, v, fs));
                        let newline = self.at(&Tok::Semi);
                        self.skip_semis();
                        if !self.eat(&Tok::Comma) && !newline {
                            break;
                        }
                    }
                    self.skip_semis();
                    self.expect(Tok::RBrace, "`,` or `}`")?;
                    ExprKind::StructLit(name, fields)
                } else {
                    ExprKind::Var(name)
                }
            }
            Tok::LParen => {
                let old = std::mem::replace(&mut self.no_struct, false);
                let e = self.expr()?;
                self.no_struct = old;
                self.expect(Tok::RParen, "`)`")?;
                return Ok(e);
            }
            Tok::LBracket => {
                let old = std::mem::replace(&mut self.no_struct, false);
                let mut items = Vec::new();
                while !self.at(&Tok::RBracket) {
                    items.push(self.expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.no_struct = old;
                self.expect(Tok::RBracket, "`,` or `]`")?;
                ExprKind::Array(items)
            }
            other => {
                self.pos -= 1;
                let d = self.unexpected("a value");
                return Err(if other == Tok::LBrace {
                    d.hint("to put a value into text, write it inside the quotes: \"My name is {name}\"")
                } else {
                    d
                });
            }
        };
        Ok(Expr::new(kind, s))
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn p(s: &str) -> PResult<Program> {
        parse(lex(s).unwrap())
    }

    #[test]
    fn precedence() {
        let prog = p("fn main() {\n let x = 1 + 2 * 3\n}").unwrap();
        match &prog.fns[0].body[0] {
            Stmt::Let { value, .. } => match &value.kind {
                ExprKind::Binary(BinOp::Add, _, r) => assert!(matches!(r.kind, ExprKind::Binary(BinOp::Mul, ..))),
                k => panic!("{:?}", k),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn if_block_not_struct_literal() {
        assert!(p("struct P { x: int }\nfn main() {\n let a = 1\n if a > 0 { print(\"y\") }\n}").is_ok());
    }

    #[test]
    fn else_on_next_line() {
        assert!(p("fn main() {\n if true {\n }\n else {\n }\n}").is_ok());
    }

    #[test]
    fn rejects() {
        assert!(p("fn main() {\n 1 + 2\n}").is_err());
        assert!(p("fn main() {\n let x\n}").is_err());
        assert!(p("fn main() {\n let a = 1 < 2 < 3\n}").is_err());
        assert!(p("struct point { x: int }").is_err());
        assert!(p("fn main() { let x = 1 let y = 2 }").is_err());
        assert!(p("enum e { A }").is_err());
        assert!(p("enum E { a }").is_err());
        assert!(p("fn main() {\n match x {\n }\n}").is_err());
        assert!(p("fn main() {\n match x {\n A => print(1)\n }\n}").is_err());
    }

    #[test]
    fn enums_and_match() {
        let prog = p("enum Shape {\n Circle(float)\n Rect(float, float)\n Empty\n}\nfn main() {\n let s = Shape.Rect(1.0, 2.0)\n let e = Shape.Empty\n match s {\n Circle(r) => { }\n Shape.Rect(w, _) => { }\n _ => { }\n }\n let m: ?int = none\n}").unwrap();
        assert_eq!(prog.enums[0].variants.len(), 3);
        assert_eq!(prog.enums[0].variants[1].payload.len(), 2);
        match &prog.fns[0].body[0] {
            Stmt::Let { value, .. } => assert!(matches!(&value.kind, ExprKind::Variant(t, v, Some(a)) if t == "Shape" && v == "Rect" && a.len() == 2)),
            _ => panic!(),
        }
        match &prog.fns[0].body[2] {
            Stmt::Match { arms, .. } => {
                assert_eq!(arms.len(), 3);
                assert!(matches!(arms[2].pattern, Pattern::Wildcard));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn min_int_literal() {
        let prog = p("fn main() {\n let x = -9223372036854775808\n}").unwrap();
        match &prog.fns[0].body[0] {
            Stmt::Let { value, .. } => assert!(matches!(value.kind, ExprKind::Int(i64::MIN))),
            _ => panic!(),
        }
        assert!(p("fn main() {\n let x = 9223372036854775808\n}").is_err());
    }
}
