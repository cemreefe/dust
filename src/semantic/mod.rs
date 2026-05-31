/// Semantic analysis pass.
///
/// Responsibilities:
///  1. Track `const` bindings and error if mutated.
///  2. Promote `let` bindings to `mut` when assigned-to later.
///  3. Auto-borrow function params (str → &str) unless `keep`.
///  4. Insert `.clone()` on `let x = y` when y is non-primitive heap type.
///
/// The AST is modified in-place (Stmt / Expr variants are replaced).

use crate::error::{DustError, Result};
use crate::parser::ast::*;
use std::collections::{HashMap, HashSet};

pub fn analyze(mut items: Vec<Item>) -> Result<Vec<Item>> {
    for item in &mut items {
        analyze_item(item)?;
    }
    Ok(items)
}

fn analyze_item(item: &mut Item) -> Result<()> {
    match item {
        Item::Fn { params, ret_ty, body, .. } => {
            apply_auto_borrow_params(params, ret_ty);
            let mut ctx = Ctx::new();
            analyze_block(body, &mut ctx)?;
        }
        Item::Struct { methods, .. } => {
            for m in methods {
                if let Some(body) = &mut m.body {
                    apply_auto_borrow_params(&mut m.params, &mut m.ret_ty);
                    let mut ctx = Ctx::new();
                    analyze_block(body, &mut ctx)?;
                }
            }
        }
        Item::Trait { .. } | Item::Enum { .. } | Item::Use { .. } => {}
    }
    Ok(())
}

/// Convert `str` params to `&str` (Ref) unless `keep`.
fn apply_auto_borrow_params(params: &mut Vec<Param>, _ret_ty: &mut Option<Ty>) {
    for p in params.iter_mut() {
        if p.keep { continue; }
        if p.name == "self" { continue; }
        p.ty = borrow_ty(&p.ty);
    }
    // Return types stay owned (String not &str) — function gives ownership back.
}

const PRIMITIVES: &[&str] = &[
    "i8","i16","i32","i64","i128","isize",
    "u8","u16","u32","u64","u128","usize",
    "f32","f64","bool","char",
];

fn borrow_ty(ty: &Ty) -> Ty {
    match ty {
        Ty::Simple(s) if s == "str" => Ty::Ref(Box::new(Ty::Simple("str".into()))),
        Ty::Generic(..) => Ty::Ref(Box::new(ty.clone())),
        Ty::Simple(s) if !PRIMITIVES.contains(&s.as_str()) => Ty::Ref(Box::new(ty.clone())),
        _ => ty.clone(),
    }
}

/// Context tracking variable binding kinds within a block.
#[derive(Default)]
struct Ctx {
    // name → (kind, is_assigned_after_binding)
    bindings: HashMap<String, BindKind>,
    // Set of names assigned after initial binding (promotes let → mut)
    assigned: HashSet<String>,
    consts: HashSet<String>,
}

#[derive(Clone, PartialEq)]
enum BindKind { Let, Const, Mut }

impl Ctx {
    fn new() -> Self { Self::default() }

    fn child(&self) -> Self {
        // Child scope inherits parent's view
        Self {
            bindings: self.bindings.clone(),
            assigned: self.assigned.clone(),
            consts: self.consts.clone(),
        }
    }

    fn declare(&mut self, name: &str, kind: BindKind) {
        self.bindings.insert(name.to_string(), kind.clone());
        if kind == BindKind::Const { self.consts.insert(name.to_string()); }
    }

    fn mark_assigned(&mut self, name: &str, line: usize, col: usize) -> Result<()> {
        if self.consts.contains(name) {
            return Err(DustError::new(
                format!("cannot assign to `const` binding '{name}'"),
                line, col,
            ));
        }
        self.assigned.insert(name.to_string());
        Ok(())
    }

    fn needs_mut(&self, name: &str) -> bool {
        self.assigned.contains(name)
            && self.bindings.get(name) == Some(&BindKind::Let)
    }
}

fn analyze_block(stmts: &mut Vec<Stmt>, ctx: &mut Ctx) -> Result<()> {
    // First pass: collect all assignments in this block to know which `let`s need mut
    collect_assignments(stmts, ctx)?;

    // Second pass: process each statement
    for stmt in stmts.iter_mut() {
        analyze_stmt(stmt, ctx)?;
    }
    Ok(())
}

fn collect_assignments(stmts: &[Stmt], ctx: &mut Ctx) -> Result<()> {
    for stmt in stmts {
        match stmt {
            Stmt::Let { name, .. }   => ctx.declare(name, BindKind::Let),
            Stmt::Const { name, .. } => ctx.declare(name, BindKind::Const),
            Stmt::Mut { name, .. }   => ctx.declare(name, BindKind::Mut),
            Stmt::Assign { target: Expr::Ident { name, line, col }, .. }
            | Stmt::CompoundAssign { target: Expr::Ident { name, line, col }, .. } => {
                ctx.mark_assigned(name, *line, *col)?;
            }
            Stmt::For { vars, .. } => { for v in vars { ctx.declare(v, BindKind::Let); } }
            _ => {}
        }
    }
    Ok(())
}

