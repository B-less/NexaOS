//! Type checker. Fills in every `Expr::ty`, reports all errors it can find
//! (one per mistake), and records what code generation needs.

use crate::ast::*;
use crate::diag::{Diag, Span};
use std::collections::{HashMap, HashSet};

/// Built-ins with special typing rules.
const SPECIAL: &[&str] = &["print", "len", "push", "pop", "to_str", "some", "unwrap_or", "ok", "err"];

/// Built-ins with a fixed signature.
pub fn fixed_builtin(name: &str) -> Option<(Vec<Type>, Type)> {
    use Type::*;
    let s = || Str;
    let maybe = |t: Type| Maybe(Box::new(t));
    let list = |t: Type| Array(Box::new(t));
    Some(match name {
        "to_float" => (vec![Int], Float),
        "to_int" => (vec![Float], Int),
        "assert" => (vec![Bool], Void),
        "parse_int" => (vec![s()], maybe(Int)),
        "parse_float" => (vec![s()], maybe(Float)),
        "find" => (vec![s(), s()], maybe(Int)),
        "contains" | "starts_with" | "ends_with" => (vec![s(), s()], Bool),
        "slice" => (vec![s(), Int, Int], s()),
        "split" => (vec![s(), s()], list(s())),
        "join" => (vec![list(s()), s()], s()),
        "trim" | "to_upper" | "to_lower" => (vec![s()], s()),
        "replace" => (vec![s(), s(), s()], s()),
        "repeat" => (vec![s(), Int], s()),
        "read_line" => (vec![], maybe(s())),
        "args" => (vec![], list(s())),
        "byte_at" => (vec![s(), Int], Int),
        "char_from_code" => (vec![Int], s()),
        "exit" => (vec![Int], Void),
        _ => return std_primitive(name),
    })
}

/// C primitives only the standard modules may call (`__name`).
fn std_primitive(name: &str) -> Option<(Vec<Type>, Type)> {
    use Type::*;
    let s = || Str;
    let res = |t: Type| Result(Box::new(t));
    let list = |t: Type| Array(Box::new(t));
    Some(match name {
        "__files_read" => (vec![s()], res(s())),
        "__files_write" => (vec![s(), s()], res(Void)),
        "__files_append" => (vec![s(), s()], res(Void)),
        "__files_exists" => (vec![s()], Bool),
        "__files_size" => (vec![s()], res(s())),
        "__files_is_dir" => (vec![s()], Bool),
        "__files_list" => (vec![s()], res(list(s()))),
        "__files_make_dir" => (vec![s()], res(Void)),
        "__files_remove" => (vec![s()], res(Void)),
        "__env" => (vec![s()], Maybe(Box::new(s()))),
        "__time_unix" => (vec![], Int),
        "__time_millis" => (vec![], Int),
        "__time_sleep_ms" => (vec![Int], Void),
        "__time_format" => (vec![Int, s(), Bool], res(s())),
        "__unix_request" => (vec![s(), s(), Int], res(s())),
        // windows (std/ui.nx over runtime_ui.h)
        "__ui_start" => (vec![s(), s()], res(Void)),
        "__ui_box" => (vec![Int, Bool], Int),
        "__ui_label" | "__ui_button" => (vec![Int, s(), Int], Int),
        "__ui_entry" | "__ui_list" => (vec![Int, s()], Int),
        "__ui_check" => (vec![Int, s(), Bool], Int),
        "__ui_set_text" | "__ui_tooltip" | "__ui_shortcut" => (vec![Int, s()], Void),
        "__ui_get_text" => (vec![Int], s()),
        "__ui_set_checked" | "__ui_set_enabled" | "__ui_set_visible" => (vec![Int, Bool], Void),
        "__ui_is_checked" => (vec![Int], Bool),
        "__ui_set_items" => (vec![Int, list(s())], Void),
        "__ui_selected" => (vec![Int], Int),
        "__ui_select" => (vec![Int, Int], Void),
        "__ui_focus" => (vec![Int], Void),
        "__ui_status" => (vec![s(), Int], Void),
        "__ui_wait" => (vec![Int], Int),
        "__ui_event_id" | "__ui_event_num" => (vec![], Int),
        "__ui_event_text" => (vec![], s()),
        "__ui_ask_password" => (vec![s(), s()], Maybe(Box::new(s()))),
        "__ui_confirm" => (vec![s(), s(), s(), Bool], Bool),
        "__ui_code" => (vec![Int, s()], Int),
        "__ui_remove" => (vec![Int, Bool], Void),
        "__ui_to_clipboard" => (vec![s()], Void),
        "__ui_choose" => (vec![s(), s(), Bool], Maybe(Box::new(s()))),
        // 0.8: adding to code views, starting programs, the code editor
        "__ui_append" => (vec![Int, s()], Void),
        "__ui_start_program" => (vec![s(), list(s()), s()], Int),
        "__ui_program_error" => (vec![], s()),
        "__ui_stop_program" => (vec![Int], Void),
        "__editor_add" => (vec![Int], Int),
        "__editor_replace" => (vec![Int, s()], Void),
        "__editor_mark" => (vec![Int, Int, Int, Int], Void),
        "__editor_clear_marks" => (vec![Int], Void),
        "__editor_go_to" => (vec![Int, Int, Int], Void),
        _ => return None,
    })
}

pub fn is_builtin(name: &str) -> bool {
    SPECIAL.contains(&name) || fixed_builtin(name).is_some()
}

#[derive(Clone)]
pub struct FnSig {
    pub params: Vec<Type>,
    pub ret: Type,
}

struct FnInfo {
    sig: FnSig,
    public: bool,
    module: usize,
}

struct TypeInfo {
    is_enum: bool,
    module: usize,
    public: bool,
}

pub struct Checked {
    pub struct_fields: HashMap<String, Vec<(String, Type)>>,
    /// variant name, payload types; in declaration order
    pub enum_variants: HashMap<String, Vec<(String, Vec<Type>)>>,
    /// Every struct and enum, in declaration order (for reproducible output).
    pub named_types: Vec<Type>,
    /// keyed by canonical name: `name` in the main program, `module.name` elsewhere
    pub fns: HashMap<String, FnSig>,
}

/// Canonical name of function `name` in module `module`.
pub fn canonical(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", module, name)
    }
}

struct VarInfo {
    ty: Type,
    mutable: bool,
}

struct Checker {
    structs: HashMap<String, Vec<(String, Type)>>,
    enums: HashMap<String, Vec<(String, Vec<Type>)>>,
    types: HashMap<String, TypeInfo>,
    fns: HashMap<String, FnInfo>,
    /// module names, by index
    mod_names: Vec<String>,
    /// for each module: the modules it uses (by index)
    uses: Vec<HashSet<usize>>,
    is_std: Vec<bool>,
    cur: usize,
    scopes: Vec<HashMap<String, VarInfo>>,
    ret: Type,
    loops: u32,
    errors: Vec<Diag>,
    last_call: Option<String>,
}

