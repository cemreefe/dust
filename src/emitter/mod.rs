use crate::parser::ast::*;
use crate::semantic::binding_needs_mut;

pub fn emit(items: &[Item]) -> String {
    let mut out = String::new();
    for item in items {
        out.push_str(&emit_item(item));
        out.push('\n');
    }
    out
}

// ── Items ─────────────────────────────────────────────────────────────────────

fn emit_item(item: &Item) -> String {
    match item {
        Item::Fn { name, is_async, params, ret_ty, body, .. } => {
            emit_fn(name, *is_async, params, ret_ty.as_ref(), body, None)
        }
        Item::Struct { name, traits, fields, methods, .. } => {
            emit_struct(name, traits, fields, methods)
        }
        Item::Trait { name, methods, .. } => emit_trait(name, methods),
        Item::Enum { name, variants, .. } => emit_enum(name, variants),
        Item::Use { path, .. } => format!("use {};", path),
    }
}

fn emit_fn(
    name: &str,
    is_async: bool,
    params: &[Param],
    ret_ty: Option<&Ty>,
    body: &[Stmt],
    _trait_name: Option<&str>,
) -> String {
    let async_kw = if is_async { "async " } else { "" };
    let params_str = params.iter().map(emit_param).collect::<Vec<_>>().join(", ");
    let ret = ret_ty.map(|t| format!(" -> {}", emit_ty_owned(t))).unwrap_or_default();
    let body_str = emit_block(body);
    format!("{async_kw}fn {name}({params_str}){ret} {{\n{body_str}}}\n")
}

fn emit_struct(name: &str, traits: &[String], fields: &[Field], methods: &[Method]) -> String {
    let mut out = String::new();

    // struct definition
    out.push_str(&format!("struct {name} {{\n"));
    for f in fields {
        out.push_str(&format!("    {}: {},\n", f.name, emit_ty_owned(&f.ty)));
    }
    out.push_str("}\n");

    // Split methods: qualified (trait impls) vs unqualified (own methods)
    let own_methods: Vec<&Method> = methods.iter().filter(|m| m.trait_qualifier.is_none()).collect();
    let trait_methods: Vec<&Method> = methods.iter().filter(|m| m.trait_qualifier.is_some()).collect();

    // impl Struct { own methods }
    if !own_methods.is_empty() {
        out.push_str(&format!("\nimpl {name} {{\n"));
        for m in &own_methods {
            out.push_str(&indent_block(&emit_method(m)));
        }
        out.push_str("}\n");
    }

    // impl Trait for Struct { trait methods } — one block per trait
    for trait_name in traits {
        let tmethods: Vec<&Method> = trait_methods
            .iter()
            .filter(|m| m.trait_qualifier.as_deref() == Some(trait_name))
            .copied()
            .collect();

        out.push_str(&format!("\nimpl {trait_name} for {name} {{\n"));
        for m in tmethods {
            out.push_str(&indent_block(&emit_method(m)));
        }
        out.push_str("}\n");
    }

    out
}

fn emit_method(m: &Method) -> String {
    let async_kw = if m.is_async { "async " } else { "" };
    let params_str = m.params.iter().map(emit_param).collect::<Vec<_>>().join(", ");
    let ret = m.ret_ty.as_ref().map(|t| format!(" -> {}", emit_ty_owned(t))).unwrap_or_default();
    match &m.body {
        Some(body) => {
            let body_str = emit_block(body);
            format!("{async_kw}fn {}({params_str}){ret} {{\n{body_str}}}\n", m.name)
        }
        None => format!("{async_kw}fn {}({params_str}){ret};\n", m.name),
    }
}

fn emit_trait(name: &str, methods: &[Method]) -> String {
    let mut out = format!("trait {name} {{\n");
    for m in methods {
        out.push_str(&format!("    {}", emit_method(m)));
    }
    out.push_str("}\n");
    out
}

