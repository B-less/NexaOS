//! Syntax tree. The checker fills in `Expr::ty`.

use crate::diag::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Void,
    Array(Box<Type>),
    Struct(String),
    Enum(String),
    /// `?T`: a value or nothing.
    Maybe(Box<Type>),
    /// `!T`: a value, or an error message.
    Result(Box<Type>),
    /// Not checked yet.
    Unknown,
    /// Already reported; matches anything so one mistake gives one error.
    Error,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Bool => write!(f, "bool"),
            Type::Str => write!(f, "str"),
            Type::Void => write!(f, "nothing"),
            Type::Array(t) => write!(f, "[{}]", t),
            Type::Struct(n) | Type::Enum(n) => write!(f, "{}", n),
            Type::Maybe(t) => write!(f, "?{}", t),
            Type::Result(t) => write!(f, "!{}", t),
            Type::Unknown | Type::Error => write!(f, "?"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypeExpr {
    Named(String, Span),
    Array(Box<TypeExpr>, Span),
    Maybe(Box<TypeExpr>, Span),
    Result(Box<TypeExpr>, Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(_, s) | TypeExpr::Array(_, s) | TypeExpr::Maybe(_, s) | TypeExpr::Result(_, s) => *s,
        }
    }
}

/// One source file: the main program (index 0), a file it uses, or a
/// standard module.
#[derive(Debug, Clone)]
pub struct Module {
    /// "" for the main program
    pub name: String,
    /// shown in error messages
    pub file: String,
    pub src: String,
    pub is_std: bool,
    pub prog: Program,
}

#[derive(Debug, Clone)]
pub struct Program {
    /// `use name` lines: module name and where.
    pub uses: Vec<(String, Span)>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub fns: Vec<FnDecl>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub payload: Vec<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub public: bool,
    pub name: String,
    pub variants: Vec<Variant>,
    pub span: Span,
}

/// One `pattern => { ... }` in a `match`.
#[derive(Debug, Clone)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    /// `_`
    Wildcard,
    /// `none`
    NoneLit,
    /// `3`, `-1`
    Int(i64),
    /// `"yes"`
    Str(String),
    /// `Name`, `Name(a, b)`, `Enum.Name(a)`, `some(x)`. `binds` is None
    /// when written without parentheses.
    Case { qual: Option<String>, name: String, binds: Option<Vec<(String, Span)>> },
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub public: bool,
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub public: bool,
    pub name: String,
    pub params: Vec<Field>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
    /// Line of the closing `}` (for the "ended without returning" trap).
    pub end: Span,
}

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: String, mutable: bool, ty: Option<TypeExpr>, value: Expr, span: Span },
    Assign { target: Expr, op: AssignOp, value: Expr, span: Span },
    Expr(Expr),
    If { cond: Expr, then: Block, els: Option<Block> },
    While { cond: Expr, body: Block },
    ForRange { var: String, start: Expr, end: Expr, body: Block, span: Span },
    ForEach { var: String, iter: Expr, body: Block, span: Span },
    Return { value: Option<Expr>, span: Span },
    Match { value: Expr, arms: Vec<Arm>, span: Span },
    Break(Span),
    Continue(Span),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub enum StrPiece {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Vec<StrPiece>),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    StructLit(String, Vec<(String, Expr, Span)>),
    /// `Enum.Variant` or `Enum.Variant(args)`; args None without parens.
    Variant(String, String, Option<Vec<Expr>>),
    /// `none`
    NoneLit,
    /// `value?`: the value inside a `?T` / `!T`, or leave the function
    /// with `none` / the error.
    Try(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Type,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span, ty: Type::Unknown }
    }
}