pub fn check(mods: &mut [Module]) -> Result<Checked, Vec<Diag>> {
    let mut c = Checker {
        structs: HashMap::new(),
        enums: HashMap::new(),
        types: HashMap::new(),
        fns: HashMap::new(),
        mod_names: mods.iter().map(|m| m.name.clone()).collect(),
        uses: Vec::new(),
        is_std: mods.iter().map(|m| m.is_std).collect(),
        cur: 0,
        scopes: Vec::new(),
        ret: Type::Void,
        loops: 0,
        errors: Vec::new(),
        last_call: None,
    };
    for m in mods.iter() {
        let set: HashSet<usize> = m
            .prog
            .uses
            .iter()
            .filter_map(|(u, _)| mods.iter().position(|x| x.name == *u))
            .collect();
        c.uses.push(set);
    }

    // 1. type names (structs and enums share one namespace across files)
    let mut named_types = Vec::new();
    // standard modules first, so a clash with one is reported in the
    // program's own file, where it can be fixed
    let mut order: Vec<usize> = (0..mods.len()).collect();
    order.sort_by_key(|&i| !mods[i].is_std);
    for mi in order {
        let m = &mods[mi];
        c.cur = mi;
        for (name, span, is_enum, public) in m
            .prog
            .structs
            .iter()
            .map(|s| (&s.name, s.span, false, s.public))
            .chain(m.prog.enums.iter().map(|e| (&e.name, e.span, true, e.public)))
        {
            if let Some(prev) = c.types.get(name) {
                let msg = if prev.module == mi {
                    format!("the type `{}` is defined twice", name)
                } else {
                    format!("the type `{}` is also defined in {}; type names must be different across files", name, c.where_is(prev.module))
                };
                c.err(span, msg);
                continue;
            }
            c.types.insert(name.clone(), TypeInfo { is_enum, module: mi, public });
            named_types.push(if is_enum { Type::Enum(name.clone()) } else { Type::Struct(name.clone()) });
        }
        c.stamp();
    }
    // 2. struct fields and enum variants
    for (mi, m) in mods.iter().enumerate() {
        c.cur = mi;
        for s in &m.prog.structs {
            let mut fields = Vec::new();
            let mut fseen = HashSet::new();
            for f in &s.fields {
                if !fseen.insert(f.name.clone()) {
                    c.err(f.span, format!("field `{}` appears twice in `{}`", f.name, s.name));
                }
                let t = c.resolve(&f.ty);
                if t == Type::Void {
                    c.err(f.ty.span(), "a field can't have the type `void`");
                }
                if s.public {
                    c.exposes(&t, f.ty.span(), &format!("pub struct `{}`", s.name));
                }
                fields.push((f.name.clone(), t));
            }
            c.structs.entry(s.name.clone()).or_insert(fields);
        }
        for e in &m.prog.enums {
            let mut vs = Vec::new();
            let mut vseen = HashSet::new();
            if e.variants.is_empty() {
                c.err(e.span, format!("enum `{}` needs at least one variant", e.name));
            }
            for v in &e.variants {
                if !vseen.insert(v.name.clone()) {
                    c.err(v.span, format!("variant `{}` appears twice in `{}`", v.name, e.name));
                }
                let mut tys = Vec::new();
                for t in &v.payload {
                    let r = c.resolve(t);
                    if r == Type::Void {
                        c.err(t.span(), "a variant can't hold `void`");
                    }
                    if e.public {
                        c.exposes(&r, t.span(), &format!("pub enum `{}`", e.name));
                    }
                    tys.push(r);
                }
                vs.push((v.name.clone(), tys));
            }
            c.enums.entry(e.name.clone()).or_insert(vs);
        }
        c.stamp();
    }
    // 3. a type can't contain itself by value
    c.check_cycles(mods);

    // 4. function signatures
    for (mi, m) in mods.iter().enumerate() {
        c.cur = mi;
        for f in &m.prog.fns {
            if is_builtin(&f.name) {
                c.err(f.span, format!("`{}` is a built-in function; pick another name", f.name));
                continue;
            }
            if f.name.starts_with("__") && !m.is_std {
                c.err(f.span, "names starting with `__` are reserved for the standard modules");
            }
            if !lower_name(&f.name) {
                c.err(f.span, format!("function names start with a lowercase letter: `{}`", lowercase(&f.name)));
            }
            let key = canonical(&m.name, &f.name);
            if c.fns.contains_key(&key) {
                c.err(f.span, format!("function `{}` is defined twice", f.name));
                continue;
            }
            let params: Vec<Type> = f.params.iter().map(|p| c.resolve(&p.ty)).collect();
            for (p, t) in f.params.iter().zip(&params) {
                if *t == Type::Void {
                    c.err(p.ty.span(), "a parameter can't have the type `void`");
                }
                if f.public {
                    c.exposes(t, p.ty.span(), &format!("pub fn `{}`", f.name));
                }
            }
            let ret = match &f.ret {
                Some(t) => c.resolve(t),
                None => Type::Void,
            };
            if let (true, Some(t)) = (f.public, &f.ret) {
                c.exposes(&ret, t.span(), &format!("pub fn `{}`", f.name));
            }
            c.fns.insert(key, FnInfo { sig: FnSig { params, ret }, public: f.public, module: mi });
        }
        c.stamp();
    }
    c.cur = 0;
    match c.fns.get("main") {
        None => c.err(Span::new(1, 1), "the program has no `fn main()`"),
        Some(info) => {
            let m = mods[0].prog.fns.iter().find(|f| f.name == "main").unwrap();
            let ok_ret = matches!(&info.sig.ret, Type::Void) || info.sig.ret == Type::Result(Box::new(Type::Void));
            if !info.sig.params.is_empty() || !ok_ret {
                c.errors.push(
                    Diag::new(m.span, "`main` takes no parameters and returns nothing (or `!void`)")
                        .hint("write `fn main() { ... }`, or `fn main() -> !void { ... }` to use `?` in it"),
                );
            }
        }
    }
    c.stamp();

    // 5. bodies
    for (mi, m) in mods.iter_mut().enumerate() {
        c.cur = mi;
        let mname = m.name.clone();
        for f in m.prog.fns.iter_mut() {
            let sig = match c.fns.get(&canonical(&mname, &f.name)) {
                Some(s) => s.sig.clone(),
                None => continue,
            };
            c.ret = sig.ret.clone();
            c.scopes = vec![HashMap::new()];
            for (p, t) in f.params.iter().zip(sig.params) {
                c.declare(&p.name, t, false, p.span);
            }
            c.block(&mut f.body, false);
            let key = canonical(&mname, &f.name);
            if let Flow::Recurses(span) = block_flow(&f.body, &key) {
                c.errors.push(
                    Diag::new(span, format!("`{}` always calls itself here, before it can return, so it would never finish", f.name))
                        .hint("make it stop first: return before this call when the work is done (for example `if n == 0 { return ... }`)"),
                );
            }
            if sig.ret != Type::Void && sig.ret != Type::Error && !always_returns(&f.body) {
                let hint = if sig.ret == Type::Result(Box::new(Type::Void)) {
                    "add `return ok()` at the end"
                } else {
                    "add a `return` at the end"
                };
                c.errors.push(
                    Diag::new(f.end, format!("`{}` can reach its end without returning a value (of type `{}`)", f.name, sig.ret))
                        .hint(hint),
                );
            }
        }
        c.stamp();
    }

    if c.errors.is_empty() {
        let fns = c.fns.into_iter().map(|(k, v)| (k, v.sig)).collect();
        Ok(Checked { struct_fields: c.structs, enum_variants: c.enums, named_types, fns })
    } else {
        c.errors.sort_by_key(|d| (d.file, d.span.line, d.span.col));
        Err(c.errors)
    }
}

// ---- functions that always call themselves --------------------------------
//
// A function that, on every path, calls itself before it can return never
// finishes (it runs out of stack). This finds the plain cases with certainty:
// it only says `Recurses` when no `return`, `?` or `exit` can come first.

#[derive(Clone, Copy, PartialEq)]
enum Flow {
    /// every path reaches a call to the function itself (here)
    Recurses(Span),
    /// some path may leave the function (or the program) first
    MayLeave,
    /// carries on to the next statement
    FallsThrough,
}

fn block_flow(stmts: &[Stmt], me: &str) -> Flow {
    for s in stmts {
        let f = stmt_flow(s, me);
        if f != Flow::FallsThrough {
            return f;
        }
    }
    Flow::FallsThrough
}

/// The flow of an expression that's always evaluated.
fn expr_flow(e: &Expr, me: &str) -> Flow {
    if may_leave(e) {
        return Flow::MayLeave;
    }
    match self_call(e, me) {
        Some(span) => Flow::Recurses(span),
        None => Flow::FallsThrough,
    }
}