fn emit_enum(name: &str, variants: &[Variant]) -> String {
    let mut out = format!("enum {name} {{\n");
    for v in variants {
        if v.fields.is_empty() {
            out.push_str(&format!("    {},\n", v.name));
        } else {
            let fields = v.fields.iter().map(emit_ty_owned).collect::<Vec<_>>().join(", ");
            out.push_str(&format!("    {}({}),\n", v.name, fields));
        }
    }
    out.push_str("}\n");
    out
}

// ── Params & Types ────────────────────────────────────────────────────────────

fn emit_param(p: &Param) -> String {
    if p.name == "self" {
        return "self".into();
    }
    format!("{}: {}", p.name, emit_ty_owned(&p.ty))
}

/// Emit a type in owned position (e.g. return type, struct field, owned param).
/// `str` → `String`, `Ref(str)` → `&str`, etc.
fn emit_ty_owned(ty: &Ty) -> String {
    match ty {
        Ty::Simple(s) if s == "str" => "String".into(),
        Ty::Simple(s) => s.clone(),
        Ty::Generic(name, args) => {
            let inner = args.iter().map(emit_ty_owned).collect::<Vec<_>>().join(", ");
            format!("{name}<{inner}>")
        }
        Ty::Ref(inner) => format!("&{}", emit_ty_ref(inner)),
        Ty::SelfTy => "Self".into(),
    }
}

/// Emit a type in borrowed position (`str` → `str`, not `String`).
fn emit_ty_ref(ty: &Ty) -> String {
    match ty {
        Ty::Simple(s) if s == "str" => "str".into(),
        _ => emit_ty_owned(ty),
    }
}

// ── Statements ────────────────────────────────────────────────────────────────

