//! C code generation.
//!
//! Order: NexaLang evaluates left to right. Every value that isn't a
//! literal or a plain variable goes into a numbered temporary (`nx_tN`) in
//! source order, so the C compiler can't reorder anything.
//!
//! Memory (reference counting). Each C value of a *managed* type (str,
//! list, or a struct/enum/?T that holds one) is either:
//!   * owned: it holds one reference and is released exactly once, at the
//!     end of the block that declared it, or earlier on `break`,
//!     `continue` and `return` (which release every block they leave); or
//!   * borrowed: a view of something owned elsewhere that lives longer
//!     (a variable, a list element, a field of an owned temporary).
//! `take()` turns a value into a new owned reference: an owned temporary
//! is moved (removed from its block's release list), a borrowed one is
//! retained. Parameters are borrowed (they can't be changed, so the
//! caller's value outlives the call). Variables, list elements, struct
//! fields and payloads always own their values.

use crate::ast::*;
use crate::checker::{canonical, fixed_builtin, Checked};
use std::collections::{BTreeMap, HashSet};

pub const RUNTIME: &str = include_str!("runtime.h");
/// Windows (GTK 3): only in programs that `use ui`.
pub const RUNTIME_UI: &str = include_str!("runtime_ui.h");

/// Whether the program uses the standard module `ui` (and so needs GTK).
pub fn uses_ui(mods: &[Module]) -> bool {
    mods.iter().any(|m| m.is_std && m.name == "ui")
}

/// Programs that `use editor` (0.8) also get GtkSourceView 4; others don't
/// need it installed.
pub fn uses_editor(mods: &[Module]) -> bool {
    mods.iter().any(|m| m.is_std && m.name == "editor")
}

/// NexaLang's colours for GtkSourceView and the Horizon fallback scheme,
/// built into programs that use `editor` (written to the app's cache
/// folder when it starts), so the editor always matches its compiler.
pub const EDITOR_LANG: &str = include_str!("../editors/gtksourceview/nexalang.lang");
pub const EDITOR_SCHEME: &str = include_str!("../editors/gtksourceview/nexa-horizon.xml");

/// A C expression with no side effects, and whether it's an owned temporary.
#[derive(Clone)]
struct CVal {
    code: String,
    owned: bool,
}

impl CVal {
    fn plain(code: impl Into<String>) -> Self {
        CVal { code: code.into(), owned: false }
    }
}

struct Scope {
    owned: Vec<(String, Type)>,
    is_loop: bool,
}

pub struct Gen<'a> {
    c: &'a Checked,
    out: String,
    indent: usize,
    tmp: usize,
    scopes: Vec<Scope>,
    /// ?T types used by the program (other than the runtime's own).
    maybes: BTreeMap<String, Type>,
    /// struct/enum/?T types that need generated retain/release functions.
    rc_types: BTreeMap<String, Type>,
    /// file of the function being generated (for runtime error messages)
    module: String,
    file_idx: u32,
    /// return type of the function being generated (for `?`)
    cur_ret: Type,
    /// standard modules, and whether the function being generated is in one
    std_mods: HashSet<String>,
    cur_std: bool,
    /// false: every source line is written as 0
    lines: bool,
}

/// Unambiguous short name of a type, used to build C identifiers.
fn mangle(t: &Type) -> String {
    match t {
        Type::Int => "i".into(),
        Type::Float => "f".into(),
        Type::Bool => "b".into(),
        Type::Str => "s".into(),
        Type::Void => "v".into(),
        Type::Array(i) => format!("L{}", mangle(i)),
        Type::Maybe(i) => format!("M{}", mangle(i)),
        Type::Result(i) => format!("R{}", mangle(i)),
        Type::Struct(n) => format!("S{}{}", n.len(), n),
        Type::Enum(n) => format!("E{}{}", n.len(), n),
        Type::Unknown | Type::Error => unreachable!("codegen after a failed check"),
    }
}

/// ?T and !T types the runtime already defines.
fn builtin_maybe(t: &Type) -> bool {
    match t {
        Type::Maybe(i) => matches!(**i, Type::Int | Type::Float | Type::Str),
        Type::Result(i) => match &**i {
            Type::Str | Type::Void => true,
            Type::Array(e) => **e == Type::Str,
            _ => false,
        },
        _ => false,
    }
}

/// C name of a function from its canonical name (`f` or `module.f`).
fn fn_cname(key: &str) -> String {
    match key.split_once('.') {
        None => format!("nx_fn_{}", key),
        // the length prefix keeps `a_.b` and `a._b` apart
        Some((m, f)) => format!("nx_fn_{}{}_{}", m.len(), m, f),
    }
}

