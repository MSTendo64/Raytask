//! Abstract Syntax Tree for RayTask.

use crate::span::Span;

#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Import(ImportDecl),
    Namespace(NamespaceDecl),
    Module(ModuleDecl),
    Class(ClassDecl),
    Struct(StructDecl),
    Interface(InterfaceDecl),
    Function(FunctionDecl),
    Const(ConstDecl),
    Attribute(Attribute, Box<Item>),
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub path: String,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct NamespaceDecl {
    pub name: String,
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ModuleDecl {
    pub name: String,
    pub fields: Vec<(String, Expr)>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Default,
    Export,
    Protected,
    Private,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub access: Access,
    pub is_abstract: bool,
    pub name: String,
    pub type_params: Vec<String>,
    pub bases: Vec<TypeRef>,
    pub constraints: Vec<GenericConstraint>,
    pub members: Vec<Member>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub access: Access,
    pub name: String,
    pub type_params: Vec<String>,
    pub members: Vec<Member>,
    pub attributes: Vec<Attribute>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub access: Access,
    pub name: String,
    pub type_params: Vec<String>,
    pub members: Vec<Member>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct GenericConstraint {
    pub type_param: String,
    pub bounds: Vec<TypeRef>,
}

#[derive(Debug, Clone)]
pub enum Member {
    Field(FieldDecl),
    Method(FunctionDecl),
    Constructor(ConstructorDecl),
    Destructor(DestructorDecl),
    Property(PropertyDecl),
    Indexer(IndexerDecl),
    Operator(OperatorDecl),
}

#[derive(Debug, Clone)]
pub struct FieldDecl {
    pub access: Access,
    pub is_const: bool,
    pub ty: Option<TypeRef>,
    pub name: String,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstructorDecl {
    pub params: Vec<Param>,
    pub base_args: Vec<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct DestructorDecl {
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PropertyDecl {
    pub access: Access,
    pub name: String,
    pub ty: TypeRef,
    pub getter: Option<Block>,
    pub setter: Option<Block>,
    pub auto: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IndexerDecl {
    pub ty: TypeRef,
    pub params: Vec<Param>,
    pub getter: Option<Block>,
    pub setter: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct OperatorDecl {
    pub op: String,
    pub params: Vec<Param>,
    pub return_type: TypeRef,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub access: Access,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_virtual: bool,
    pub is_override: bool,
    pub is_abstract: bool,
    pub is_extension: bool,
    pub return_type: TypeRef,
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub constraints: Vec<GenericConstraint>,
    pub body: Option<FunctionBody>,
    pub attributes: Vec<Attribute>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum FunctionBody {
    Block(Block),
    Expr(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub is_params: bool,
    pub is_this: bool,
    pub name: String,
    pub ty: TypeRef,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub ty: TypeRef,
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeRef {
    pub name: String,
    pub args: Vec<TypeRef>,
    pub nullable: bool,
    pub is_array: bool,
    pub array_dims: usize,
    pub span: Span,
}

impl TypeRef {
    pub fn named(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            args: vec![],
            nullable: false,
            is_array: false,
            array_dims: 0,
            span,
        }
    }

    pub fn void(span: Span) -> Self {
        Self::named("void", span)
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Decl(VarDecl),
    Const(ConstDecl),
    Return(Option<Expr>, Span),
    If {
        cond: Expr,
        then_block: Block,
        else_branch: Option<ElseBranch>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    DoWhile {
        body: Block,
        cond: Expr,
        span: Span,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Block,
        span: Span,
    },
    Foreach {
        var_name: String,
        index_name: Option<String>,
        iter: Expr,
        body: Block,
        span: Span,
    },
    Switch {
        expr: Expr,
        cases: Vec<SwitchCase>,
        span: Span,
    },
    Match {
        expr: Expr,
        arms: Vec<MatchArm>,
        span: Span,
    },
    Try {
        body: Block,
        catches: Vec<CatchClause>,
        finally: Option<Block>,
        span: Span,
    },
    Throw(Expr, Span),
    Break(Span),
    Continue(Span),
    Using {
        decl: VarDecl,
        body: Block,
        span: Span,
    },
    Unsafe(Block, Span),
    Block(Block),
}

#[derive(Debug, Clone)]
pub enum ElseBranch {
    Block(Block),
    If(Box<Stmt>),
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    /// None = default arm.
    /// One or more patterns separated by `|`.
    pub patterns: Vec<SwitchPattern>,
    pub pattern_bind: Option<String>,
    /// Optional guard: `when <expr>`
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
}

/// A single pattern inside a `case` arm.
#[derive(Debug, Clone)]
pub enum SwitchPattern {
    /// Literal / expression equality: `case 42:`
    Expr(Expr),
    /// Inclusive range: `case 1..10:`
    Range(Expr, Expr),
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: String,
    pub bind: Option<String>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub struct CatchClause {
    pub exception_type: Option<TypeRef>,
    pub name: Option<String>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub kind: VarKind,
    pub ty: Option<TypeRef>,
    pub name: String,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Typed,
    Dyn,
    Stack,
    Owned,
    Const,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64, Span),
    UInt(u64, Span),
    Float(f64, Span),
    Decimal(String, Span),
    Bool(bool, Span),
    Char(char, Span),
    String(String, Span),
    Interpolated(Vec<InterpPart>, Span),
    Null(Span),
    Ident(String, Span),
    This(Span),
    Base(Span),
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
        span: Span,
    },
    Assign {
        target: Box<Expr>,
        op: AssignOp,
        value: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        type_args: Vec<TypeRef>,
        args: Vec<Arg>,
        span: Span,
    },
    Index {
        object: Box<Expr>,
        indices: Vec<Expr>,
        span: Span,
    },
    Member {
        object: Box<Expr>,
        field: String,
        null_safe: bool,
        span: Span,
    },
    New {
        ty: TypeRef,
        args: Vec<Arg>,
        init: Vec<(String, Expr)>,
        span: Span,
    },
    ArrayLit(Vec<Expr>, Span),
    Lambda {
        params: Vec<Param>,
        body: FunctionBody,
        span: Span,
    },
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
        span: Span,
    },
    Cast {
        ty: TypeRef,
        expr: Box<Expr>,
        span: Span,
    },
    TypeOf(TypeRef, Span),
    Is {
        expr: Box<Expr>,
        ty: TypeRef,
        span: Span,
    },
    As {
        expr: Box<Expr>,
        ty: TypeRef,
        span: Span,
    },
    Await(Box<Expr>, Span),
    Deref(Box<Expr>, Span),
    AddressOf(Box<Expr>, Span),
    PtrMember {
        object: Box<Expr>,
        field: String,
        span: Span,
    },
    Grouped(Box<Expr>, Span),
    /// Error-propagate operator `?`
    Try(Box<Expr>, Span),
}

#[derive(Debug, Clone)]
pub enum InterpPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    NullCoalesce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    NullCoalesce,
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, s)
            | Expr::UInt(_, s)
            | Expr::Float(_, s)
            | Expr::Decimal(_, s)
            | Expr::Bool(_, s)
            | Expr::Char(_, s)
            | Expr::String(_, s)
            | Expr::Interpolated(_, s)
            | Expr::Null(s)
            | Expr::Ident(_, s)
            | Expr::This(s)
            | Expr::Base(s)
            | Expr::Binary { span: s, .. }
            | Expr::Unary { span: s, .. }
            | Expr::Assign { span: s, .. }
            | Expr::Call { span: s, .. }
            | Expr::Index { span: s, .. }
            | Expr::Member { span: s, .. }
            | Expr::New { span: s, .. }
            | Expr::ArrayLit(_, s)
            | Expr::Lambda { span: s, .. }
            | Expr::Ternary { span: s, .. }
            | Expr::Cast { span: s, .. }
            | Expr::TypeOf(_, s)
            | Expr::Is { span: s, .. }
            | Expr::As { span: s, .. }
            | Expr::Await(_, s)
            | Expr::Deref(_, s)
            | Expr::AddressOf(_, s)
            | Expr::PtrMember { span: s, .. }
            | Expr::Grouped(_, s)
            | Expr::Try(_, s) => *s,
        }
    }
}