fn analyze_stmt(stmt: &mut Stmt, ctx: &mut Ctx) -> Result<()> {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Mut { value, .. } => {
            analyze_expr(value, ctx)?;
            // Auto-clone disabled: too aggressive for non-Clone types (e.g. TcpStream).
            // Users clone explicitly where needed.
        }
        Stmt::Const { value, .. } => {
            analyze_expr(value, ctx)?;
        }
        Stmt::Assign { target, value, line, col }
        | Stmt::CompoundAssign { target, value, line, col, .. } => {
            if let Expr::Ident { name, .. } = target {
                ctx.mark_assigned(name, *line, *col)?;
            }
            analyze_expr(value, ctx)?;
        }
        Stmt::Expr(e) => { analyze_expr(e, ctx)?; }
        Stmt::Return(Some(e), ..) => { analyze_expr(e, ctx)?; }
        Stmt::Return(None, ..) => {}
        Stmt::TryCatch { try_block, catch_block, .. } => {
            let mut child = ctx.child();
            analyze_block(try_block, &mut child)?;
            let mut child2 = ctx.child();
            analyze_block(catch_block, &mut child2)?;
        }
        Stmt::For { iter, body, .. } => {
            analyze_expr(iter, ctx)?;
            let mut child = ctx.child();
            analyze_block(body, &mut child)?;
        }
        Stmt::Break(..) | Stmt::Continue(..) => {}
        Stmt::Use { .. } => {}
    }
    Ok(())
}

fn analyze_expr(expr: &mut Expr, ctx: &mut Ctx) -> Result<()> {
    match expr {
        Expr::BinOp { left, right, .. } => {
            analyze_expr(left, ctx)?;
            analyze_expr(right, ctx)?;
        }
        Expr::UnaryOp { expr: inner, .. } => analyze_expr(inner, ctx)?,
        Expr::Call { func, args, .. } => {
            analyze_expr(func, ctx)?;
            for a in args { analyze_expr(a, ctx)?; }
        }
        Expr::FieldAccess { obj, .. } => analyze_expr(obj, ctx)?,
        Expr::If { cond, then_branch, else_branch, .. } => {
            analyze_expr(cond, ctx)?;
            analyze_expr(then_branch, ctx)?;
            if let Some(e) = else_branch { analyze_expr(e, ctx)?; }
        }
        Expr::Match { scrutinee, arms, .. } => {
            analyze_expr(scrutinee, ctx)?;
            for arm in arms { analyze_expr(&mut arm.body, ctx)?; }
        }
        Expr::Block { stmts, .. } => {
            let mut child = ctx.child();
            analyze_block(stmts, &mut child)?;
        }
        Expr::Try(inner, ..) | Expr::Unwrap(inner, ..) | Expr::Await(inner, ..) => {
            analyze_expr(inner, ctx)?;
        }
        Expr::Closure { body, .. } => analyze_expr(body, ctx)?,
        Expr::Turbofish { inner, .. } => analyze_expr(inner, ctx)?,
        Expr::Index { obj, idx, .. } => {
            analyze_expr(obj, ctx)?;
            analyze_expr(idx, ctx)?;
        }
        _ => {}
    }
    Ok(())
}

/// If the expression is a simple identifier binding to a non-primitive,
/// wrap it in a synthetic clone call.
fn maybe_insert_clone(expr: &mut Expr) {
    match expr {
        // Already a call or literal — don't clone
        Expr::Str(_) | Expr::Int(_) | Expr::Float(_) | Expr::Bool(_)
        | Expr::Call { .. } | Expr::Macro { .. } => {}

        Expr::Ident { name, line, col } => {
            // We don't know types at this stage — insert clone for any ident
            // (primitives will still clone cheaply; semantic improvement possible later)
            let original = Expr::Ident { name: name.clone(), line: *line, col: *col };
            *expr = Expr::Call {
                func: Box::new(Expr::FieldAccess {
                    obj: Box::new(original),
                    field: "clone".into(),
                    line: *line,
                    col: *col,
                }),
                args: vec![],
                line: *line,
                col: *col,
            };
        }
        _ => {}
    }
}

/// After analysis, check whether a `let` binding should become `mut`.
/// This is called by the emitter to decide which keyword to emit.
pub fn binding_needs_mut(name: &str, stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| {
        matches!(s, Stmt::Assign { target: Expr::Ident { name: n, .. }, .. } if n == name)
    })
}