fn stmt_flow(s: &Stmt, me: &str) -> Flow {
    match s {
        Stmt::Let { value, .. } => expr_flow(value, me),
        Stmt::Assign { target, value, .. } => match expr_flow(target, me) {
            Flow::FallsThrough => expr_flow(value, me),
            f => f,
        },
        Stmt::Expr(e) => expr_flow(e, me),
        Stmt::Return { value: Some(e), .. } => match expr_flow(e, me) {
            Flow::Recurses(sp) => Flow::Recurses(sp),
            _ => Flow::MayLeave,
        },
        Stmt::Return { value: None, .. } => Flow::MayLeave,
        Stmt::If { cond, then, els } => {
            let c = expr_flow(cond, me);
            if c != Flow::FallsThrough {
                return c;
            }
            let t = block_flow(then, me);
            let e = els.as_ref().map_or(Flow::FallsThrough, |b| block_flow(b, me));
            match (t, e) {
                (Flow::Recurses(sp), Flow::Recurses(_)) => Flow::Recurses(sp),
                (Flow::MayLeave, _) | (_, Flow::MayLeave) => Flow::MayLeave,
                _ => Flow::FallsThrough,
            }
        }
        Stmt::Match { value, arms, .. } => {
            let v = expr_flow(value, me);
            if v != Flow::FallsThrough {
                return v;
            }
            let flows: Vec<Flow> = arms.iter().map(|a| block_flow(&a.body, me)).collect();
            if !flows.is_empty() && flows.iter().all(|f| matches!(f, Flow::Recurses(_))) {
                return flows[0];
            }
            if flows.contains(&Flow::MayLeave) {
                return Flow::MayLeave;
            }
            Flow::FallsThrough
        }
        // the condition or range is evaluated at least once; the body
        // maybe never
        Stmt::While { cond, body } => loop_flow(&[cond], body, me),
        Stmt::ForRange { start, end, body, .. } => loop_flow(&[start, end], body, me),
        Stmt::ForEach { iter, body, .. } => loop_flow(&[iter], body, me),
        Stmt::Break(_) | Stmt::Continue(_) => Flow::MayLeave,
    }
}

fn loop_flow(heads: &[&Expr], body: &[Stmt], me: &str) -> Flow {
    for h in heads {
        let f = expr_flow(h, me);
        if f != Flow::FallsThrough {
            return f;
        }
    }
    if body.iter().any(stmt_may_leave) {
        Flow::MayLeave
    } else {
        Flow::FallsThrough
    }
}

/// Whether running this statement could leave the function or the program.
fn stmt_may_leave(s: &Stmt) -> bool {
    let b = |blk: &Vec<Stmt>| blk.iter().any(stmt_may_leave);
    match s {
        Stmt::Return { .. } => true,
        Stmt::Let { value, .. } | Stmt::Expr(value) => may_leave(value),
        Stmt::Assign { target, value, .. } => may_leave(target) || may_leave(value),
        Stmt::If { cond, then, els } => may_leave(cond) || b(then) || els.as_ref().is_some_and(b),
        Stmt::While { cond, body } => may_leave(cond) || b(body),
        Stmt::ForRange { start, end, body, .. } => may_leave(start) || may_leave(end) || b(body),
        Stmt::ForEach { iter, body, .. } => may_leave(iter) || b(body),
        Stmt::Match { value, arms, .. } => may_leave(value) || arms.iter().any(|a| b(&a.body)),
        Stmt::Break(_) | Stmt::Continue(_) => false,
    }
}

fn children(e: &Expr) -> Vec<&Expr> {
    match &e.kind {
        ExprKind::Unary(_, a) | ExprKind::Field(a, _) | ExprKind::Try(a) => vec![a],
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => vec![a, b],
        ExprKind::Call(_, args) | ExprKind::Array(args) => args.iter().collect(),
        ExprKind::Variant(_, _, Some(args)) => args.iter().collect(),
        ExprKind::StructLit(_, fields) => fields.iter().map(|(_, e, _)| e).collect(),
        ExprKind::Str(pieces) => pieces
            .iter()
            .filter_map(|p| match p {
                StrPiece::Expr(e) => Some(e),
                StrPiece::Lit(_) => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// `?` or `exit(...)` anywhere in the expression.
fn may_leave(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Try(_)) || matches!(&e.kind, ExprKind::Call(n, _) if n == "exit") || children(e).into_iter().any(may_leave)
}

/// A call to `me` that's always evaluated (not on the right of `and`/`or`).
fn self_call(e: &Expr, me: &str) -> Option<Span> {
    match &e.kind {
        ExprKind::Call(n, _) if n == me => Some(e.span),
        ExprKind::Binary(BinOp::And | BinOp::Or, a, _) => self_call(a, me),
        _ => children(e).into_iter().find_map(|c| self_call(c, me)),
    }
}

fn lower_name(n: &str) -> bool {
    n.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
}

fn lowercase(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

fn printable(t: &Type) -> bool {
    matches!(t, Type::Int | Type::Float | Type::Bool | Type::Str | Type::Error)
}

/// Same type, treating an already-reported error as matching anything.
pub fn same(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Error, _) | (_, Type::Error) => true,
        (Type::Array(x), Type::Array(y)) | (Type::Maybe(x), Type::Maybe(y)) | (Type::Result(x), Type::Result(y)) => same(x, y),
        _ => a == b,
    }
}

/// Named types held by value inside `t` (lists hold a reference, so they
/// don't count).
fn by_value_names(t: &Type, out: &mut Vec<String>) {
    match t {
        Type::Struct(n) | Type::Enum(n) => out.push(n.clone()),
        Type::Maybe(i) | Type::Result(i) => by_value_names(i, out),
        _ => {}
    }
}

/// The variable at the root of `a.b[i].c`, if there is one.
pub fn root_var(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Var(n) => Some(n),
        ExprKind::Field(b, _) | ExprKind::Index(b, _) => root_var(b),
        _ => None,
    }
}

fn always_returns(b: &Block) -> bool {
    b.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If { then, els: Some(e), .. } => always_returns(then) && always_returns(e),
        Stmt::While { cond, body } => matches!(cond.kind, ExprKind::Bool(true)) && !breaks(body),
        // the checker makes every match exhaustive
        Stmt::Match { arms, .. } => arms.iter().all(|a| always_returns(&a.body)),
        _ => false,
    })
}

/// Does this loop body `break` out of *this* loop?
fn breaks(b: &Block) -> bool {
    b.iter().any(|s| match s {
        Stmt::Break(_) => true,
        Stmt::If { then, els, .. } => breaks(then) || els.as_ref().is_some_and(breaks),
        Stmt::Match { arms, .. } => arms.iter().any(|a| breaks(&a.body)),
        _ => false, // breaks inside nested loops belong to those loops
    })
}

