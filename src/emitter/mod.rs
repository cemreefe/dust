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

        Expr::Str(s) => emit_str(s),

        Expr::Ident { name, .. } => name.clone(),

        Expr::Macro { raw, .. } => process_macro_str(raw),

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

/// Like emit_expr but in an owned binding context: plain string literals get .to_string()
fn emit_expr_owned(expr: &Expr, ty: Option<&Ty>) -> String {
    let wants_string = match ty {
        Some(Ty::Simple(s)) if s == "str" => true,
        None => true,
        _ => false,
    };
    match expr {
        Expr::Str(s) if wants_string => {
            let (fmt, args) = extract_str_args(s);
            if args.is_empty() {
                format!("\"{fmt}\".to_string()")
            } else {
                format!("format!(\"{fmt}\", {})", args.join(", "))
            }
        }
        _ => emit_expr(expr),
    }
}

/// Scan a string for `{expr}` interpolations.
/// Returns (format_str_with_positional_placeholders, vec_of_expr_strings).
/// Handles nested braces, escapes special chars for Rust string literals.
fn extract_str_args(s: &str) -> (String, Vec<String>) {
    let mut fmt = String::new();
    let mut args = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                // Only treat as interpolation if next char looks like an expression start
                let next = chars.get(i + 1).copied().unwrap_or('\0');
                let is_interp = next.is_alphabetic() || next == '_'
                    || next.is_ascii_digit()
                    || next == '-' || next == '!' || next == '(';
                if !is_interp {
                    fmt.push_str("{{");
                    i += 1;
                } else {
                    // Scan for matching } tracking depth
                    let start = i + 1;
                    let mut depth = 1usize;
                    let mut j = start;
                    while j < chars.len() {
                        match chars[j] {
                            '{' => depth += 1,
                            '}' => { depth -= 1; if depth == 0 { break; } }
                            _ => {}
                        }
                        j += 1;
                    }
                    if depth == 0 {
                        let expr: String = chars[start..j].iter().collect();
                        fmt.push_str("{}");
                        args.push(expr);
                        i = j + 1;
                    } else {
                        fmt.push_str("{{");
                        i += 1;
                    }
                }
            }
            '}' => { fmt.push_str("}}"); i += 1; }
            '\\' => { fmt.push_str("\\\\"); i += 1; }
            '"'  => { fmt.push_str("\\\""); i += 1; }
            '\n' => { fmt.push_str("\\n");  i += 1; }
            '\r' => { fmt.push_str("\\r");  i += 1; }
            '\t' => { fmt.push_str("\\t");  i += 1; }
            c    => { fmt.push(c); i += 1; }
        }
    }
    (fmt, args)
}

/// Emit a Dust string literal as a Rust expression.
fn emit_str(s: &str) -> String {
    let (fmt, args) = extract_str_args(s);
    if args.is_empty() {
        format!("\"{fmt}\"")
    } else {
        format!("format!(\"{fmt}\", {})", args.join(", "))
    }
}

/// Process a macro's raw string, applying Dust interpolation to its first string arg.
/// e.g. `println!("{stack.pop()}")` → `println!("{}", stack.pop())`
fn process_macro_str(raw: &str) -> String {
    // Find opening delimiter
    let open_idx = match raw.find(|c: char| matches!(c, '(' | '[')) {
        Some(i) => i,
        None => return raw.to_string(),
    };
    let close_ch = if raw.chars().nth(open_idx) == Some('(') { ')' } else { ']' };
    let inner = raw[open_idx + 1..raw.len() - 1].trim_start();

    if !inner.starts_with('"') {
        return raw.to_string(); // first arg not a string literal
    }

    // Parse the string literal (lexer stored it verbatim including escape seqs as text)
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 1; // skip opening "
    let mut str_content = String::new();
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                let esc = match chars[i + 1] {
                    'n' => '\n', 'r' => '\r', 't' => '\t',
                    '"' => '"',  '\\' => '\\', '0' => '\0',
                    c   => c,
                };
                str_content.push(esc);
                i += 2;
            }
            '"' => { i += 1; break; }
            c   => { str_content.push(c); i += 1; }
        }
    }
    let rest = inner[i..].trim();

    let (fmt, args) = extract_str_args(&str_content);
    if args.is_empty() {
        return raw.to_string(); // no expression interpolation, leave as-is
    }

    let prefix = &raw[..=open_idx];
    let mut new_args = format!("\"{fmt}\"");
    for arg in &args { new_args.push_str(&format!(", {arg}")); }
    if !rest.is_empty() { new_args.push_str(&format!(", {rest}")); }
    format!("{prefix}{new_args}{close_ch}")
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