/// `lines` false writes every source line as 0 (`nexac emit-c --no-lines`,
/// used to prove `nexac fmt` changes nothing but the layout).
pub fn generate(mods: &[Module], checked: &Checked, lines: bool) -> String {
    let file_name = mods[0].file.as_str();
    let mut g = Gen {
        c: checked,
        out: String::new(),
        indent: 0,
        tmp: 0,
        scopes: Vec::new(),
        maybes: BTreeMap::new(),
        rc_types: BTreeMap::new(),
        module: String::new(),
        cur_ret: Type::Void,
        file_idx: 0,
        std_mods: mods.iter().filter(|m| m.is_std).map(|m| m.name.clone()).collect(),
        cur_std: false,
        lines,
    };
    // 1. function bodies (this discovers which ?T and rc helpers are used)
    let mut protos = String::new();
    for m in mods {
        for f in &m.prog.fns {
            let key = canonical(&m.name, &f.name);
            protos += &format!("__attribute__((unused)) static {};\n", g.signature(f, &key));
        }
    }
    for (mi, m) in mods.iter().enumerate() {
        g.module = m.file.clone();
        g.file_idx = mi as u32;
        g.cur_std = m.is_std;
        for f in &m.prog.fns {
            let key = canonical(&m.name, &f.name);
            g.function(f, &key);
        }
    }
    let bodies = std::mem::take(&mut g.out);
    // 2. retain/release helpers (may discover more types)
    let rc = g.rc_helpers();
    // 3. type definitions, each after the types it contains by value
    let types = g.type_defs();

    let mut out = String::new();
    out += "/* Generated by nexac from ";
    out += &file_name.replace("*/", "* /");
    out += ". Do not edit. */\n";
    let files: Vec<String> = mods.iter().map(|m| c_string_literal(m.file.as_bytes())).collect();
    out += &format!("#define NX_FILES {}\n", files.join(", "));
    let std_flags: Vec<&str> = mods.iter().map(|m| if m.is_std { "1" } else { "0" }).collect();
    out += &format!("#define NX_STD_FILES {}\n", std_flags.join(", "));
    out += RUNTIME;
    if uses_ui(mods) {
        if uses_editor(mods) {
            out += "#define NX_UI_SOURCE 1\n";
            out += &format!("static const char NX_UI_LANG[] = {};\n", c_string_literal(EDITOR_LANG.as_bytes()));
            out += &format!("static const char NX_UI_SCHEME[] = {};\n", c_string_literal(EDITOR_SCHEME.as_bytes()));
        }
        out += RUNTIME_UI;
    }
    out += "\n/* ---- types ---- */\n";
    out += &types;
    out += "\n/* ---- memory helpers ---- */\n";
    out += &rc;
    out += "\n/* ---- program ---- */\n";
    out += &protos;
    out += &bodies;
    if checked.fns["main"].ret == Type::Void {
        out += "\nint main(int argc, char **argv) {\n    nx_init();\n    nx_argc = argc;\n    nx_argv = argv;\n    nx_fn_main();\n    return nx_finish();\n}\n";
    } else {
        // fn main() -> !void: an error ends the program with its message
        out += "\nint main(int argc, char **argv) {\n    nx_init();\n    nx_argc = argc;\n    nx_argv = argv;\n    nx_R_v r = nx_fn_main();\n    if (!r.ok) {\n        fflush(stdout);\n        fprintf(stderr, \"error: %.*s\\n\", (int)r.e.len, r.e.p);\n        nx_str_release(r.e);\n        int code = nx_finish();\n        return code ? code : 1;\n    }\n    return nx_finish();\n}\n";
    }
    out
}

/// A C string literal for any bytes. Non-printable bytes, quotes, `\` and
/// `?` (trigraphs) become 3-digit octal escapes, which can't run into the
/// next character the way `\x` escapes can.
pub fn c_string_literal(bytes: &[u8]) -> String {
    let mut s = String::from("\"");
    for &b in bytes {
        if (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\' && b != b'?' {
            s.push(b as char);
        } else {
            s += &format!("\\{:03o}", b);
        }
    }
    s.push('"');
    s
}

fn var(n: &str) -> String {
    format!("nx_v_{}", n)
}

impl<'a> Gen<'a> {
    // ---------- types ----------

    fn ctype(&mut self, t: &Type) -> String {
        match t {
            Type::Int => "int64_t".into(),
            Type::Float => "double".into(),
            Type::Bool => "bool".into(),
            Type::Str => "nx_str".into(),
            Type::Void => "void".into(),
            Type::Array(_) => "nx_arr *".into(),
            Type::Struct(n) => format!("nx_S_{}", n),
            Type::Enum(n) => format!("nx_E_{}", n),
            Type::Maybe(i) | Type::Result(i) => {
                if !builtin_maybe(t) {
                    self.maybes.entry(mangle(t)).or_insert_with(|| t.clone());
                }
                let kind = if matches!(t, Type::Maybe(_)) { "M" } else { "R" };
                format!("nx_{}_{}", kind, mangle(i))
            }
            Type::Unknown | Type::Error => unreachable!("codegen after a failed check"),
        }
    }

    fn managed(&self, t: &Type) -> bool {
        match t {
            Type::Str | Type::Array(_) => true,
            Type::Maybe(i) => self.managed(i),
            // the error message is text
            Type::Result(_) => true,
            Type::Struct(n) => self.c.struct_fields[n].iter().any(|(_, t)| self.managed(t)),
            Type::Enum(n) => self.c.enum_variants[n].iter().any(|(_, p)| p.iter().any(|t| self.managed(t))),
            _ => false,
        }
    }

    fn rc_fn(&mut self, t: &Type, kind: &str) -> String {
        match t {
            Type::Str => format!("nx_str_{}", kind),
            Type::Array(_) => format!("nx_arr_{}", kind),
            _ => {
                self.rc_types.entry(mangle(t)).or_insert_with(|| t.clone());
                let short = match kind {
                    "retain" => "rt",
                    "release" => "rl",
                    "eretain" => "er",
                    _ => "el",
                };
                format!("nx_{}_{}", short, mangle(t))
            }
        }
    }

    /// Element retain/release functions for a list of `t` ("NULL" if none).
    fn elem_fns(&mut self, t: &Type) -> (String, String) {
        if self.managed(t) {
            (self.rc_fn(t, "eretain"), self.rc_fn(t, "erelease"))
        } else {
            ("NULL".into(), "NULL".into())
        }
    }

