/// A type expression, e.g. `i32`, `Vec<T>`, `Option<str>`, `str`
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Simple(String),           // i32, bool, str, etc.
    Generic(String, Vec<Ty>), // Vec<T>, Option<i32>
    Tuple(Vec<Ty>),           // (A, B, C)
    Ref(Box<Ty>),             // &T  (used internally by semantic pass)
    SelfTy,
}

impl Ty {
    pub fn is_primitive(&self) -> bool {
        matches!(self, Ty::Simple(s) if matches!(
            s.as_str(),
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "f32" | "f64" | "bool" | "char"
        ))
    }
}

/// A function parameter
#[derive(Debug, Clone)]
pub struct Param {
    pub keep: bool,        // `keep name: T` → take ownership
    pub mutable: bool,     // `keep mut name: T` → mutable owned binding
    pub name: String,
    pub ty: Ty,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Expr,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),    // raw string literal (may contain {name} interpolation)
    Char(char),
    ByteChar(char),
    Bool(bool),
    Tuple(Vec<Expr>),
    Ident { name: String, line: usize, col: usize },

    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        line: usize,
        col: usize,
    },
    UnaryOp { op: UnaryOp, expr: Box<Expr>, line: usize, col: usize },

    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
        line: usize,
        col: usize,
    },
    FieldAccess {
        obj: Box<Expr>,
        field: String,
        line: usize,
        col: usize,
    },
    // foo::bar::baz  — module-qualified path
    Path {
        segments: Vec<String>,
        line: usize,
        col: usize,
    },
    // Struct literal: Foo { x: 1, y: 2 }
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
        line: usize,
        col: usize,
    },

    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
        line: usize,
        col: usize,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
        line: usize,
        col: usize,
    },

    // x: i32, y: i32 -> x + y   or   move || expr
    Closure {
        params: Vec<Param>,
        body: Box<Expr>,
        is_move: bool,
        line: usize,
        col: usize,
    },

    // if let PATTERN = VALUE { ... } [else { ... }]
    IfLet {
        pattern: Box<Expr>,
        value: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
        line: usize,
        col: usize,
    },

    Block {
        stmts: Vec<Stmt>,
        line: usize,
        col: usize,
    },

    Return(Option<Box<Expr>>, usize, usize),

    // expr?
    Try(Box<Expr>, usize, usize),
    // expr.unwrap!  — stored as a special node since .unwrap! isn't valid method syntax
    Unwrap(Box<Expr>, usize, usize),

    // await expr
    Await(Box<Expr>, usize, usize),

    // Inline macro passthrough: the entire "println!(...)" string
    Macro { raw: String, line: usize, col: usize },

    // expr::<Type, ...> — turbofish
    Turbofish { inner: Box<Expr>, type_args: String, line: usize, col: usize },

    // expr[idx]
    Index { obj: Box<Expr>, idx: Box<Expr>, line: usize, col: usize },

    // expr as Type
    Cast { expr: Box<Expr>, ty: Ty, line: usize, col: usize },

    // vec! Namespace[Circle(3.0), Rect(4.0, 5.0)] — namespaced vec literal
    NamespacedVec { ns: String, items: Vec<Expr> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod, Pow,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    And, Or,
    Assign,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    Ref,    // &expr
    RefMut, // &mut expr
    Deref,  // *expr
}

#[derive(Debug, Clone)]
pub enum Stmt {
    // let / const / mut  name [: ty] = expr
    Let   { name: String, ty: Option<Ty>, value: Expr, line: usize, col: usize },
    Const { name: String, ty: Option<Ty>, value: Expr, line: usize, col: usize },
    Mut   { name: String, ty: Option<Ty>, value: Expr, line: usize, col: usize },

    // name = expr  (reassignment, not declaration)
    Assign { target: Expr, value: Expr, line: usize, col: usize },

    // name += expr / name++ / name--
    CompoundAssign { target: Expr, op: String, value: Expr, line: usize, col: usize },

    Expr(Expr),
    Return(Option<Expr>, usize, usize),

    TryCatch {
        try_block: Vec<Stmt>,
        catch_var: String,
        catch_block: Vec<Stmt>,
        line: usize,
        col: usize,
    },

    For {
        vars: Vec<String>,   // ["x"] or ["a", "b"] for tuple destructuring
        iter: Expr,
        body: Vec<Stmt>,
        line: usize,
        col: usize,
    },

    While    { cond: Expr,               body: Vec<Stmt>, line: usize, col: usize },
    WhileLet { pattern: Expr, value: Expr, body: Vec<Stmt>, line: usize, col: usize },

    Break(usize, usize),
    Continue(usize, usize),

    // use path::to::thing;  — pass through
    Use { path: String, line: usize, col: usize },
}

/// A field in a struct definition
#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub attrs: Vec<String>,
    pub line: usize,
    pub col: usize,
}

/// A method definition inside a struct or trait
#[derive(Debug, Clone)]
pub struct Method {
    /// None = own method, Some("Trait") = trait impl method
    pub trait_qualifier: Option<String>,
    pub name: String,
    pub generics: String,
    pub is_async: bool,
    pub params: Vec<Param>,
    pub ret_ty: Option<Ty>,
    pub body: Option<Vec<Stmt>>, // None = trait declaration (no body)
    pub line: usize,
    pub col: usize,
}

/// A variant in an enum
#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Ty>,  // tuple-style fields
    pub line: usize,
    pub col: usize,
}

/// Top-level items
#[derive(Debug, Clone)]
pub enum Item {
    Const { name: String, value: Expr, line: usize, col: usize },
    Fn {
        name: String,
        generics: String,
        is_async: bool,
        params: Vec<Param>,
        ret_ty: Option<Ty>,
        body: Vec<Stmt>,
        attrs: Vec<String>,
        line: usize,
        col: usize,
    },
    Struct {
        name: String,
        generics: String,
        traits: Vec<String>,
        fields: Vec<Field>,
        methods: Vec<Method>,
        assoc_types: Vec<(String, Ty)>, // type Name = Ty  inside trait impls
        attrs: Vec<String>,
        line: usize,
        col: usize,
    },
    Trait {
        name: String,
        generics: String,
        methods: Vec<Method>,
        line: usize,
        col: usize,
    },
    Enum {
        name: String,
        traits: Vec<String>,
        variants: Vec<Variant>,
        attrs: Vec<String>,
        line: usize,
        col: usize,
    },
    Use { path: String, line: usize, col: usize },
}