fn emit_block(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    let last = stmts.len().saturating_sub(1);
    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i == last;
        let line = emit_stmt(stmt, is_last);
        for l in line.lines() {
            out.push_str("    ");
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

fn emit_stmt(stmt: &Stmt, is_last: bool) -> String {
    match stmt {
        Stmt::Let { name, ty, value, .. } => {
            let ty_ann = ty.as_ref().map(|t| format!(": {}", emit_ty_owned(t))).unwrap_or_default();
            let val = emit_expr_owned(value, ty.as_ref());
            format!("let {name}{ty_ann} = {val};")
        }
        Stmt::Const { name, ty, value, .. } => {
            let ty_ann = ty.as_ref().map(|t| format!(": {}", emit_ty_owned(t))).unwrap_or_default();
            let val = emit_expr_owned(value, ty.as_ref());
            format!("let {name}{ty_ann} = {val};")
        }
        Stmt::Mut { name, ty, value, .. } => {
            let ty_ann = ty.as_ref().map(|t| format!(": {}", emit_ty_owned(t))).unwrap_or_default();
            let val = emit_expr_owned(value, ty.as_ref());
            format!("let mut {name}{ty_ann} = {val};")
        }
        Stmt::Assign { target, value, .. } => {
            format!("{} = {};", emit_expr(target), emit_expr(value))
        }
        Stmt::Expr(e) => {
            let s = emit_expr(e);
            if is_last {
                // Last expression in a block — implicit return, no semicolon
                s
            } else {
                format!("{s};")
            }
        }
        Stmt::Return(Some(e), ..) => format!("return {};", emit_expr(e)),
        Stmt::Return(None, ..)    => "return;".into(),
        Stmt::TryCatch { try_block, catch_var, catch_block, .. } => {
            let try_stmts = emit_block(try_block);
            let catch_stmts = emit_block(catch_block);
            format!(
                "match (|| -> Result<_, _> {{\n{try_stmts}}})() {{\n    Ok(_) => {{}},\n    Err({catch_var}) => {{\n{catch_stmts}    }},\n}}"
            )
        }
        Stmt::Use { path, .. } => format!("use {path};"),
    }
}

// ── Expressions ───────────────────────────────────────────────────────────────

fn emit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Int(n)   => n.to_string(),
        Expr::Float(f) => {
            let s = format!("{f}");
            if s.contains('.') { s } else { format!("{s}.0") }
        }
        Expr::Bool(b)  => b.to_string(),

        Expr::Str(s) => {
            // If string contains {ident} interpolation, emit format!()
            // Otherwise emit a raw string literal — Rust coerces &str / String as needed
            if s.contains('{') {
                format!("format!(\"{s}\")")
            } else {
                format!("\"{s}\"")
            }
        }

        Expr::Ident { name, .. } => name.clone(),

        Expr::Macro { raw, .. } => raw.clone(),

        Expr::Path { segments, .. } => segments.join("::"),

        Expr::BinOp { op, left, right, .. } => {
            let op_str = match op {
                BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                BinOp::Div => "/", BinOp::Mod => "%",
                BinOp::Eq  => "==", BinOp::NotEq => "!=",
                BinOp::Lt  => "<",  BinOp::Gt    => ">",
                BinOp::LtEq => "<=", BinOp::GtEq => ">=",
                BinOp::And => "&&", BinOp::Or => "||",
                BinOp::Assign => "=",
            };
            format!("({} {} {})", emit_expr(left), op_str, emit_expr(right))
        }

        Expr::UnaryOp { op, expr, .. } => {
            let op_str = match op { UnaryOp::Neg => "-", UnaryOp::Not => "!" };
            format!("{op_str}{}", emit_expr(expr))
        }

        Expr::Call { func, args, .. } => {
            let f = emit_expr(func);
            let a = args.iter().map(emit_expr).collect::<Vec<_>>().join(", ");
            format!("{f}({a})")
        }

        Expr::FieldAccess { obj, field, .. } => {
            format!("{}.{field}", emit_expr(obj))
        }

        Expr::StructLit { name, fields, .. } => {
            let fs = fields.iter()
                .map(|(k, v)| format!("{k}: {}", emit_expr(v)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name} {{ {fs} }}")
        }

        Expr::If { cond, then_branch, else_branch, .. } => {
            let c = emit_expr(cond);
            let t = emit_expr_as_block(then_branch);
            match else_branch {
                None    => format!("if {c} {t}"),
                Some(e) => format!("if {c} {t} else {}", emit_expr_as_block(e)),
            }
        }

        Expr::Match { scrutinee, arms, .. } => {
            let s = emit_expr(scrutinee);
            let arms_str = arms.iter().map(|arm| {
                format!("    {} => {},", emit_expr(&arm.pattern), emit_expr(&arm.body))
            }).collect::<Vec<_>>().join("\n");
            format!("match {s} {{\n{arms_str}\n}}")
        }

        Expr::Closure { params, body, .. } => {
            let ps = params.iter().map(|p| {
                if p.ty == Ty::SelfTy { "self".into() }
                else { format!("{}: {}", p.name, emit_ty_owned(&p.ty)) }
            }).collect::<Vec<_>>().join(", ");
            format!("|{ps}| {}", emit_expr(body))
        }

        Expr::Block { stmts, .. } => {
            format!("{{\n{}}}", emit_block(stmts))
        }

        Expr::Return(Some(e), ..) => format!("return {}", emit_expr(e)),
        Expr::Return(None, ..)    => "return".into(),

        Expr::Try(e, ..)    => format!("{}?", emit_expr(e)),
        Expr::Unwrap(e, ..) => format!("{}.unwrap()", emit_expr(e)),
        Expr::Await(e, ..)  => format!("{}.await", emit_expr(e)),
    }
}

/// Like emit_expr but in an owned binding context: string literals get .to_string()
fn emit_expr_owned(expr: &Expr, ty: Option<&Ty>) -> String {
    // If expr is a bare string literal and we're in an owned context, add .to_string()
    let wants_string = match ty {
        Some(Ty::Simple(s)) if s == "str" => true,
        None => true, // infer owned for let bindings without annotation
        _ => false,
    };
    match expr {
        Expr::Str(s) if wants_string && !s.contains('{') => {
            format!("\"{s}\".to_string()")
        }
        _ => emit_expr(expr),
    }
}

fn emit_expr_as_block(expr: &Expr) -> String {
    match expr {
        Expr::Block { stmts, .. } => format!("{{\n{}}}", emit_block(stmts)),
        _ => format!("{{ {} }}", emit_expr(expr)),
    }
}

fn indent_block(s: &str) -> String {
    s.lines().map(|l| format!("    {l}\n")).collect()
}