    fn retain_expr(&mut self, t: &Type, code: &str) -> String {
        if self.managed(t) {
            format!("{}({})", self.rc_fn(t, "retain"), code)
        } else {
            code.to_string()
        }
    }

    fn release_stmt(&mut self, t: &Type, code: &str) -> String {
        format!("{}({});", self.rc_fn(t, "release"), code)
    }

    /// Generated retain/release functions for struct, enum and ?T types.
    fn rc_helpers(&mut self) -> String {
        let mut protos = String::new();
        let mut defs = String::new();
        let mut done: HashSet<String> = HashSet::new();
        loop {
            let next = self.rc_types.iter().find(|(k, _)| !done.contains(*k)).map(|(k, t)| (k.clone(), t.clone()));
            let Some((m, t)) = next else { break };
            done.insert(m.clone());
            let ct = self.ctype(&t);
            let mut rt = String::new();
            let mut rl = String::new();
            let part = |g: &mut Gen, ty: &Type, path: &str, rt: &mut String, rl: &mut String, pad: &str| {
                if g.managed(ty) {
                    let r = g.retain_expr(ty, path);
                    *rt += &format!("{}(void){};\n", pad, r);
                    *rl += &format!("{}{}\n", pad, g.release_stmt(ty, path));
                }
            };
            match &t {
                Type::Struct(n) => {
                    for (f, ft) in self.c.struct_fields[n].clone() {
                        part(self, &ft, &format!("v.nx_f_{}", f), &mut rt, &mut rl, "    ");
                    }
                }
                Type::Enum(n) => {
                    rt += "    switch (v.tag) {\n";
                    rl += "    switch (v.tag) {\n";
                    for (k, (vn, payload)) in self.c.enum_variants[n].clone().iter().enumerate() {
                        if !payload.iter().any(|p| self.managed(p)) {
                            continue;
                        }
                        rt += &format!("    case {}:\n", k);
                        rl += &format!("    case {}:\n", k);
                        for (i, pt) in payload.iter().enumerate() {
                            part(self, pt, &format!("v.u.v_{}.a{}", vn, i), &mut rt, &mut rl, "        ");
                        }
                        rt += "        break;\n";
                        rl += "        break;\n";
                    }
                    rt += "    default:\n        break;\n    }\n";
                    rl += "    default:\n        break;\n    }\n";
                }
                Type::Maybe(i) => {
                    let mut a = String::new();
                    let mut b = String::new();
                    part(self, i, "v.v", &mut a, &mut b, "        ");
                    rt += &format!("    if (v.has) {{\n{}    }}\n", a);
                    rl += &format!("    if (v.has) {{\n{}    }}\n", b);
                }
                Type::Result(i) => {
                    let mut a = String::new();
                    let mut b = String::new();
                    if **i != Type::Void {
                        part(self, i, "v.v", &mut a, &mut b, "        ");
                    }
                    rt += &format!("    if (v.ok) {{\n{}    }} else {{\n        (void)nx_str_retain(v.e);\n    }}\n", a);
                    rl += &format!("    if (v.ok) {{\n{}    }} else {{\n        nx_str_release(v.e);\n    }}\n", b);
                }
                _ => unreachable!(),
            }
            protos += &format!(
                "__attribute__((unused)) static {ct} nx_rt_{m}({ct} v);\n\
                 __attribute__((unused)) static void nx_rl_{m}({ct} v);\n\
                 __attribute__((unused)) static void nx_er_{m}(void *p);\n\
                 __attribute__((unused)) static void nx_el_{m}(void *p);\n"
            );
            defs += &format!(
                "static {ct} nx_rt_{m}({ct} v) {{\n{rt}    return v;\n}}\n\
                 static void nx_rl_{m}({ct} v) {{\n{rl}}}\n\
                 static void nx_er_{m}(void *p) {{ (void)nx_rt_{m}(*({ct} *)p); }}\n\
                 static void nx_el_{m}(void *p) {{ nx_rl_{m}(*({ct} *)p); }}\n"
            );
        }
        protos + &defs
    }

    /// Named types held by value inside `t`.
    fn deps(&self, t: &Type) -> Vec<Type> {
        let mut out = Vec::new();
        let add = |ty: &Type, out: &mut Vec<Type>| {
            if matches!(ty, Type::Struct(_) | Type::Enum(_) | Type::Maybe(_) | Type::Result(_)) {
                out.push(ty.clone());
            }
        };
        match t {
            Type::Struct(n) => self.c.struct_fields[n].iter().for_each(|(_, f)| add(f, &mut out)),
            Type::Enum(n) => self.c.enum_variants[n].iter().for_each(|(_, p)| p.iter().for_each(|f| add(f, &mut out))),
            Type::Maybe(i) | Type::Result(i) => add(i, &mut out),
            _ => {}
        }
        out
    }

