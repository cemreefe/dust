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
        return if p.keep { "self".into() } else { "&mut self".into() };
    }
    if p.keep && p.mutable {
        return format!("mut {}: {}", p.name, emit_ty_owned(&p.ty));
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
        Stmt::CompoundAssign { target, op, value, .. } => {
            // &&= and ||= desugar to x = x op y (not valid Rust operators)
            if op == "&&" || op == "||" {
                let t = emit_expr(target);
                format!("{t} = {t} {op} {};", emit_expr(value))
            } else {
                format!("{} {}= {};", emit_expr(target), op, emit_expr(value))
            }
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
        Stmt::For { var, iter, body, .. } => {
            let iter_s = emit_expr(iter);
            let body_s = emit_block(body);
            format!("for {var} in {iter_s} {{\n{body_s}}}")
        }
        Stmt::Break(..)    => "break".into(),
        Stmt::Continue(..) => "continue".into(),
        Stmt::Use { path, .. } => format!("use {path};"),
    }
}

// ── Expressions ───────────────────────────────────────────────────────────────

fn emit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Char(c)  => format!("'{}'", c.escape_default()),
        Expr::Int(n)   => n.to_string(),
        Expr::Float(f) => {
            let s = format!("{f}");
            if s.contains('.') { s } else { format!("{s}.0") }
        }
        Expr::Bool(b)  => b.to_string(),

        Expr::Str(s) => {
            // Check if the string has any {ident} interpolation
            if has_interpolation(s) {
                let fmt = prepare_format_str(s);
                format!("format!(\"{fmt}\")")
            } else {
                format!("\"{}\"", escape_str(s))
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
            match op {
                UnaryOp::Neg    => format!("-{}", emit_expr(expr)),
                UnaryOp::Not    => format!("!{}", emit_expr(expr)),
                UnaryOp::Ref    => format!("&{}", emit_expr(expr)),
                UnaryOp::RefMut => format!("&mut {}", emit_expr(expr)),
            }
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

        Expr::Turbofish { inner, type_args, .. } => {
            format!("{}::<{}>", emit_expr(inner), type_args)
        }

        Expr::Index { obj, idx, .. } => {
            format!("{}[{}]", emit_expr(obj), emit_expr(idx))
        }
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
            format!("\"{}\".to_string()", escape_str(s))
        }
        _ => emit_expr(expr),
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"'  => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c    => out.push(c),
        }
    }
    out
}

/// True if the string contains at least one `{ident}` format placeholder
fn has_interpolation(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut ident = String::new();
            let mut closed = false;
            for c2 in chars.by_ref() {
                if c2 == '}' { closed = true; break; }
                if c2.is_alphanumeric() || c2 == '_' { ident.push(c2); }
                else { break; }
            }
            if closed && !ident.is_empty() { return true; }
        }
    }
    false
}

/// Prepare a format string: escape non-placeholder `{` as `{{` and `}` as `}}`,
/// while leaving `{ident}` placeholders untouched. Also escapes quotes etc.
fn prepare_format_str(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' => {
                // Try to scan {ident}
                let mut j = i + 1;
                let mut ident = String::new();
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    ident.push(chars[j]);
                    j += 1;
                }
                if !ident.is_empty() && j < chars.len() && chars[j] == '}' {
                    // It's a placeholder — emit as-is
                    out.push('{');
                    out.push_str(&ident);
                    out.push('}');
                    i = j + 1;
                } else {
                    out.push_str("{{");
                    i += 1;
                }
            }
            '}' => { out.push_str("}}"); i += 1; }
            '\\' => { out.push_str("\\\\"); i += 1; }
            '"'  => { out.push_str("\\\""); i += 1; }
            '\n' => { out.push_str("\\n");  i += 1; }
            '\r' => { out.push_str("\\r");  i += 1; }
            '\t' => { out.push_str("\\t");  i += 1; }
            c    => { out.push(c); i += 1; }
        }
    }
    out
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