impl Checker {
    fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.errors.push(Diag::new(span, msg));
    }

    fn where_is(&self, module: usize) -> String {
        if self.mod_names[module].is_empty() {
            "the main program".to_string()
        } else {
            format!("module `{}`", self.mod_names[module])
        }
    }

    /// Stamp errors that don't know their file yet with the current one.
    fn stamp(&mut self) {
        for d in self.errors.iter_mut() {
            if d.file.is_none() {
                d.file = Some(self.cur);
            }
        }
    }

    /// Is the named type usable from the current module? Reports if not.
    fn visible(&mut self, name: &str, span: Span) -> bool {
        let Some(info) = self.types.get(name) else { return true };
        if info.module == self.cur {
            return true;
        }
        let (m, public) = (info.module, info.public);
        if !public {
            let w = self.where_is(m);
            self.errors.push(
                Diag::new(span, format!("`{}` is private to {}", name, w)).hint("mark it `pub` there to share it"),
            );
            return false;
        }
        if !self.uses[self.cur].contains(&m) {
            let mn = self.mod_names[m].clone();
            self.errors.push(
                Diag::new(span, format!("`{}` comes from module `{}`", name, mn)).hint(format!("add `use {}` at the top", mn)),
            );
            return false;
        }
        true
    }

    /// A pub item mustn't mention a private type of its own module.
    fn exposes(&mut self, t: &Type, span: Span, what: &str) {
        match t {
            Type::Struct(n) | Type::Enum(n) => {
                if let Some(info) = self.types.get(n) {
                    if !info.public && info.module == self.cur {
                        let n = n.clone();
                        self.errors.push(
                            Diag::new(span, format!("{} uses `{}`, which is private", what, n))
                                .hint(format!("mark `{}` as `pub` too", n)),
                        );
                    }
                }
            }
            Type::Array(i) | Type::Maybe(i) | Type::Result(i) => self.exposes(i, span, what),
            _ => {}
        }
    }

    fn resolve(&mut self, t: &TypeExpr) -> Type {
        match t {
            TypeExpr::Array(inner, s) => {
                let i = self.resolve(inner);
                if i == Type::Void {
                    self.err(*s, "a list can't hold `void`");
                    return Type::Error;
                }
                Type::Array(Box::new(i))
            }
            TypeExpr::Maybe(inner, s) => {
                let i = self.resolve(inner);
                if i == Type::Void {
                    self.err(*s, "`?void` isn't a type");
                    return Type::Error;
                }
                Type::Maybe(Box::new(i))
            }
            TypeExpr::Result(inner, _) => {
                let i = self.resolve(inner);
                if i == Type::Error {
                    return Type::Error;
                }
                Type::Result(Box::new(i))
            }
            TypeExpr::Named(n, s) => match n.as_str() {
                "int" => Type::Int,
                "float" => Type::Float,
                "bool" => Type::Bool,
                "str" => Type::Str,
                "void" => Type::Void,
                _ => match self.types.get(n).map(|i| i.is_enum) {
                    Some(is_enum) => {
                        if !self.visible(n, *s) {
                            return Type::Error;
                        }
                        if is_enum {
                            Type::Enum(n.clone())
                        } else {
                            Type::Struct(n.clone())
                        }
                    }
                    None => {
                        let hint = match n.as_str() {
                            "string" | "String" => Some("the string type is `str`"),
                            "i64" | "i32" | "integer" | "long" => Some("the integer type is `int`"),
                            "f64" | "double" | "number" => Some("the decimal type is `float`"),
                            "boolean" => Some("the type is `bool`"),
                            "Option" | "Optional" | "Maybe" => Some("write `?T`, like `?int`"),
                            "Result" => Some("write `!T`, like `!str`"),
                            _ => None,
                        };
                        let mut d = Diag::new(*s, format!("unknown type `{}`", n));
                        if let Some(h) = hint {
                            d = d.hint(h);
                        }
                        self.errors.push(d);
                        Type::Error
                    }
                },
            },
        }
    }

    fn check_cycles(&mut self, mods: &[Module]) {
        let mut deps: HashMap<String, (Vec<String>, Span, usize)> = HashMap::new();
        for (mi, m) in mods.iter().enumerate() {
        let prog = &m.prog;
        for s in &prog.structs {
            let mut d = Vec::new();
            for (_, t) in self.structs.get(&s.name).into_iter().flatten() {
                by_value_names(t, &mut d);
            }
            deps.insert(s.name.clone(), (d, s.span, mi));
        }
        for e in &prog.enums {
            let mut d = Vec::new();
            for (_, ts) in self.enums.get(&e.name).into_iter().flatten() {
                for t in ts {
                    by_value_names(t, &mut d);
                }
            }
            deps.insert(e.name.clone(), (d, e.span, mi));
        }
        }
        // 0 unvisited, 1 in progress, 2 done
        let mut state: HashMap<String, u8> = HashMap::new();
        let mut names: Vec<&String> = deps.keys().collect();
        names.sort();
        let mut reported = HashSet::new();
        for n in names {
            self.visit(n, &deps, &mut state, &mut reported);
        }
    }

    fn visit(
        &mut self,
        n: &str,
        deps: &HashMap<String, (Vec<String>, Span, usize)>,
        state: &mut HashMap<String, u8>,
        reported: &mut HashSet<String>,
    ) {
        if state.get(n).copied().unwrap_or(0) != 0 {
            return;
        }
        state.insert(n.to_string(), 1);
        if let Some((ds, span, mi)) = deps.get(n) {
            for d in ds {
                match state.get(d).copied().unwrap_or(0) {
                    1 => {
                        if reported.insert(d.clone()) {
                            let mut diag = Diag::new(*span, format!("`{}` contains itself, so it would be infinitely big", d))
                                .hint("hold it in a list instead, like `children: [Node]`");
                            diag.file = Some(*mi);
                            self.errors.push(diag);
                        }
                    }
                    0 => self.visit(d, deps, state, reported),
                    _ => {}
                }
            }
        }
        state.insert(n.to_string(), 2);
    }

    fn lookup(&self, n: &str) -> Option<&VarInfo> {
        self.scopes.iter().rev().find_map(|s| s.get(n))
    }

    fn declare(&mut self, name: &str, ty: Type, mutable: bool, span: Span) {
        if !lower_name(name) {
            self.errors.push(
                Diag::new(span, format!("names of values start with a lowercase letter: `{}`", lowercase(name)))
                    .hint("names starting with a capital letter are for types"),
            );
        }
        if self.lookup(name).is_some() {
            self.errors.push(
                Diag::new(span, format!("`{}` already exists here", name))
                    .hint("NexaLang doesn't allow reusing a name in an inner block; pick a new name"),
            );
        }
        // Values may share a name with a function (`let ok = ...`, a
        // parameter called `value`): values are never called and functions
        // are never used as values, so the two can't be confused.
        self.scopes.last_mut().unwrap().insert(name.to_string(), VarInfo { ty, mutable });
    }

    fn block(&mut self, b: &mut Block, new_scope: bool) {
        if new_scope {
            self.scopes.push(HashMap::new());
        }
        let mut done = false;
        for s in b.iter_mut() {
            if done {
                let sp = stmt_span(s);
                self.err(sp, "this code can never run (it comes after `return`, `break` or `continue`)");
                break;
            }
            self.stmt(s);
            if matches!(s, Stmt::Return { .. } | Stmt::Break(_) | Stmt::Continue(_)) {
                done = true;
            }
        }
        if new_scope {
            self.scopes.pop();
        }
    }

    fn need_mutable(&mut self, target: &Expr, span: Span, what: &str) {
        match root_var(target) {
            None => self.err(span, format!("can't {} this", what)),
            Some(n) => {
                if let Some(v) = self.lookup(n) {
                    if !v.mutable {
                        let n = n.to_string();
                        self.errors.push(
                            Diag::new(span, format!("can't {} `{}`: it was made with `let` (or is a parameter)", what, n))
                                .hint(format!("make it changeable with `var {} = ...`", n)),
                        );
                    }
                }
            }
        }
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { name, mutable, ty, value, span } => {
                let want = ty.as_ref().map(|t| self.resolve(t));
                let got = self.expr(value, want.as_ref());
                if got == Type::Void {
                    self.err(value.span, "this doesn't produce a value");
                }
                let t = match want {
                    Some(w) => {
                        if !same(&w, &got) {
                            self.mismatch(value.span, &w, &got);
                        }
                        w
                    }
                    None => got,
                };
                self.declare(name, t, *mutable, *span);
            }
            Stmt::Assign { target, op, value, span } => {
                let tt = self.expr(target, None);
                let vt = self.expr(value, Some(&tt));
                self.need_mutable(target, *span, "change");
                if *op == AssignOp::Set {
                    if !same(&tt, &vt) {
                        self.mismatch(value.span, &tt, &vt);
                    }
                } else {
                    let bop = match op {
                        AssignOp::Add => BinOp::Add,
                        AssignOp::Sub => BinOp::Sub,
                        AssignOp::Mul => BinOp::Mul,
                        AssignOp::Div => BinOp::Div,
                        _ => BinOp::Rem,
                    };
                    self.arith(bop, &tt, &vt, *span);
                }
            }
            Stmt::Expr(e) => {
                self.expr(e, None);
            }
            Stmt::If { cond, then, els } => {
                self.cond(cond, "`if`");
                self.block(then, true);
                if let Some(e) = els {
                    self.block(e, true);
                }
            }
            Stmt::While { cond, body } => {
                self.cond(cond, "`while`");
                self.loops += 1;
                self.block(body, true);
                self.loops -= 1;
            }
            Stmt::ForRange { var, start, end, body, span } => {
                for e in [&mut *start, &mut *end] {
                    let t = self.expr(e, Some(&Type::Int));
                    if !same(&t, &Type::Int) {
                        self.err(e.span, format!("a range needs `int` numbers, but this is `{}`", t));
                    }
                }
                self.scopes.push(HashMap::new());
                self.declare(var, Type::Int, false, *span);
                self.loops += 1;
                self.block(body, false);
                self.loops -= 1;
                self.scopes.pop();
            }
            Stmt::ForEach { var, iter, body, span } => {
                let t = self.expr(iter, None);
                let elem = match t {
                    Type::Array(e) => *e,
                    Type::Error => Type::Error,
                    Type::Int => {
                        self.errors.push(
                            Diag::new(iter.span, "can't loop over a single `int`")
                                .hint("to count, use a range: `for i in 0..n`"),
                        );
                        Type::Error
                    }
                    Type::Str => {
                        self.errors.push(
                            Diag::new(iter.span, "can't loop over a `str` directly")
                                .hint("loop over its parts: `for word in split(text, \" \")`"),
                        );
                        Type::Error
                    }
                    other => {
                        self.err(iter.span, format!("can only loop over a list, but this is `{}`", other));
                        Type::Error
                    }
                };
                self.scopes.push(HashMap::new());
                self.declare(var, elem, false, *span);
                self.loops += 1;
                self.block(body, false);
                self.loops -= 1;
                self.scopes.pop();
            }
            Stmt::Return { value, span } => {
                let want = self.ret.clone();
                match value {
                    None => {
                        if want != Type::Void && want != Type::Error {
                            self.err(*span, format!("this function must return a `{}`", want));
                        }
                    }
                    Some(v) => {
                        let got = self.expr(v, Some(&want));
                        if want == Type::Void {
                            self.err(v.span, "this function doesn't return a value (add `-> type` to its signature)");
                        } else if !same(&want, &got) {
                            self.err(v.span, format!("this function returns `{}`, but this is `{}`", want, got));
                        }
                    }
                }
            }
            Stmt::Break(sp) | Stmt::Continue(sp) => {
                if self.loops == 0 {
                    self.err(*sp, "`break` and `continue` only work inside a loop");
                }
            }
            Stmt::Match { value, arms, span } => self.match_stmt(value, arms, *span),
        }
    }

    fn match_stmt(&mut self, value: &mut Expr, arms: &mut [Arm], span: Span) {
        let t = self.expr(value, None);
        // what each case binds, decided per arm
        let mut covered: HashSet<String> = HashSet::new();
        let mut wildcard = false;
        let all: Vec<String> = match &t {
            Type::Enum(n) => self.enums[n].iter().map(|(v, _)| v.clone()).collect(),
            Type::Maybe(_) => vec!["some".into(), "none".into()],
            Type::Result(_) => vec!["ok".into(), "err".into()],
            // numbers and text can't all be listed: `_` is required
            Type::Int | Type::Str => vec!["_".into()],
            Type::Error => vec![],
            other => {
                self.errors.push(
                    Diag::new(value.span, format!("can't `match` on a `{}`", other))
                        .hint("`match` works on enums, `?` and `!` values, `int` and `str`; use `if` for others"),
                );
                vec![]
            }
        };
        for arm in arms.iter_mut() {
            if wildcard {
                self.err(arm.span, "this case is never reached: `_` above already matches everything");
            }
            let mut binds: Vec<(String, Type, Span)> = Vec::new();
            let key: Option<String> = match (&arm.pattern, &t) {
                (_, Type::Error) => None,
                (Pattern::Wildcard, _) => {
                    wildcard = true;
                    None
                }
                (Pattern::NoneLit, Type::Maybe(_)) => Some("none".into()),
                (Pattern::Int(v), Type::Int) => Some(format!("{}", v)),
                (Pattern::Str(v), Type::Str) => Some(format!("{:?}", v)),
                (Pattern::Int(_), _) | (Pattern::Str(_), _) => {
                    self.err(arm.span, format!("this case doesn't fit a `{}`", t));
                    None
                }
                (_, Type::Int) | (_, Type::Str) => {
                    self.err(arm.span, format!("a `{}` is matched with values like `1 =>` or `\"yes\" =>`, and `_`", t));
                    None
                }
                (Pattern::Case { qual: None, name, binds: b }, Type::Result(inner)) if name == "ok" || name == "err" => {
                    let want = if name == "ok" { (**inner).clone() } else { Type::Str };
                    match b {
                        Some(b) if b.len() == 1 && want != Type::Void => binds.push((b[0].0.clone(), want, b[0].1)),
                        None if want == Type::Void => {}
                        _ if want == Type::Void => self.err(arm.span, "this result holds no value: write just `ok`"),
                        _ => self.err(arm.span, format!("write `{}(name)` to get the value out", name)),
                    }
                    Some(name.clone())
                }
                (_, Type::Result(_)) => {
                    self.err(arm.span, "a `!` value is matched with `ok(x)` and `err(message)`");
                    None
                }
                (Pattern::Case { qual: None, name, binds: b }, Type::Maybe(inner)) if name == "some" => {
                    match b {
                        Some(b) if b.len() == 1 => binds.push((b[0].0.clone(), (**inner).clone(), b[0].1)),
                        _ => self.err(arm.span, "write `some(name)` to get the value out"),
                    }
                    Some("some".into())
                }
                (Pattern::Case { qual, name, binds: b }, Type::Enum(en)) => {
                    if let Some(q) = qual {
                        if q != en {
                            self.err(arm.span, format!("this is a `{}`, not a `{}`", en, q));
                        }
                    }
                    match self.enums[en].iter().find(|(v, _)| v == name).cloned() {
                        None => {
                            self.err(arm.span, format!("`{}` has no variant `{}`", en, name));
                            None
                        }
                        Some((_, payload)) => {
                            let given = b.as_ref().map(|b| b.len()).unwrap_or(0);
                            if given != payload.len() {
                                let d = Diag::new(arm.span, format!("`{}` holds {} value(s), but the case names {}", name, payload.len(), given));
                                self.errors.push(if payload.is_empty() {
                                    d.hint(format!("write just `{}`", name))
                                } else {
                                    d.hint("use `_` for values you don't need")
                                });
                            } else if let Some(b) = b {
                                for ((bn, bs), pt) in b.iter().zip(payload) {
                                    binds.push((bn.clone(), pt, *bs));
                                }
                            }
                            Some(name.clone())
                        }
                    }
                }
                (_, Type::Maybe(_)) => {
                    self.err(arm.span, "a `?` value is matched with `some(x)` and `none`");
                    None
                }
                (_, Type::Enum(en)) => {
                    let en = en.clone();
                    self.err(arm.span, format!("`none` isn't a variant of `{}`", en));
                    None
                }
                _ => None,
            };
            if let Some(k) = key {
                if !covered.insert(k.clone()) {
                    self.err(arm.span, format!("`{}` is already handled above", k));
                }
            }
            self.scopes.push(HashMap::new());
            for (n, ty, s) in binds {
                if n != "_" {
                    self.declare(&n, ty, false, s);
                }
            }
            self.block(&mut arm.body, false);
            self.scopes.pop();
        }
        if !wildcard && all == ["_"] {
            self.errors.push(
                Diag::new(span, format!("a `match` on a `{}` needs a `_ => {{ }}` case for every other value", t))
            );
        } else if !wildcard && !all.is_empty() {
            let missing: Vec<String> = all.iter().filter(|v| !covered.contains(*v)).map(|v| format!("`{}`", v)).collect();
            if !missing.is_empty() {
                self.errors.push(
                    Diag::new(span, format!("this `match` doesn't handle {}", missing.join(", ")))
                        .hint("add those cases, or `_ => { }` for everything else"),
                );
            }
        }
    }

    fn mismatch(&mut self, span: Span, want: &Type, got: &Type) {
        let mut d = Diag::new(span, format!("expected `{}`, but this is `{}`", want, got));
        if let Type::Maybe(inner) = want {
            if same(inner, got) {
                d = d.hint("wrap it: `some(value)`");
            }
        }
        if let Type::Maybe(inner) = got {
            if same(inner, want) {
                d = d.hint("this might be `none`; use `match`, `unwrap_or(value, default)` or `value?`");
            }
        }
        if let Type::Result(inner) = want {
            if same(inner, got) {
                d = d.hint("wrap it: `ok(value)`");
            }
        }
        if let Type::Result(inner) = got {
            if same(inner, want) {
                d = d.hint("this might be an error; use `match`, `unwrap_or(value, default)` or `value?`");
            }
        }
        self.errors.push(d);
    }

    fn cond(&mut self, e: &mut Expr, what: &str) {
        let t = self.expr(e, Some(&Type::Bool));
        if !same(&t, &Type::Bool) {
            let mut d = Diag::new(e.span, format!("{} needs a `bool`, but this is `{}`", what, t));
            if t == Type::Int {
                d = d.hint("compare it, like `x != 0`");
            }
            self.errors.push(d);
        }
    }

    /// Type of `l op r` for arithmetic; reports a mismatch.
    fn arith(&mut self, op: BinOp, l: &Type, r: &Type, span: Span) -> Type {
        if *l == Type::Error || *r == Type::Error {
            return Type::Error;
        }
        match (l, r) {
            (Type::Int, Type::Int) => Type::Int,
            (Type::Float, Type::Float) if op != BinOp::Rem => Type::Float,
            (Type::Str, Type::Str) if op == BinOp::Add => Type::Str,
            (Type::Int, Type::Float) | (Type::Float, Type::Int) => {
                self.errors.push(
                    Diag::new(span, format!("can't use `{}` on an `int` and a `float`", op.symbol()))
                        .hint("convert one: `to_float(n)` or `to_int(x)`"),
                );
                Type::Error
            }
            (Type::Str, _) | (_, Type::Str) if op == BinOp::Add => {
                self.errors.push(
                    Diag::new(span, format!("can't add `{}` and `{}`", l, r))
                        .hint("put the value in the string instead: \"total: {n}\""),
                );
                Type::Error
            }
            _ => {
                self.err(span, format!("can't use `{}` on `{}` and `{}`", op.symbol(), l, r));
                Type::Error
            }
        }
    }

    fn expr(&mut self, e: &mut Expr, expected: Option<&Type>) -> Type {
        let t = self.expr_inner(e, expected);
        e.ty = t.clone();
        t
    }

    fn plain_enum(&self, n: &str) -> bool {
        self.enums.get(n).is_some_and(|vs| vs.iter().all(|(_, p)| p.is_empty()))
    }

    fn expr_inner(&mut self, e: &mut Expr, expected: Option<&Type>) -> Type {
        let span = e.span;
        match &mut e.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Str(pieces) => {
                for p in pieces.iter_mut() {
                    if let StrPiece::Expr(x) = p {
                        let t = self.expr(x, None);
                        if !printable(&t) {
                            self.err(x.span, format!("a `{}` can't be put in a string", t));
                        }
                    }
                }
                Type::Str
            }
            ExprKind::Try(x) => {
                let t = self.expr(x, None);
                let ret = self.ret.clone();
                match (&t, &ret) {
                    (Type::Error, _) => Type::Error,
                    (Type::Maybe(i), Type::Maybe(_)) | (Type::Result(i), Type::Result(_)) => (**i).clone(),
                    (Type::Maybe(_), _) | (Type::Result(_), _) => {
                        let (kind, need) = if matches!(t, Type::Maybe(_)) { ("`none`", "a `?` value") } else { ("the error", "a `!` value") };
                        let d = Diag::new(span, format!("`?` passes {} on to the caller, so this function must return {}", kind, need));
                        self.errors.push(if self.ret == Type::Void && self.cur == 0 {
                            d.hint("use `match` here, or make it `fn main() -> !void`")
                        } else {
                            d.hint("use `match` here instead")
                        });
                        Type::Error
                    }
                    _ => {
                        self.err(span, format!("`?` works on `?` and `!` values, not on a `{}`", t));
                        Type::Error
                    }
                }
            }
            ExprKind::NoneLit => match expected {
                Some(t @ Type::Maybe(_)) => t.clone(),
                _ => {
                    self.errors.push(
                        Diag::new(span, "can't tell what type this `none` is")
                            .hint("write the type: `var found: ?int = none`"),
                    );
                    Type::Error
                }
            },
            ExprKind::Var(n) => match self.lookup(n) {
                Some(v) => v.ty.clone(),
                None => {
                    let n = n.clone();
                    let d = Diag::new(span, format!("unknown name `{}`", n));
                    self.errors.push(if self.fns.contains_key(&n) {
                        d.hint(format!("`{}` is a function; call it with `{}(...)`", n, n))
                    } else if self.enums.contains_key(&n) {
                        d.hint(format!("pick a variant: `{}.Name`", n))
                    } else {
                        d
                    });
                    Type::Error
                }
            },
            ExprKind::Variant(en, vn, args) => {
                if !self.visible(&en.clone(), span) {
                    return Type::Error;
                }
                let def = match self.enums.get(en.as_str()) {
                    Some(d) => d.clone(),
                    None => {
                        let msg = if self.structs.contains_key(en.as_str()) {
                            format!("`{}` is a struct, not an enum", en)
                        } else {
                            format!("unknown enum `{}`", en)
                        };
                        self.err(span, msg);
                        for a in args.iter_mut().flatten() {
                            self.expr(a, None);
                        }
                        return Type::Error;
                    }
                };
                let Some((_, payload)) = def.iter().find(|(v, _)| v == vn) else {
                    let (en, vn) = (en.clone(), vn.clone());
                    self.err(span, format!("`{}` has no variant `{}`", en, vn));
                    for a in args.iter_mut().flatten() {
                        self.expr(a, None);
                    }
                    return Type::Error;
                };
                match args {
                    None if !payload.is_empty() => {
                        self.err(span, format!("`{}.{}` needs {} value(s): `{}.{}(...)`", en, vn, payload.len(), en, vn));
                    }
                    Some(_) if payload.is_empty() => {
                        self.errors.push(
                            Diag::new(span, format!("`{}.{}` holds no values", en, vn)).hint(format!("write `{}.{}`", en, vn)),
                        );
                    }
                    Some(a) => {
                        if a.len() != payload.len() {
                            self.err(span, format!("`{}.{}` holds {} value(s), but {} were given", en, vn, payload.len(), a.len()));
                        }
                        for (i, x) in a.iter_mut().enumerate() {
                            let want = payload.get(i);
                            let t = self.expr(x, want);
                            if let Some(w) = want {
                                if !same(w, &t) {
                                    self.mismatch(x.span, w, &t);
                                }
                            }
                        }
                    }
                    None => {}
                }
                Type::Enum(en.clone())
            }
            ExprKind::Unary(op, x) => {
                let op = *op;
                let t = self.expr(x, None);
                match (op, &t) {
                    (_, Type::Error) => Type::Error,
                    (UnOp::Neg, Type::Int) | (UnOp::Neg, Type::Float) => t,
                    (UnOp::Not, Type::Bool) => Type::Bool,
                    (UnOp::Neg, _) => {
                        self.err(span, format!("can't make a `{}` negative", t));
                        Type::Error
                    }
                    (UnOp::Not, _) => {
                        self.err(span, format!("`not` needs a `bool`, but this is `{}`", t));
                        Type::Error
                    }
                }
            }
            ExprKind::Binary(op, l, r) => {
                let op = *op;
                match op {
                    BinOp::And | BinOp::Or => {
                        for x in [&mut **l, &mut **r] {
                            let t = self.expr(x, Some(&Type::Bool));
                            if !same(&t, &Type::Bool) {
                                self.err(x.span, format!("`{}` needs `bool` on both sides, but this is `{}`", op.symbol(), t));
                            }
                        }
                        Type::Bool
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let lt = self.expr(l, None);
                        let rt = self.expr(r, Some(&lt));
                        if lt == Type::Error || rt == Type::Error {
                            return Type::Bool;
                        }
                        if lt != rt {
                            let mut d = Diag::new(span, format!("can't compare `{}` with `{}`", lt, rt));
                            if matches!((&lt, &rt), (Type::Int, Type::Float) | (Type::Float, Type::Int)) {
                                d = d.hint("convert one: `to_float(n)`");
                            }
                            self.errors.push(d);
                        } else {
                            let ordered = matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge);
                            let ok = match &lt {
                                Type::Int | Type::Float | Type::Str => true,
                                Type::Bool => !ordered,
                                Type::Enum(n) => !ordered && self.plain_enum(n),
                                _ => false,
                            };
                            if !ok {
                                let mut d = Diag::new(span, format!("`{}` can't be used on `{}`", op.symbol(), lt));
                                if matches!(lt, Type::Enum(_) | Type::Maybe(_)) && !ordered {
                                    d = d.hint("use `match` to look inside it");
                                }
                                self.errors.push(d);
                            }
                        }
                        Type::Bool
                    }
                    _ => {
                        let lt = self.expr(l, expected);
                        let rt = self.expr(r, Some(&lt));
                        self.arith(op, &lt, &rt, span)
                    }
                }
            }
            ExprKind::Field(b, f) => {
                let bt = self.expr(b, None);
                match &bt {
                    Type::Error => Type::Error,
                    Type::Struct(sn) => {
                        let found = self.structs.get(sn).and_then(|fs| fs.iter().find(|(n, _)| n == f)).map(|(_, t)| t.clone());
                        match found {
                            Some(t) => t,
                            None => {
                                let f = f.clone();
                                self.err(span, format!("`{}` has no field `{}`", bt, f));
                                Type::Error
                            }
                        }
                    }
                    _ => {
                        let mut d = Diag::new(span, format!("a `{}` has no fields", bt));
                        if f == "len" || f == "length" {
                            d = d.hint("use `len(x)`");
                        }
                        self.errors.push(d);
                        Type::Error
                    }
                }
            }
            ExprKind::Index(b, i) => {
                let bt = self.expr(b, None);
                let it = self.expr(i, Some(&Type::Int));
                if !same(&it, &Type::Int) {
                    self.err(i.span, format!("a list position must be an `int`, but this is `{}`", it));
                }
                match bt {
                    Type::Array(t) => *t,
                    Type::Error => Type::Error,
                    Type::Str => {
                        self.errors.push(
                            Diag::new(span, "strings can't be indexed").hint("use `slice(text, start, end)`"),
                        );
                        Type::Error
                    }
                    other => {
                        self.err(span, format!("can't index into a `{}`", other));
                        Type::Error
                    }
                }
            }
            ExprKind::Array(items) => {
                let want_elem = match expected {
                    Some(Type::Array(t)) => Some((**t).clone()),
                    _ => None,
                };
                if items.is_empty() {
                    return match want_elem {
                        Some(t) => Type::Array(Box::new(t)),
                        None => {
                            self.errors.push(
                                Diag::new(span, "can't tell what type this empty list holds")
                                    .hint("write the type: `var items: [int] = []`"),
                            );
                            Type::Error
                        }
                    };
                }
                let mut elem = want_elem;
                for it in items.iter_mut() {
                    let t = self.expr(it, elem.as_ref());
                    if t == Type::Void {
                        self.err(it.span, "this doesn't produce a value");
                        continue;
                    }
                    match &elem {
                        None => elem = Some(t),
                        Some(w) => {
                            if !same(w, &t) {
                                self.err(it.span, format!("every item in a list has the same type: expected `{}`, but this is `{}`", w, t));
                            }
                        }
                    }
                }
                Type::Array(Box::new(elem.unwrap_or(Type::Error)))
            }
            ExprKind::StructLit(name, fields) => {
                if !self.visible(&name.clone(), span) {
                    return Type::Error;
                }
                let def = match self.structs.get(name.as_str()) {
                    Some(d) => d.clone(),
                    None => {
                        let n = name.clone();
                        let msg = if self.enums.contains_key(&n) {
                            format!("`{}` is an enum; make one with `{}.Variant`", n, n)
                        } else {
                            format!("unknown struct `{}`", n)
                        };
                        self.err(span, msg);
                        for (_, v, _) in fields.iter_mut() {
                            self.expr(v, None);
                        }
                        return Type::Error;
                    }
                };
                let mut given = HashSet::new();
                for (fname, v, fspan) in fields.iter_mut() {
                    let want = def.iter().find(|(n, _)| n == fname).map(|(_, t)| t.clone());
                    let t = self.expr(v, want.as_ref());
                    if !given.insert(fname.clone()) {
                        self.err(*fspan, format!("field `{}` is given twice", fname));
                    }
                    match want {
                        None => self.err(*fspan, format!("`{}` has no field `{}`", name, fname)),
                        Some(w) => {
                            if !same(&w, &t) {
                                self.err(v.span, format!("field `{}` is `{}`, but this is `{}`", fname, w, t));
                            }
                        }
                    }
                }
                let missing: Vec<String> = def.iter().filter(|(n, _)| !given.contains(n)).map(|(n, _)| format!("`{}`", n)).collect();
                if !missing.is_empty() {
                    self.err(span, format!("missing field(s) {} for `{}`", missing.join(", "), name));
                }
                Type::Struct(name.clone())
            }
            ExprKind::Call(name, args) => {
                let n = name.clone();
                self.last_call = None;
                let t = self.call(&n, args, span, expected);
                // user functions get their canonical name (`module.f`)
                if let Some(k) = self.last_call.take() {
                    *name = k;
                }
                t
            }
        }
    }

    /// Canonical key for a call to `name` (`f` or `module.f`) from the
    /// current module. Reports and returns Err when it can't be called.
    fn resolve_fn(&mut self, name: &str, span: Span) -> Result<String, ()> {
        let Some((m, f)) = name.split_once('.') else {
            return Ok(canonical(&self.mod_names[self.cur], name));
        };
        let Some(mi) = self.mod_names.iter().position(|x| x == m) else {
            self.errors.push(
                Diag::new(span, format!("module `{}` isn't used here", m))
                    .hint(format!("add `use {}` at the top (it loads {}.nx from this folder, or a standard module)", m, m)),
            );
            return Err(());
        };
        if mi == 0 || !self.uses[self.cur].contains(&mi) {
            self.errors.push(Diag::new(span, format!("module `{}` isn't used here", m)).hint(format!("add `use {}` at the top", m)));
            return Err(());
        }
        let key = canonical(m, f);
        match self.fns.get(&key) {
            None => {
                self.err(span, format!("module `{}` has no function `{}`", m, f));
                Err(())
            }
            Some(info) if !info.public && info.module != self.cur => {
                self.errors.push(Diag::new(span, format!("`{}` is private to module `{}`", f, m)).hint("mark it `pub` there to share it"));
                Err(())
            }
            Some(_) => Ok(key),
        }
    }

    fn call(&mut self, name: &str, args: &mut [Expr], span: Span, expected: Option<&Type>) -> Type {
        let arity = |c: &mut Checker, n: usize| -> bool {
            if args.len() != n {
                c.err(span, format!("`{}` takes {} value(s), but {} were given", name, n, args.len()));
                false
            } else {
                true
            }
        };
        if name.starts_with("__") && !self.is_std[self.cur] {
            self.err(span, format!("`{}` is for the standard modules only", name));
            for a in args.iter_mut() {
                self.expr(a, None);
            }
            return Type::Error;
        }
        if let Some((params, ret)) = fixed_builtin(name) {
            if arity(self, params.len()) {
                for (a, p) in args.iter_mut().zip(&params) {
                    let t = self.expr(a, Some(p));
                    if !same(&t, p) {
                        self.err(a.span, format!("`{}` needs a `{}` here, but this is `{}`", name, p, t));
                    }
                }
            }
            return ret;
        }
        match name {
            "print" | "to_str" => {
                if name == "print" && args.len() > 1 {
                    self.errors.push(
                        Diag::new(span, format!("`print` takes 1 value, but {} were given", args.len()))
                            .hint("put them all in one text: print(\"My name is {name}\")"),
                    );
                    for a in args.iter_mut() {
                        self.expr(a, None);
                    }
                    return Type::Void;
                }
                if !arity(self, 1) {
                    return if name == "print" { Type::Void } else { Type::Error };
                }
                let t = self.expr(&mut args[0], None);
                if !printable(&t) {
                    let mut d = Diag::new(args[0].span, format!("a `{}` can't be turned into text", t));
                    if matches!(t, Type::Maybe(_)) {
                        d = d.hint("it might be `none`; use `match` or `unwrap_or` first");
                    }
                    self.errors.push(d);
                }
                if name == "print" { Type::Void } else { Type::Str }
            }
            "len" => {
                if !arity(self, 1) {
                    return Type::Int;
                }
                let t = self.expr(&mut args[0], None);
                if !matches!(t, Type::Str | Type::Array(_) | Type::Error) {
                    self.err(args[0].span, format!("`len` works on a list or a `str`, not a `{}`", t));
                }
                Type::Int
            }
            "some" => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                let want = match expected {
                    Some(Type::Maybe(t)) => Some((**t).clone()),
                    _ => None,
                };
                let t = self.expr(&mut args[0], want.as_ref());
                match t {
                    Type::Void => {
                        self.err(args[0].span, "this doesn't produce a value");
                        Type::Error
                    }
                    Type::Error => Type::Error,
                    Type::Maybe(_) => {
                        self.err(args[0].span, "this is already a `?` value");
                        Type::Error
                    }
                    t => Type::Maybe(Box::new(t)),
                }
            }
            "ok" => {
                let want = match expected {
                    Some(Type::Result(t)) => Some((**t).clone()),
                    _ => None,
                };
                if args.is_empty() {
                    return match want {
                        Some(Type::Void) => Type::Result(Box::new(Type::Void)),
                        _ => {
                            self.err(span, "`ok()` with nothing in it is only for `!void` results");
                            Type::Error
                        }
                    };
                }
                if !arity(self, 1) {
                    return Type::Error;
                }
                let t = self.expr(&mut args[0], want.as_ref());
                match t {
                    Type::Void => {
                        self.err(args[0].span, "this doesn't produce a value");
                        Type::Error
                    }
                    Type::Error => Type::Error,
                    t => Type::Result(Box::new(t)),
                }
            }
            "err" => {
                if !arity(self, 1) {
                    return Type::Error;
                }
                let t = self.expr(&mut args[0], Some(&Type::Str));
                if !same(&t, &Type::Str) {
                    self.err(args[0].span, format!("an error is described with a `str`, but this is `{}`", t));
                }
                match expected {
                    Some(r @ Type::Result(_)) => r.clone(),
                    _ => {
                        self.errors.push(
                            Diag::new(span, "can't tell what type this `err` is")
                                .hint("use it where a `!T` is expected, like `return err(\"...\")`"),
                        );
                        Type::Error
                    }
                }
            }
            "unwrap_or" => {
                if !arity(self, 2) {
                    return Type::Error;
                }
                let t = self.expr(&mut args[0], None);
                let inner = match &t {
                    Type::Result(i) if **i == Type::Void => {
                        self.err(args[0].span, "this result holds no value to unwrap");
                        Type::Error
                    }
                    Type::Maybe(i) | Type::Result(i) => (**i).clone(),
                    Type::Error => Type::Error,
                    other => {
                        self.err(args[0].span, format!("`unwrap_or` needs a `?` or `!` value, not a `{}`", other));
                        Type::Error
                    }
                };
                let d = self.expr(&mut args[1], Some(&inner));
                if !same(&inner, &d) {
                    self.err(args[1].span, format!("the default must be a `{}`, but this is `{}`", inner, d));
                }
                inner
            }
            "push" | "pop" => {
                let n = if name == "push" { 2 } else { 1 };
                if !arity(self, n) {
                    return Type::Error;
                }
                let t = self.expr(&mut args[0], None);
                let elem = match &t {
                    Type::Array(e) => (**e).clone(),
                    Type::Error => Type::Error,
                    other => {
                        self.err(args[0].span, format!("`{}` needs a list, not a `{}`", name, other));
                        Type::Error
                    }
                };
                let a0 = args[0].clone();
                self.need_mutable(&a0, a0.span, &format!("`{}` to", name));
                if name == "push" {
                    let vt = self.expr(&mut args[1], Some(&elem));
                    if !same(&elem, &vt) {
                        self.err(args[1].span, format!("this list holds `{}`, but this is `{}`", elem, vt));
                    }
                    Type::Void
                } else {
                    elem
                }
            }
            _ => {
                let key = match self.resolve_fn(name, span) {
                    Ok(k) => k,
                    Err(()) => {
                        for a in args.iter_mut() {
                            self.expr(a, None);
                        }
                        return Type::Error;
                    }
                };
                let sig = match self.fns.get(&key) {
                    Some(s) => s.sig.clone(),
                    None => {
                        let d = Diag::new(span, format!("unknown function `{}`", name));
                        self.errors.push(match name {
                            "println" | "printf" | "echo" | "puts" => d.hint("use `print(...)`"),
                            "length" | "size" | "count" => d.hint("use `len(...)`"),
                            "append" | "add" => d.hint("use `push(list, value)`"),
                            "str" | "string" => d.hint("use `to_str(...)`"),
                            "int" => d.hint("use `to_int(...)` or `parse_int(text)`"),
                            "float" => d.hint("use `to_float(...)` or `parse_float(text)`"),
                            "substr" | "substring" => d.hint("use `slice(text, start, end)`"),
                            "index_of" | "indexOf" => d.hint("use `find(text, part)`"),
                            "input" => d.hint("use `read_line()`"),
                            "upper" => d.hint("use `to_upper(text)`"),
                            "lower" => d.hint("use `to_lower(text)`"),
                            _ => d,
                        });
                        for a in args.iter_mut() {
                            self.expr(a, None);
                        }
                        return Type::Error;
                    }
                };
                if args.len() != sig.params.len() {
                    self.err(span, format!("`{}` takes {} value(s), but {} were given", name, sig.params.len(), args.len()));
                }
                for (i, a) in args.iter_mut().enumerate() {
                    let want = sig.params.get(i);
                    let t = self.expr(a, want);
                    if let Some(w) = want {
                        if !same(w, &t) {
                            self.mismatch(a.span, w, &t);
                        }
                    }
                }
                // set last, after the arguments (which may hold calls of their own)
                self.last_call = Some(key);
                sig.ret
            }
        }
    }
}

fn stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::ForRange { span, .. }
        | Stmt::ForEach { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::Return { span, .. } => *span,
        Stmt::Break(s) | Stmt::Continue(s) => *s,
        Stmt::Expr(e) => e.span,
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => cond.span,
    }
}