    fn type_defs(&mut self) -> String {
        let mut order: Vec<Type> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut roots: Vec<Type> = self.c.named_types.clone();
        roots.extend(self.maybes.values().cloned());
        for r in roots {
            self.order_visit(&r, &mut seen, &mut order);
        }
        let mut s = String::new();
        for t in &order {
            let ct = self.ctype(t);
            s += &format!("typedef struct {0} {0};\n", ct);
        }
        for t in order.clone() {
            let ct = self.ctype(&t);
            s += &format!("struct {} {{\n", ct);
            match &t {
                Type::Struct(n) => {
                    let fields = self.c.struct_fields[n].clone();
                    if fields.is_empty() {
                        s += "    char nx_empty;\n";
                    }
                    for (f, ft) in fields {
                        s += &format!("    {} nx_f_{};\n", self.ctype(&ft), f);
                    }
                }
                Type::Enum(n) => {
                    s += "    int32_t tag;\n    union {\n        char nx_none;\n";
                    for (vn, payload) in self.c.enum_variants[n].clone() {
                        if payload.is_empty() {
                            continue;
                        }
                        s += "        struct {";
                        for (i, pt) in payload.iter().enumerate() {
                            s += &format!(" {} a{};", self.ctype(pt), i);
                        }
                        s += &format!(" }} v_{};\n", vn);
                    }
                    s += "    } u;\n";
                }
                Type::Maybe(i) => {
                    s += &format!("    bool has;\n    {} v;\n", self.ctype(i));
                }
                Type::Result(i) => {
                    s += "    bool ok;\n";
                    if **i != Type::Void {
                        s += &format!("    {} v;\n", self.ctype(i));
                    }
                    s += "    nx_str e;\n";
                }
                _ => unreachable!(),
            }
            s += "};\n";
        }
        s
    }

    fn order_visit(&mut self, t: &Type, seen: &mut HashSet<String>, order: &mut Vec<Type>) {
        let m = mangle(t);
        if !seen.insert(m) {
            return;
        }
        for d in self.deps(t) {
            self.order_visit(&d, seen, order);
        }
        if !builtin_maybe(t) {
            order.push(t.clone());
        }
    }

    // ---------- scopes and ownership ----------

    /// Where in the source, as one number for the runtime: the file's
    /// index times 1,000,000 plus the line (files are limited to 999,999 lines).
    fn loc(&self, span: crate::diag::Span) -> u32 {
        self.file_idx * 1_000_000 + if self.lines { span.line } else { 0 }
    }

    fn line(&mut self, s: impl AsRef<str>) {
        for _ in 0..self.indent {
            self.out += "    ";
        }
        self.out += s.as_ref();
        self.out.push('\n');
    }

    fn new_tmp(&mut self) -> String {
        self.tmp += 1;
        format!("nx_t{}", self.tmp)
    }

    /// Declare a temporary holding `value`. If `owned`, the block will
    /// release it.
    fn temp(&mut self, ty: &Type, value: String, owned: bool) -> CVal {
        let n = self.new_tmp();
        let ct = self.ctype(ty);
        self.line(format!("{} {} = {};", ct, n, value));
        let owned = owned && self.managed(ty);
        if owned {
            self.own(&n, ty);
        }
        CVal { code: n, owned }
    }

    fn own(&mut self, name: &str, ty: &Type) {
        if self.managed(ty) {
            self.scopes.last_mut().unwrap().owned.push((name.to_string(), ty.clone()));
        }
    }

    /// A new owned reference to the value.
    fn take(&mut self, v: CVal, ty: &Type) -> String {
        if !self.managed(ty) {
            return v.code;
        }
        if v.owned {
            for s in self.scopes.iter_mut().rev() {
                if let Some(i) = s.owned.iter().position(|(n, _)| *n == v.code) {
                    s.owned.remove(i);
                    return v.code;
                }
            }
            unreachable!("owned temporary {} not found", v.code);
        }
        self.retain_expr(ty, &v.code)
    }

    fn open(&mut self, is_loop: bool) {
        self.scopes.push(Scope { owned: Vec::new(), is_loop });
        self.indent += 1;
    }

    fn close(&mut self) {
        let top = self.scopes.len() - 1;
        self.release_from(top);
        self.scopes.pop();
        self.indent -= 1;
    }

    /// Release everything owned by scopes[from..], innermost first,
    /// without forgetting them (used before leaving early).
    fn release_from(&mut self, from: usize) {
        let mut lines = Vec::new();
        for i in (from..self.scopes.len()).rev() {
            for (n, t) in self.scopes[i].owned.clone().iter().rev() {
                lines.push(self.release_stmt(t, n));
            }
        }
        for l in lines {
            self.line(l);
        }
    }

    // ---------- functions and statements ----------

    fn signature(&mut self, f: &FnDecl, key: &str) -> String {
        let sig = self.c.fns[key].clone();
        let mut params = Vec::new();
        for (p, t) in f.params.iter().zip(&sig.params) {
            params.push(format!("{} {}", self.ctype(t), var(&p.name)));
        }
        format!(
            "{} {}({})",
            self.ctype(&sig.ret),
            fn_cname(key),
            if params.is_empty() { "void".to_string() } else { params.join(", ") }
        )
    }

    fn function(&mut self, f: &FnDecl, key: &str) {
        self.out.push('\n');
        let sig = self.signature(f, key);
        self.cur_ret = self.c.fns[key].ret.clone();
        self.line(format!("static {} {{", sig));
        self.scopes = Vec::new();
        self.open(false);
        for p in &f.params {
            self.line(format!("(void){};", var(&p.name)));
        }
        for s in &f.body {
            self.stmt(s);
        }
        self.close();
        if f.ret.is_some() {
            // The checker proved every path returns; this keeps C's
            // -Wreturn-type quiet and is a safety net.
            self.indent += 1;
            self.line(format!("nx_trap({}, \"function `{}` ended without returning a value\");", self.loc(f.end), key));
            self.indent -= 1;
        }
        self.line("}");
    }

    fn block(&mut self, b: &Block, is_loop: bool) {
        self.open(is_loop);
        for s in b {
            self.stmt(s);
        }
        self.close();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, value, .. } => {
                let ty = value.ty.clone();
                let v = self.expr(value);
                let code = self.take(v, &ty);
                let ct = self.ctype(&ty);
                self.line(format!("{} {} = {};", ct, var(name), code));
                self.line(format!("(void){};", var(name)));
                self.own(&var(name), &ty);
            }
            Stmt::Assign { target, op, value, span } => self.assign(target, *op, value, self.loc(*span)),
            Stmt::Expr(e) => {
                let v = self.expr(e);
                if !v.code.is_empty() {
                    self.line(format!("(void){};", v.code));
                }
            }
            Stmt::If { cond, then, els } => {
                let c = self.expr(cond);
                self.line(format!("if ({}) {{", c.code));
                self.block(then, false);
                if let Some(e) = els {
                    self.line("} else {");
                    self.block(e, false);
                }
                self.line("}");
            }
            Stmt::While { cond, body } => {
                self.line("for (;;) {");
                self.open(true);
                let c = self.expr(cond);
                self.line(format!("if (!({})) {{", c.code));
                self.indent += 1;
                let top = self.scopes.len() - 1;
                self.release_from(top);
                self.line("break;");
                self.indent -= 1;
                self.line("}");
                for st in body {
                    self.stmt(st);
                }
                self.close();
                self.line("}");
            }
            Stmt::ForRange { var: v, start, end, body, .. } => {
                let s = self.expr(start);
                let s = self.temp(&Type::Int, s.code, false);
                let e = self.expr(end);
                let e = self.temp(&Type::Int, e.code, false);
                let i = var(v);
                self.line(format!("for (int64_t {i} = {}; {i} < {}; {i}++) {{", s.code, e.code));
                self.open(true);
                self.line(format!("(void){};", i));
                for st in body {
                    self.stmt(st);
                }
                self.close();
                self.line("}");
            }
            Stmt::ForEach { var: v, iter, body, span } => {
                // Loop over a private snapshot of the list, so changing the
                // list inside the loop never affects the loop.
                self.line("{");
                self.open(false);
                let a = self.expr(iter);
                let snap_code = self.take(a, &iter.ty);
                let snap = self.temp(&iter.ty, snap_code, true);
                let elem = match &iter.ty {
                    Type::Array(t) => (**t).clone(),
                    _ => unreachable!(),
                };
                let i = self.new_tmp();
                self.line(format!("for (int64_t {i} = 0; {i} < nx_arr_len({}); {i}++) {{", snap.code));
                self.open(true);
                let ct = self.ctype(&elem);
                // borrowed from the snapshot, which outlives the loop body
                self.line(format!("{ct} {} = *({ct} *)nx_arr_at({}, {i}, {});", var(v), snap.code, self.loc(*span)));
                self.line(format!("(void){};", var(v)));
                for st in body {
                    self.stmt(st);
                }
                self.close();
                self.line("}");
                self.close();
                self.line("}");
            }
            Stmt::Return { value, .. } => match value {
                Some(v) => {
                    let ty = v.ty.clone();
                    let x = self.expr(v);
                    let code = self.take(x, &ty);
                    let r = self.temp(&ty, code, false);
                    self.release_from(0);
                    self.line(format!("return {};", r.code));
                }
                None => {
                    self.release_from(0);
                    self.line("return;");
                }
            },
            Stmt::Break(_) | Stmt::Continue(_) => {
                let lp = self.scopes.iter().rposition(|s| s.is_loop).expect("checker ensures a loop");
                self.release_from(lp);
                self.line(if matches!(s, Stmt::Break(_)) { "break;" } else { "continue;" });
            }
            Stmt::Match { value, arms, span } => self.match_stmt(value, arms, span.line),
        }
    }

    fn assign(&mut self, target: &Expr, op: AssignOp, value: &Expr, ln: u32) {
        let ty = target.ty.clone();
        let v = self.expr(value);
        let lv = self.lvalue(target);
        let ct = self.ctype(&ty);
        let slot = self.new_tmp();
        self.line(format!("{ct} *{slot} = &({lv});"));
        match (op, &ty) {
            (AssignOp::Set, _) => {
                if self.managed(&ty) {
                    let nv = self.take(v, &ty);
                    let old = self.new_tmp();
                    self.line(format!("{ct} {old} = *{slot};"));
                    self.line(format!("*{slot} = {nv};"));
                    let rel = self.release_stmt(&ty, &old);
                    self.line(rel);
                } else {
                    self.line(format!("*{slot} = {};", v.code));
                }
            }
            (AssignOp::Add, Type::Str) => {
                let nv = self.new_tmp();
                self.line(format!("nx_str {nv} = nx_str_cat(*{slot}, {});", v.code));
                self.line(format!("nx_str_release(*{slot});"));
                self.line(format!("*{slot} = {nv};"));
            }
            (op, Type::Int) => {
                let f = match op {
                    AssignOp::Add => "nx_add",
                    AssignOp::Sub => "nx_sub",
                    AssignOp::Mul => "nx_mul",
                    AssignOp::Div => "nx_div",
                    _ => "nx_rem",
                };
                self.line(format!("*{slot} = {f}(*{slot}, {}, {ln});", v.code));
            }
            (op, _) => {
                let o = match op {
                    AssignOp::Add => "+",
                    AssignOp::Sub => "-",
                    AssignOp::Mul => "*",
                    _ => "/",
                };
                self.line(format!("*{slot} = (*{slot} {o} {});", v.code));
            }
        }
    }

    fn match_stmt(&mut self, value: &Expr, arms: &[Arm], _ln: u32) {
        let ty = value.ty.clone();
        let v = self.expr(value);
        // own the value being matched, so the cases can't lose it even if
        // they change the variable it came from
        let code = self.take(v, &ty);
        let m = self.temp(&ty, code, true).code;
        let enum_def = match &ty {
            Type::Enum(n) => Some(self.c.enum_variants[n].clone()),
            _ => None,
        };
        for (k, arm) in arms.iter().enumerate() {
            let mut binds: Vec<(String, Type, String)> = Vec::new();
            let cond: Option<String> = match &arm.pattern {
                Pattern::Wildcard => None,
                Pattern::NoneLit => Some(format!("!{}.has", m)),
                Pattern::Int(v) => Some(if *v == i64::MIN { format!("{} == INT64_MIN", m) } else { format!("{} == INT64_C({})", m, v) }),
                Pattern::Str(v) => Some(format!("nx_str_eq({}, nx_str_lit({}, {}))", m, c_string_literal(v.as_bytes()), v.len())),
                Pattern::Case { name, binds: b, .. } => match (&ty, &enum_def) {
                    (Type::Result(inner), _) => {
                        let is_ok = name == "ok";
                        if let Some(b) = b {
                            let (bt, src) = if is_ok { ((**inner).clone(), format!("{}.v", m)) } else { (Type::Str, format!("{}.e", m)) };
                            binds.push((b[0].0.clone(), bt, src));
                        }
                        Some(format!("{}{}.ok", if is_ok { "" } else { "!" }, m))
                    }
                    (Type::Maybe(inner), _) => {
                        if let Some(b) = b {
                            binds.push((b[0].0.clone(), (**inner).clone(), format!("{}.v", m)));
                        }
                        Some(format!("{}.has", m))
                    }
                    (_, Some(def)) => {
                        let tag = def.iter().position(|(v, _)| v == name).expect("checked");
                        if let Some(b) = b {
                            for (i, (bn, _)) in b.iter().enumerate() {
                                binds.push((bn.clone(), def[tag].1[i].clone(), format!("{}.u.v_{}.a{}", m, name, i)));
                            }
                        }
                        Some(format!("{}.tag == {}", m, tag))
                    }
                    _ => unreachable!(),
                },
            };
            let head = match (k, cond) {
                (0, Some(c)) => format!("if ({}) {{", c),
                (0, None) => "{".to_string(),
                (_, Some(c)) => format!("}} else if ({}) {{", c),
                (_, None) => "} else {".to_string(),
            };
            self.line(head);
            self.open(false);
            for (bn, bt, src) in binds {
                if bn == "_" {
                    continue;
                }
                let code = self.retain_expr(&bt, &src);
                let ct = self.ctype(&bt);
                self.line(format!("{} {} = {};", ct, var(&bn), code));
                self.line(format!("(void){};", var(&bn)));
                self.own(&var(&bn), &bt);
            }
            for st in &arm.body {
                self.stmt(st);
            }
            self.close();
        }
        self.line("}");
    }

    /// A C lvalue for something being changed. Lists on the way are made
    /// private first (copy-on-write), so a change never shows through
    /// another copy.
    fn lvalue(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Var(n) => var(n),
            ExprKind::Field(b, f) => format!("{}.nx_f_{}", self.lvalue(b), f),
            ExprKind::Index(b, i) => {
                let base = self.lvalue(b);
                let i = self.expr(i);
                let ct = self.ctype(&e.ty);
                let ln = self.loc(e.span);
                format!("(*({ct} *)nx_arr_at(nx_arr_mut(&({base}), {ln}), {}, {ln}))", i.code)
            }
            _ => unreachable!("checker rejects other targets"),
        }
    }

    fn to_str(&mut self, t: &Type, v: CVal) -> CVal {
        match t {
            Type::Str => v,
            Type::Int => self.temp(&Type::Str, format!("nx_int_str({})", v.code), true),
            Type::Float => self.temp(&Type::Str, format!("nx_float_str({})", v.code), true),
            Type::Bool => CVal::plain(format!("nx_bool_str({})", v.code)),
            _ => unreachable!(),
        }
    }

    // ---------- expressions ----------

    /// Emits what the expression needs and returns a C expression with no
    /// side effects. `code` is "" for calls that return nothing.
    fn expr(&mut self, e: &Expr) -> CVal {
        let ln = self.loc(e.span);
        match &e.kind {
            ExprKind::Int(v) => CVal::plain(if *v == i64::MIN { "INT64_MIN".to_string() } else { format!("INT64_C({})", v) }),
            ExprKind::Float(v) => {
                let s = format!("{:?}", v);
                CVal::plain(if s.contains(['.', 'e']) { s } else { format!("{}.0", s) })
            }
            ExprKind::Bool(b) => CVal::plain(b.to_string()),
            ExprKind::Var(n) => CVal::plain(var(n)),
            ExprKind::Try(x) => {
                let v = self.expr(x);
                let is_result = matches!(x.ty, Type::Result(_));
                let flag = if is_result { "ok" } else { "has" };
                self.line(format!("if (!{}.{}) {{", v.code, flag));
                self.indent += 1;
                let ret_ty = self.cur_ret.clone();
                let rct = self.ctype(&ret_ty);
                let r = self.new_tmp();
                if is_result {
                    self.line(format!("{rct} {r} = {{ .ok = false, .e = nx_str_retain({}.e) }};", v.code));
                } else {
                    self.line(format!("{rct} {r} = {{ .has = false }};"));
                }
                self.release_from(0);
                self.line(format!("return {r};"));
                self.indent -= 1;
                self.line("}");
                if e.ty == Type::Void {
                    CVal::plain("")
                } else {
                    // borrowed from `v`, which this block still holds
                    self.temp(&e.ty, format!("{}.v", v.code), false)
                }
            }
            ExprKind::NoneLit => {
                let ct = self.ctype(&e.ty);
                let n = self.new_tmp();
                self.line(format!("{ct} {n} = {{ .has = false }};"));
                CVal::plain(n)
            }
            ExprKind::Str(pieces) => {
                let mut acc: Option<CVal> = None;
                for p in pieces {
                    let part = match p {
                        StrPiece::Lit(l) => CVal::plain(format!("nx_str_lit({}, {})", c_string_literal(l.as_bytes()), l.len())),
                        StrPiece::Expr(x) => {
                            let v = self.expr(x);
                            self.to_str(&x.ty, v)
                        }
                    };
                    acc = Some(match acc {
                        None => part,
                        Some(a) => self.temp(&Type::Str, format!("nx_str_cat({}, {})", a.code, part.code), true),
                    });
                }
                acc.unwrap_or_else(|| CVal::plain("nx_str_lit(\"\", 0)"))
            }
            ExprKind::Unary(op, x) => {
                let v = self.expr(x);
                let c = match (op, &x.ty) {
                    (UnOp::Not, _) => format!("(!{})", v.code),
                    (UnOp::Neg, Type::Int) => format!("nx_neg({}, {})", v.code, ln),
                    (UnOp::Neg, _) => format!("(-{})", v.code),
                };
                self.temp(&e.ty, c, false)
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
                let lv = self.expr(l);
                let t = self.temp(&Type::Bool, lv.code, false);
                self.line(format!("if ({}{}) {{", if *op == BinOp::Or { "!" } else { "" }, t.code));
                self.open(false);
                let rv = self.expr(r);
                self.line(format!("{} = {};", t.code, rv.code));
                self.close();
                self.line("}");
                t
            }
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l);
                let rv = self.expr(r);
                let (l, r) = (lv.code, rv.code);
                let lt = &e_ty_of(e);
                let c = match (op, lt) {
                    (BinOp::Add, Type::Str) => return self.temp(&Type::Str, format!("nx_str_cat({}, {})", l, r), true),
                    (BinOp::Eq, Type::Str) => format!("nx_str_eq({}, {})", l, r),
                    (BinOp::Ne, Type::Str) => format!("(!nx_str_eq({}, {}))", l, r),
                    (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, Type::Str) => {
                        format!("(nx_str_cmp({}, {}) {} 0)", l, r, op.symbol())
                    }
                    (BinOp::Eq | BinOp::Ne, Type::Enum(_)) => format!("({}.tag {} {}.tag)", l, op.symbol(), r),
                    (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, Type::Int) => {
                        let f = match op {
                            BinOp::Add => "nx_add",
                            BinOp::Sub => "nx_sub",
                            BinOp::Mul => "nx_mul",
                            BinOp::Div => "nx_div",
                            _ => "nx_rem",
                        };
                        format!("{}({}, {}, {})", f, l, r, ln)
                    }
                    _ => format!("({} {} {})", l, op.symbol(), r),
                };
                self.temp(&e.ty, c, false)
            }
            ExprKind::Field(b, f) => {
                let v = self.expr(b);
                // borrowed: `v` (owned by this block or a variable) outlives it
                self.temp(&e.ty, format!("{}.nx_f_{}", v.code, f), false)
            }
            ExprKind::Index(b, i) => {
                let a = self.expr(b);
                let i = self.expr(i);
                let ct = self.ctype(&e.ty);
                self.temp(&e.ty, format!("*({ct} *)nx_arr_at({}, {}, {ln})", a.code, i.code), false)
            }
            ExprKind::Array(items) => {
                let elem = match &e.ty {
                    Type::Array(t) => (**t).clone(),
                    _ => unreachable!(),
                };
                let vals: Vec<CVal> = items.iter().map(|x| self.expr(x)).collect();
                let codes: Vec<String> = vals.into_iter().map(|v| self.take(v, &elem)).collect();
                let ct = self.ctype(&elem);
                let (er, el) = self.elem_fns(&elem);
                let a = self.temp(&e.ty, format!("nx_arr_new((int64_t)sizeof({}), {}, {}, {})", ct, codes.len(), er, el), true);
                for c in codes {
                    let t = self.new_tmp();
                    self.line(format!("{ct} {t} = {c};"));
                    self.line(format!("nx_arr_push({}, &{}, {});", a.code, t, ln));
                }
                a
            }
            ExprKind::StructLit(name, fields) => {
                let def = self.c.struct_fields[name].clone();
                let vals: Vec<(String, CVal)> = fields.iter().map(|(f, x, _)| (f.clone(), self.expr(x))).collect();
                let mut inits = Vec::new();
                for (f, v) in vals {
                    let ft = def.iter().find(|(n, _)| *n == f).map(|(_, t)| t.clone()).expect("checked");
                    let c = self.take(v, &ft);
                    inits.push(format!(".nx_f_{} = {}", f, c));
                }
                let init = if inits.is_empty() { "{0}".to_string() } else { format!("{{ {} }}", inits.join(", ")) };
                self.temp(&e.ty, init, true)
            }
            ExprKind::Variant(en, vn, args) => {
                let def = self.c.enum_variants[en].clone();
                let tag = def.iter().position(|(v, _)| v == vn).expect("checked");
                let payload = def[tag].1.clone();
                let vals: Vec<CVal> = args.iter().flatten().map(|x| self.expr(x)).collect();
                let mut inits = Vec::new();
                for (i, (v, pt)) in vals.into_iter().zip(&payload).enumerate() {
                    let c = self.take(v, pt);
                    inits.push(format!(".a{} = {}", i, c));
                }
                let init = if inits.is_empty() {
                    format!("{{ .tag = {} }}", tag)
                } else {
                    format!("{{ .tag = {}, .u.v_{} = {{ {} }} }}", tag, vn, inits.join(", "))
                };
                self.temp(&e.ty, init, true)
            }
            ExprKind::Call(name, args) => self.call(name, args, e),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], e: &Expr) -> CVal {
        let ln = self.loc(e.span);
        match name {
            "push" => {
                let lv = self.lvalue(&args[0]);
                let v = self.expr(&args[1]);
                let ty = args[1].ty.clone();
                let code = self.take(v, &ty);
                let ct = self.ctype(&ty);
                let t = self.new_tmp();
                self.line(format!("{ct} {t} = {code};"));
                self.line(format!("nx_arr_push(nx_arr_mut(&({lv}), {ln}), &{t}, {ln});"));
                return CVal::plain("");
            }
            "pop" => {
                let lv = self.lvalue(&args[0]);
                let ct = self.ctype(&e.ty);
                let t = self.new_tmp();
                self.line(format!("{ct} {t} = {{0}};"));
                self.line(format!("nx_arr_pop(nx_arr_mut(&({lv}), {ln}), &{t}, {ln});"));
                self.own(&t, &e.ty);
                let owned = self.managed(&e.ty);
                return CVal { code: t, owned };
            }
            _ => {}
        }
        let vals: Vec<CVal> = args.iter().map(|a| self.expr(a)).collect();
        match name {
            "print" => {
                let s = self.to_str(&args[0].ty, vals[0].clone());
                self.line(format!("nx_print({});", s.code));
                CVal::plain("")
            }
            "to_str" => self.to_str(&args[0].ty, vals[0].clone()),
            "len" => {
                let c = if args[0].ty == Type::Str {
                    format!("{}.len", vals[0].code)
                } else {
                    format!("nx_arr_len({})", vals[0].code)
                };
                self.temp(&Type::Int, c, false)
            }
            "to_float" => self.temp(&Type::Float, format!("(double){}", vals[0].code), false),
            "to_int" => self.temp(&Type::Int, format!("nx_to_int({}, {})", vals[0].code, ln), false),
            "assert" => {
                self.line(format!("nx_assert({}, {});", vals[0].code, ln));
                CVal::plain("")
            }
            "some" => {
                let ty = args[0].ty.clone();
                let c = self.take(vals[0].clone(), &ty);
                self.temp(&e.ty, format!("{{ .has = true, .v = {} }}", c), true)
            }
            "unwrap_or" => {
                let (m, d) = (vals[0].code.clone(), vals[1].code.clone());
                let a = self.retain_expr(&e.ty, &format!("{}.v", m));
                let b = self.retain_expr(&e.ty, &d);
                let flag = if matches!(args[0].ty, Type::Result(_)) { "ok" } else { "has" };
                self.temp(&e.ty, format!("{}.{} ? {} : {}", m, flag, a, b), true)
            }
            "ok" => {
                if args.is_empty() {
                    self.temp(&e.ty, "{ .ok = true }".to_string(), true)
                } else {
                    let ty = args[0].ty.clone();
                    let c = self.take(vals[0].clone(), &ty);
                    self.temp(&e.ty, format!("{{ .ok = true, .v = {} }}", c), true)
                }
            }
            "err" => {
                let c = self.take(vals[0].clone(), &Type::Str);
                self.temp(&e.ty, format!("{{ .ok = false, .e = {} }}", c), true)
            }
            _ if fixed_builtin(name).is_some() => {
                let mut cargs: Vec<String> = vals.iter().map(|v| v.code.clone()).collect();
                cargs.push(ln.to_string());
                let call = format!("nx_b_{}({})", name, cargs.join(", "));
                if e.ty == Type::Void {
                    self.line(format!("{};", call));
                    CVal::plain("")
                } else {
                    self.temp(&e.ty, call, true)
                }
            }
            _ => {
                // a runtime error inside a standard module is reported
                // where the program called it
                if !self.cur_std && name.split_once('.').is_some_and(|(m, _)| self.std_mods.contains(m)) {
                    self.line(format!("nx_user_loc = {};", ln));
                }
                let call = format!("{}({})", fn_cname(name), vals.iter().map(|v| v.code.clone()).collect::<Vec<_>>().join(", "));
                if e.ty == Type::Void {
                    self.line(format!("{};", call));
                    CVal::plain("")
                } else {
                    // a function result is a new owned reference
                    self.temp(&e.ty, call, true)
                }
            }
        }
    }
}

/// Type of the left operand of a binary expression.
fn e_ty_of(e: &Expr) -> Type {
    match &e.kind {
        ExprKind::Binary(_, l, _) => l.ty.clone(),
        _ => e.ty.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c_strings() {
        assert_eq!(c_string_literal(b"hi"), "\"hi\"");
        assert_eq!(c_string_literal(b"a\"b\\?\n"), "\"a\\042b\\134\\077\\012\"");
        assert_eq!(c_string_literal("é".as_bytes()), "\"\\303\\251\"");
    }
    #[test]
    fn mangling_is_unambiguous() {
        let a = Type::Array(Box::new(Type::Maybe(Box::new(Type::Struct("Ab".into())))));
        assert_eq!(mangle(&a), "LMS2Ab");
        assert_ne!(mangle(&Type::Struct("S1".into())), mangle(&Type::Struct("1S".into())));
    }
}
