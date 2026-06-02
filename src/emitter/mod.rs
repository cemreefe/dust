use crate::parser::{self, ast::*};
use std::collections::HashMap;

pub fn emit(items: &[Item]) -> String {
    let emitter = Emitter::new(items);
    emitter.emit_all(items)
}

// ── Sig table ─────────────────────────────────────────────────────────────────

struct SigTable {
    fns:     HashMap<String, Vec<bool>>, // fn_name  → [should_ref per arg]
    methods: HashMap<String, Vec<bool>>, // method_name → [should_ref per arg]
    structs_with_new: std::collections::HashSet<String>, // structs with explicit fn new
}

fn should_ref_param(p: &Param) -> bool {
    if p.keep || p.name == "self" { return false; }
    // After the semantic pass, borrowed params have Ty::Ref; originals were str or Generic
    matches!(&p.ty, Ty::Ref(..))
}

fn build_sig_table(items: &[Item]) -> SigTable {
    let mut fns     = HashMap::new();
    let mut methods = HashMap::new();
    let mut structs_with_new = std::collections::HashSet::new();
    for item in items {
        match item {
            Item::Fn { name, params, .. } => {
                fns.insert(name.clone(), params.iter().map(should_ref_param).collect());
            }
            Item::Struct { name, methods: meths, .. } => {
                for m in meths {
                    methods.insert(m.name.clone(), m.params.iter().map(should_ref_param).collect());
                    if m.name == "new" {
                        structs_with_new.insert(name.clone());
                    }
                }
            }
            _ => {}
        }
    }
    SigTable { fns, methods, structs_with_new }
}

fn is_str_ret(ty: Option<&Ty>) -> bool {
    matches!(ty, Some(Ty::Simple(s)) if s == "str")
}

fn needs_owned_coerce(expr: &Expr) -> bool {
    match expr {
        // Non-interpolated string literals are &str — need .to_string() for String return
        Expr::Str(s) => extract_str_args(s).1.is_empty(),
        // Variables and field accesses may hold &str — coerce conservatively
        Expr::Ident { .. } | Expr::FieldAccess { .. } | Expr::Index { .. } => true,
        _ => false,
    }
}

fn needs_ref(expr: &Expr) -> bool {
    match expr {
        Expr::Ident { .. } | Expr::FieldAccess { .. } | Expr::Index { .. } => true,
        // Interpolated strings emit as format!(...) → String, needs & to coerce to &str
        Expr::Str(s) => !extract_str_args(s).1.is_empty(),
        _ => false,
    }
}

/// Macros whose first argument is a format string.
const FORMAT_MACROS: &[&str] = &[
    "println!", "print!", "eprintln!", "eprint!", "format!", "panic!", "write!", "writeln!",
];

/// Stdlib methods whose arguments take &T.
const STDLIB_REF_METHODS: &[&str] = &[
    "cmp", "partial_cmp", "max", "min", "clamp",
    "eq", "ne",
    "contains", "contains_key", "starts_with", "ends_with",
    "find", "rfind", "split_once", "strip_prefix", "strip_suffix",
    "trim_matches", "trim_start_matches", "trim_end_matches",
    "get", "get_mut", "remove", "binary_search",
    "push_str",
];

// ── Emitter ───────────────────────────────────────────────────────────────────

struct Emitter {
    sig: SigTable,
}

impl Emitter {
    fn new(items: &[Item]) -> Self {
        Self { sig: build_sig_table(items) }
    }

    fn emit_all(&self, items: &[Item]) -> String {
        let mut out = String::new();
        for item in items {
            out.push_str(&self.emit_item(item));
            out.push('\n');
        }
        out
    }

    // ── Items ──────────────────────────────────────────────────────────────────

    fn emit_item(&self, item: &Item) -> String {
        match item {
            Item::Fn { name, generics, is_async, params, ret_ty, body, .. } =>
                self.emit_fn(name, generics, *is_async, params, ret_ty.as_ref(), body),
            Item::Struct { name, generics, traits, fields, methods, .. } =>
                self.emit_struct(name, generics, traits, fields, methods),
            Item::Trait { name, generics, methods, .. } => self.emit_trait(name, generics, methods),
            Item::Enum { name, variants, .. } => emit_enum(name, variants),
            Item::Use { path, .. } => format!("use {};", path),
            Item::Const { name, value, .. } => {
                let (ty, val) = match value {
                    Expr::Str(s)   => {
                        let escaped: String = s.chars().map(|c| match c {
                            '\n' => "\\n".to_string(),
                            '\r' => "\\r".to_string(),
                            '\t' => "\\t".to_string(),
                            '"'  => "\\\"".to_string(),
                            '\\' => "\\\\".to_string(),
                            c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
                            c    => c.to_string(),
                        }).collect();
                        ("&str".to_string(), format!("\"{escaped}\""))
                    }
                    Expr::Int(n)   => ("i64".to_string(), n.to_string()),
                    Expr::Float(f) => ("f64".to_string(), format!("{f}")),
                    Expr::Bool(b)  => ("bool".to_string(), b.to_string()),
                    other          => ("_".to_string(), self.emit_expr(other)),
                };
                format!("const {name}: {ty} = {val};")
            }
        }
    }

    fn emit_fn(&self, name: &str, generics: &str, is_async: bool, params: &[Param], ret_ty: Option<&Ty>, body: &[Stmt]) -> String {
        let async_kw = if is_async { "async " } else { "" };
        let gp = if generics.is_empty() { String::new() } else { format!("<{generics}>") };
        let params_str = params.iter().map(emit_param).collect::<Vec<_>>().join(", ");
        let ret = ret_ty.map(|t| format!(" -> {}", emit_ty_owned(t))).unwrap_or_default();
        let body_str = self.emit_block(body, ret_ty);
        format!("{async_kw}fn {name}{gp}({params_str}){ret} {{\n{body_str}}}\n")
    }

    fn emit_struct(&self, name: &str, generics: &str, traits: &[String], fields: &[Field], methods: &[Method]) -> String {
        let mut out = String::new();
        let own_methods: Vec<&Method> = methods.iter().filter(|m| m.trait_qualifier.is_none()).collect();
        let trait_methods: Vec<&Method> = methods.iter().filter(|m| m.trait_qualifier.is_some()).collect();
        let has_no_arg_new = own_methods.iter().any(|m| m.name == "new" && m.params.is_empty());
        let has_any_new   = own_methods.iter().any(|m| m.name == "new");

        // Generic structs can't always derive Default without bounds — skip for now
        if !has_no_arg_new && !fields.is_empty() && generics.is_empty() {
            out.push_str("#[derive(Default)]\n");
        }
        let gp     = if generics.is_empty() { String::new() } else { format!("<{generics}> ") };
        let gp_use = if generics.is_empty() { String::new() } else { format!("<{}>", strip_bounds(generics)) };
        out.push_str(&format!("struct {name}{gp} {{\n"));
        for f in fields {
            out.push_str(&format!("    {}: {},\n", f.name, emit_ty_owned(&f.ty)));
        }
        out.push_str("}\n");

        let auto_new = if !has_any_new && !fields.is_empty() {
            let params = fields.iter()
                .map(|f| format!("{}: {}", f.name, emit_ty_owned(&f.ty)))
                .collect::<Vec<_>>().join(", ");
            let field_inits = fields.iter()
                .map(|f| format!("        {},\n", f.name))
                .collect::<String>();
            Some(format!("fn new({params}) -> {name} {{\n    {name} {{\n{field_inits}    }}\n}}\n"))
        } else {
            None
        };

        if !own_methods.is_empty() || auto_new.is_some() {
            out.push_str(&format!("\nimpl{gp} {name}{gp_use} {{\n"));
            if let Some(new_fn) = auto_new {
                out.push_str(&indent_block(&new_fn));
            }
            for m in &own_methods {
                out.push_str(&indent_block(&self.emit_method(m)));
            }
            out.push_str("}\n");
        }

        for trait_name in traits {
            let tmethods: Vec<&Method> = trait_methods.iter()
                .filter(|m| m.trait_qualifier.as_deref() == Some(trait_name.split('<').next().unwrap_or(trait_name)))
                .copied().collect();
            out.push_str(&format!("\nimpl{gp} {trait_name} for {name}{gp_use} {{\n"));
            for m in tmethods {
                out.push_str(&indent_block(&self.emit_method(m)));
            }
            out.push_str("}\n");
        }
        out
    }

    fn emit_method(&self, m: &Method) -> String {
        let async_kw = if m.is_async { "async " } else { "" };
        let params_str = m.params.iter().map(emit_param).collect::<Vec<_>>().join(", ");
        let ret = m.ret_ty.as_ref().map(|t| format!(" -> {}", emit_ty_owned(t))).unwrap_or_default();
        match &m.body {
            Some(body) => {
                let body_str = self.emit_block(body, m.ret_ty.as_ref());
                format!("{async_kw}fn {}({params_str}){ret} {{\n{body_str}}}\n", m.name)
            }
            None => format!("{async_kw}fn {}({params_str}){ret};\n", m.name),
        }
    }

    fn emit_trait(&self, name: &str, generics: &str, methods: &[Method]) -> String {
        let gp = if generics.is_empty() { String::new() } else { format!("<{generics}>") };
        let mut out = format!("trait {name}{gp} {{\n");
        for m in methods {
            out.push_str(&format!("    {}", self.emit_method(m)));
        }
        out.push_str("}\n");
        out
    }

    // ── Blocks & Statements ────────────────────────────────────────────────────

    fn emit_block(&self, stmts: &[Stmt], ret_ty: Option<&Ty>) -> String {
        let mut out = String::new();
        let last = stmts.len().saturating_sub(1);
        for (i, stmt) in stmts.iter().enumerate() {
            let line = self.emit_stmt(stmt, i == last, ret_ty);
            for l in line.lines() {
                out.push_str("    ");
                out.push_str(l);
                out.push('\n');
            }
        }
        out
    }

    fn emit_stmt(&self, stmt: &Stmt, is_last: bool, ret_ty: Option<&Ty>) -> String {
        match stmt {
            Stmt::Let { name, ty, value, .. } => {
                let ty_ann = ty.as_ref().map(|t| format!(": {}", emit_ty_owned(t))).unwrap_or_default();
                if let Expr::Ident { name: sentinel, .. } = value {
                    if sentinel == "~uninit~" {
                        return format!("let {name}{ty_ann};");
                    } else if sentinel == "~default~" {
                        return format!("let {name}{ty_ann} = Default::default();");
                    }
                }
                format!("let {name}{ty_ann} = {};", self.emit_expr_bare_owned(value, ty.as_ref()))
            }
            Stmt::Const { name, ty, value, .. } => {
                let ty_ann = ty.as_ref().map(|t| format!(": {}", emit_ty_owned(t))).unwrap_or_default();
                format!("let {name}{ty_ann} = {};", self.emit_expr_bare_owned(value, ty.as_ref()))
            }
            Stmt::Mut { name, ty, value, .. } => {
                let ty_ann = ty.as_ref().map(|t| format!(": {}", emit_ty_owned(t))).unwrap_or_default();
                if let Expr::Ident { name: sentinel, .. } = value {
                    if sentinel == "~uninit~" {
                        return format!("let mut {name}{ty_ann};");
                    } else if sentinel == "~default~" {
                        return format!("let mut {name}{ty_ann} = Default::default();");
                    }
                }
                format!("let mut {name}{ty_ann} = {};", self.emit_expr_bare_owned(value, ty.as_ref()))
            }
            Stmt::Assign { target, value, .. } => {
                format!("{} = {};", self.emit_expr(target), self.emit_expr(value))
            }
            Stmt::CompoundAssign { target, op, value, .. } => {
                if op == "&&" || op == "||" {
                    let t = self.emit_expr(target);
                    format!("{t} = {t} {op} {};", self.emit_expr(value))
                } else {
                    format!("{} {}= {};", self.emit_expr(target), op, self.emit_expr(value))
                }
            }
            Stmt::Expr(e) => {
                if is_last {
                    // Tail expression: coerce to String if this block returns `str`
                    if is_str_ret(ret_ty) && needs_owned_coerce(e) {
                        self.coerce_to_owned(e)
                    } else {
                        strip_outer_parens(self.emit_expr_ret(e, ret_ty))
                    }
                } else {
                    format!("{};", self.emit_expr_ret(e, ret_ty))
                }
            }
            Stmt::Return(Some(e), ..) => {
                let val = if is_str_ret(ret_ty) && needs_owned_coerce(e) {
                    self.coerce_to_owned(e)
                } else {
                    self.emit_expr(e)
                };
                format!("return {};", val)
            }
            Stmt::Return(None, ..) => "return;".into(),
            Stmt::TryCatch { try_block, catch_var, catch_block, .. } => {
                let try_s   = self.emit_block(try_block, ret_ty);
                let catch_s = self.emit_block(catch_block, ret_ty);
                format!("match (|| -> Result<_, _> {{\n{try_s}}})() {{\n    Ok(_) => {{}},\n    Err({catch_var}) => {{\n{catch_s}    }},\n}}")
            }
            Stmt::For { vars, iter, body, .. } => {
                let pat = if vars.len() == 1 { vars[0].clone() } else { format!("({})", vars.join(", ")) };
                format!("for {pat} in {} {{\n{}}}", self.emit_expr(iter), self.emit_block(body, ret_ty))
            }
            Stmt::Break(..)    => "break".into(),
            Stmt::Continue(..) => "continue".into(),
            Stmt::Use { path, .. } => format!("use {path};"),
        }
    }

    // ── Expressions ────────────────────────────────────────────────────────────

    fn emit_expr(&self, expr: &Expr) -> String {
        match expr {
            Expr::Char(c)  => format!("'{}'", c.escape_default()),
            Expr::Int(n)   => n.to_string(),
            Expr::Float(f) => { let s = format!("{f}"); if s.contains('.') { s } else { format!("{s}.0") } }
            Expr::Bool(b)  => b.to_string(),
            Expr::Str(s)   => emit_str(s),
            Expr::Ident { name, .. } => name.clone(),
            Expr::Macro { raw, .. }  => self.process_macro_str(raw),
            Expr::Path { segments, .. } => segments.join("::"),

            Expr::BinOp { op, left, right, .. } => {
                let op_str = match op {
                    BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                    BinOp::Div => "/", BinOp::Mod => "%",
                    BinOp::Eq  => "==", BinOp::NotEq => "!=",
                    BinOp::Lt  => "<",  BinOp::Gt    => ">",
                    BinOp::LtEq => "<=", BinOp::GtEq => ">=",
                    BinOp::And => "&&", BinOp::Or    => "||",
                    BinOp::Assign => "=",
                };
                format!("({} {} {})", self.emit_expr(left), op_str, self.emit_expr(right))
            }

            Expr::UnaryOp { op, expr, .. } => match op {
                UnaryOp::Neg    => format!("-{}", self.emit_expr(expr)),
                UnaryOp::Not    => format!("!{}", self.emit_expr(expr)),
                UnaryOp::Ref    => format!("&{}", self.emit_expr(expr)),
                UnaryOp::RefMut => format!("&mut {}", self.emit_expr(expr)),
                UnaryOp::Deref  => format!("*{}", self.emit_expr(expr)),
            },

            Expr::Call { func, args, .. } => self.emit_call(func, args),

            Expr::FieldAccess { obj, field, .. } => {
                format!("{}.{field}", self.emit_expr(obj))
            }

            Expr::StructLit { name, fields, .. } => {
                let fs = fields.iter()
                    .map(|(k, v)| format!("{k}: {}", self.emit_expr(v)))
                    .collect::<Vec<_>>().join(", ");
                format!("{name} {{ {fs} }}")
            }

            Expr::If { cond, then_branch, else_branch, .. } => {
                let c = self.emit_expr_bare(cond);
                let t = self.emit_expr_as_block(then_branch);
                match else_branch {
                    None    => format!("if {c} {t}"),
                    Some(e) => format!("if {c} {t} else {}", self.emit_expr_as_block(e)),
                }
            }

            Expr::Match { scrutinee, arms, .. } => {
                let s = self.emit_expr(scrutinee);
                let arms_str = arms.iter()
                    .map(|arm| format!("    {} => {},", self.emit_expr(&arm.pattern), self.emit_expr_bare(&arm.body)))
                    .collect::<Vec<_>>().join("\n");
                format!("match {s} {{\n{arms_str}\n}}")
            }

            Expr::Closure { params, body, .. } => {
                let ps = params.iter().map(|p| {
                    if p.ty == Ty::SelfTy { "self".into() }
                    else { format!("{}: {}", p.name, emit_ty_owned(&p.ty)) }
                }).collect::<Vec<_>>().join(", ");
                format!("|{ps}| {}", self.emit_expr_bare(body))
            }

            Expr::Block { stmts, .. } => format!("{{\n{}}}", self.emit_block(stmts, None)),

            Expr::Return(Some(e), ..) => format!("return {}", self.emit_expr(e)),
            Expr::Return(None, ..)    => "return".into(),
            Expr::Try(e, ..)          => format!("{}?", self.emit_expr(e)),
            Expr::Unwrap(e, ..)       => format!("{}.unwrap()", self.emit_expr(e)),
            Expr::Await(e, ..)        => format!("{}.await", self.emit_expr(e)),

            Expr::Turbofish { inner, type_args, .. } => {
                format!("{}::<{}>", self.emit_expr(inner), type_args)
            }
            Expr::Index { obj, idx, .. } => {
                format!("{}[{}]", self.emit_expr(obj), self.emit_expr(idx))
            }
            Expr::Cast { expr, ty, .. } => {
                format!("{} as {}", self.emit_expr(expr), emit_ty_owned(ty))
            }
        }
    }

    fn emit_call(&self, func: &Expr, args: &[Expr]) -> String {
        const ENUM_VARIANTS: &[&str] = &["Some", "None", "Ok", "Err"];

        // Struct constructor: Uppercase(args) → Type::default() / Type::new(args)
        // Also handle `str()` as a type constructor (maps to String)
        if let Expr::Ident { name, .. } = func {
            let name = &(if name == "str" { "String".to_string() } else { name.clone() });
            if name.starts_with(|c: char| c.is_uppercase()) && !ENUM_VARIANTS.contains(&name.as_str()) {
                if args.is_empty() {
                    let ctor = if self.sig.structs_with_new.contains(name.as_str()) { "new" } else { "default" };
                    return format!("{name}::{ctor}()");
                }
                let a = self.emit_args(args, self.sig.fns.get(name.as_str()).map(Vec::as_slice));
                return format!("{name}::new({a})");
            }
            // Dust free function
            if let Some(refs) = self.sig.fns.get(name.as_str()) {
                let a = self.emit_args(args, Some(refs));
                return format!("{name}({a})");
            }
        }

        // Method call
        if let Expr::FieldAccess { obj, field, .. } = func {
            let obj_s = self.emit_expr(obj);
            // Dust method
            if let Some(refs) = self.sig.methods.get(field.as_str()) {
                let a = self.emit_args(args, Some(refs));
                return format!("{obj_s}.{field}({a})");
            }
            // Stdlib ref-arg method — always insert & unless arg is already a ref expression
            if STDLIB_REF_METHODS.contains(&field.as_str()) {
                let a = args.iter().map(|arg| {
                    let s = self.emit_expr(arg);
                    let already_ref = matches!(arg,
                        Expr::UnaryOp { op: UnaryOp::Ref, .. } |
                        Expr::UnaryOp { op: UnaryOp::RefMut, .. }
                    );
                    if already_ref { s } else { format!("&{s}") }
                }).collect::<Vec<_>>().join(", ");
                return format!("{obj_s}.{field}({a})");
            }
        }

        let f = self.emit_expr(func);
        let a = args.iter().map(|arg| {
            let s = self.emit_expr(arg);
            // Index into a collection (e.g. args[1]) yields &T in Rust — always needs & when passed to fns
            if matches!(arg, Expr::Index { .. }) { format!("&{s}") } else { s }
        }).collect::<Vec<_>>().join(", ");
        format!("{f}({a})")
    }

    fn process_macro_str(&self, raw: &str) -> String {
        let open_idx = match raw.find(|c: char| matches!(c, '(' | '[')) {
            Some(i) => i,
            None => return raw.to_string(),
        };
        let close_ch = if raw.chars().nth(open_idx) == Some('(') { ')' } else { ']' };
        let inner = raw[open_idx + 1..raw.len() - 1].trim_start();
        if !inner.starts_with('"') {
            let macro_name = &raw[..=open_idx]; // includes the '('
            let name_only = raw[..open_idx].trim();
            if FORMAT_MACROS.contains(&name_only) && is_single_arg(inner) {
                let emitted = if let Some(expr) = parser::parse_expr_str(inner) {
                    self.emit_expr(&expr)
                } else {
                    inner.to_string()
                };
                return format!("{macro_name}\"{{}}\", {emitted}{close_ch}");
            }
            return raw.to_string();
        }

        let chars: Vec<char> = inner.chars().collect();
        let mut i = 1;
        let mut str_content = String::new();
        while i < chars.len() {
            match chars[i] {
                '\\' if i + 1 < chars.len() => {
                    let esc = match chars[i + 1] {
                        'n' => '\n', 'r' => '\r', 't' => '\t',
                        '"' => '"',  '\\' => '\\', '0' => '\0', c => c,
                    };
                    str_content.push(esc);
                    i += 2;
                }
                '"' => { i += 1; break; }
                c   => { str_content.push(c); i += 1; }
            }
        }
        let byte_offset: usize = inner.chars().take(i).map(|c| c.len_utf8()).sum();
        let rest = inner[byte_offset..].trim();
        let (fmt, raw_args) = extract_str_args(&str_content);
        if raw_args.is_empty() { return raw.to_string(); }

        // Re-parse and re-emit each interpolated expression so auto-ref applies.
        let emitted_args: Vec<String> = raw_args.iter().map(|expr_str| {
            if let Some(expr) = parser::parse_expr_str(expr_str) {
                self.emit_expr(&expr)
            } else {
                expr_str.clone()
            }
        }).collect();

        let prefix = &raw[..=open_idx];
        let mut new_args = format!("\"{fmt}\"");
        for arg in &emitted_args { new_args.push_str(&format!(", {arg}")); }
        if !rest.is_empty() { new_args.push_str(&format!(", {rest}")); }
        format!("{prefix}{new_args}{close_ch}")
    }

    /// Emit call args, auto-inserting `&` where the sig says the param is borrowed.
    fn emit_args(&self, args: &[Expr], refs: Option<&[bool]>) -> String {
        args.iter().enumerate().map(|(i, arg)| {
            let s = self.emit_expr(arg);
            let should_ref = refs.and_then(|r| r.get(i)).copied().unwrap_or(false);
            if should_ref && needs_ref(arg) { format!("&{s}") } else { s }
        }).collect::<Vec<_>>().join(", ")
    }

    fn emit_expr_bare(&self, expr: &Expr) -> String {
        strip_outer_parens(self.emit_expr(expr))
    }

    fn emit_expr_owned(&self, expr: &Expr, ty: Option<&Ty>) -> String {
        // Only coerce to String when the type is explicitly `str`; otherwise let Rust infer.
        let wants_string = matches!(ty, Some(Ty::Simple(s)) if s == "str");
        match expr {
            Expr::Str(s) if wants_string => {
                let (fmt, args) = extract_str_args(s);
                if args.is_empty() { format!("\"{fmt}\".to_string()") }
                else { format!("format!(\"{fmt}\", {})", args.join(", ")) }
            }
            _ => self.emit_expr(expr),
        }
    }

    /// Coerce expr to String when returning from a `-> str` function.
    /// Only plain literals and variables need coercion; format!/to_string() calls already produce String.
    fn coerce_to_owned(&self, expr: &Expr) -> String {
        let s = self.emit_expr(expr);
        if needs_owned_coerce(expr) { format!("{s}.to_string()") } else { s }
    }

    fn emit_expr_bare_owned(&self, expr: &Expr, ty: Option<&Ty>) -> String {
        strip_outer_parens(self.emit_expr_owned(expr, ty))
    }

    fn emit_expr_as_block(&self, expr: &Expr) -> String {
        self.emit_expr_as_block_ret(expr, None)
    }

    fn emit_expr_as_block_ret(&self, expr: &Expr, ret_ty: Option<&Ty>) -> String {
        match expr {
            Expr::Block { stmts, .. } => format!("{{\n{}}}", self.emit_block(stmts, ret_ty)),
            _ => format!("{{ {} }}", self.emit_expr_ret(expr, ret_ty)),
        }
    }

    /// Like emit_expr but threads ret_ty into block-containing sub-expressions (if, match, block).
    fn emit_expr_ret(&self, expr: &Expr, ret_ty: Option<&Ty>) -> String {
        match expr {
            Expr::If { cond, then_branch, else_branch, .. } => {
                let c = self.emit_expr_bare(cond);
                let t = self.emit_expr_as_block_ret(then_branch, ret_ty);
                match else_branch {
                    None    => format!("if {c} {t}"),
                    Some(e) => format!("if {c} {t} else {}", self.emit_expr_as_block_ret(e, ret_ty)),
                }
            }
            Expr::Match { scrutinee, arms, .. } => {
                let s = self.emit_expr(scrutinee);
                let arms_str = arms.iter()
                    .map(|arm| format!("    {} => {},", self.emit_expr(&arm.pattern), self.emit_expr_as_block_ret(&arm.body, ret_ty)))
                    .collect::<Vec<_>>().join("\n");
                format!("match {s} {{\n{arms_str}\n}}")
            }
            Expr::Block { stmts, .. } => format!("{{\n{}}}", self.emit_block(stmts, ret_ty)),
            _ => self.emit_expr(expr),
        }
    }
}

// ── Pure helpers (no sig needed) ──────────────────────────────────────────────

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

fn emit_param(p: &Param) -> String {
    if p.name == "self" {
        return if p.keep { "self".into() } else { "&mut self".into() };
    }
    if p.keep && p.mutable {
        return format!("mut {}: {}", p.name, emit_ty_owned(&p.ty));
    }
    if !p.keep && p.mutable {
        // mutable borrow: Ref(T) → &mut T
        let inner = match &p.ty {
            Ty::Ref(inner) => emit_ty_ref(inner),
            other => emit_ty_owned(other),
        };
        return format!("{}: &mut {}", p.name, inner);
    }
    format!("{}: {}", p.name, emit_ty_owned(&p.ty))
}

fn emit_ty_owned(ty: &Ty) -> String {
    match ty {
        Ty::Simple(s) if s == "str" => "String".into(),
        Ty::Simple(s) => s.clone(),
        Ty::Generic(name, args) => {
            let inner = args.iter().map(emit_ty_owned).collect::<Vec<_>>().join(", ");
            format!("{name}<{inner}>")
        }
        Ty::Tuple(elems) => format!("({})", elems.iter().map(emit_ty_owned).collect::<Vec<_>>().join(", ")),
        Ty::Ref(inner)   => format!("&{}", emit_ty_ref(inner)),
        Ty::SelfTy       => "Self".into(),
    }
}

fn emit_ty_ref(ty: &Ty) -> String {
    match ty {
        Ty::Simple(s) if s == "str" => "str".into(),
        _ => emit_ty_owned(ty),
    }
}

fn strip_outer_parens(s: String) -> String {
    if s.starts_with('(') && s.ends_with(')') {
        let mut depth = 0usize;
        let chars: Vec<char> = s.chars().collect();
        let mut matched = false;
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && i == chars.len() - 1 { matched = true; }
                    else if depth == 0 { break; }
                }
                _ => {}
            }
        }
        if matched { return s[1..s.len()-1].to_string(); }
    }
    s
}

fn is_single_arg(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return false,
            _ => {}
        }
    }
    true
}

fn emit_str(s: &str) -> String {
    let (fmt, args) = extract_str_args(s);
    if args.is_empty() { format!("\"{fmt}\"") }
    else { format!("format!(\"{fmt}\", {})", args.join(", ")) }
}


fn extract_str_args(s: &str) -> (String, Vec<String>) {
    let mut fmt  = String::new();
    let mut args = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                let next = chars.get(i + 1).copied().unwrap_or('\0');
                let is_interp = next.is_alphabetic() || next == '_'
                    || next.is_ascii_digit() || next == '-' || next == '!' || next == '(';
                if !is_interp {
                    fmt.push_str("{{"); i += 1;
                } else {
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
                        if expr.contains(':') {
                            fmt.push('{'); fmt.push_str(&expr); fmt.push('}');
                        } else {
                            fmt.push_str("{}");
                            args.push(single_to_double_quotes(&expr));
                        }
                        i = j + 1;
                    } else {
                        fmt.push_str("{{"); i += 1;
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

fn single_to_double_quotes(expr: &str) -> String {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if (chars[i] == 'c' || chars[i] == 'b') && chars.get(i + 1) == Some(&'\'') {
            out.push(chars[i]); i += 1; continue;
        }
        if chars[i] == '\'' {
            out.push('"'); i += 1;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    out.push(chars[i]); out.push(chars[i + 1]); i += 2;
                } else { out.push(chars[i]); i += 1; }
            }
            out.push('"'); i += 1;
        } else { out.push(chars[i]); i += 1; }
    }
    out
}

fn indent_block(s: &str) -> String {
    s.lines().map(|l| format!("    {l}\n")).collect()
}

/// Strip bounds from generic params: `"A: Clone, B: Default"` → `"A, B"`
fn strip_bounds(generics: &str) -> String {
    generics.split(',')
        .map(|p| p.trim().split(':').next().unwrap_or("").trim().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser, semantic};

    fn transpile(src: &str) -> String {
        let tokens = lexer::lex(src).expect("lex failed");
        let ast    = parser::parse(&tokens).expect("parse failed");
        let ast    = semantic::analyze(ast).expect("semantic failed");
        emit(&ast)
    }

    // ── top-level const ───────────────────────────────────────────────────────

    #[test]
    fn const_str_literal() {
        let out = transpile(r#"const RESET = "\x1b[0m""#);
        assert!(out.contains(r#"const RESET: &str = "\x1b[0m";"#), "got: {out}");
    }

    #[test]
    fn const_int_literal() {
        let out = transpile("const MAX = 42");
        assert!(out.contains("const MAX: i64 = 42;"), "got: {out}");
    }

    // ── unified str type ──────────────────────────────────────────────────────

    #[test]
    fn str_param_becomes_ref_str() {
        let out = transpile("fn greet(name: str) -> bool\n    true");
        assert!(out.contains("name: &str"), "got: {out}");
    }

    #[test]
    fn str_return_becomes_string() {
        let out = transpile("fn greet(name: str) -> str\n    \"hello\"");
        assert!(out.contains("-> String"), "got: {out}");
    }

    #[test]
    fn str_field_becomes_string() {
        let out = transpile("struct Foo\n    label: str");
        assert!(out.contains("label: String"), "got: {out}");
    }

    #[test]
    fn vec_str_becomes_vec_string() {
        let out = transpile("fn main()\n    let v: Vec<str> = vec![]");
        assert!(out.contains("Vec<String>"), "got: {out}");
    }

    #[test]
    fn str_constructor_becomes_string_new() {
        let out = transpile("fn main()\n    let s = str()");
        assert!(out.contains("String::new()") || out.contains("String::default()"), "got: {out}");
    }

    // ── default / uninit bindings ─────────────────────────────────────────────

    #[test]
    fn mut_typed_default_initializes() {
        let out = transpile("fn main()\n    mut lines: Vec<str>");
        assert!(out.contains("let mut lines: Vec<String> = Default::default();"), "got: {out}");
    }

    #[test]
    fn mut_typed_tilde_is_uninit() {
        let out = transpile("fn main()\n    mut buf: Vec<u8> ~");
        assert!(out.contains("let mut buf: Vec<u8>;"), "got: {out}");
        assert!(!out.contains("default"), "should not have initializer, got: {out}");
    }

    #[test]
    fn let_typed_default_initializes() {
        let out = transpile("fn main()\n    let x: i32");
        assert!(out.contains("let x: i32 = Default::default();"), "got: {out}");
    }

    // ── format macro ergonomics ───────────────────────────────────────────────

    #[test]
    fn println_single_expr_autowraps() {
        let out = transpile("fn main()\n    let x = 1\n    println!(x)");
        assert!(out.contains(r#"println!("{}", x)"#), "got: {out}");
    }

    #[test]
    fn println_string_literal_unchanged() {
        let out = transpile(r#"fn main()
    println!("hello")"#);
        assert!(out.contains(r#"println!("hello")"#), "got: {out}");
    }

    #[test]
    fn eprintln_single_expr_autowraps() {
        let out = transpile("fn main()\n    let msg = str()\n    eprintln!(msg)");
        assert!(out.contains(r#"eprintln!("{}", msg)"#), "got: {out}");
    }

    // ── auto-ref ──────────────────────────────────────────────────────────────

    #[test]
    fn stdlib_contains_autorefs_arg() {
        let out = transpile("fn main()\n    let kws = vec![\"fn\"]\n    let word = str()\n    kws.contains(word)");
        assert!(out.contains("kws.contains(&word)"), "got: {out}");
    }

    #[test]
    fn index_expr_autorefs_in_fn_call() {
        let out = transpile("fn main()\n    let args: Vec<str> = vec![]\n    let s = str()\n    s.push_str(args[0])");
        assert!(out.contains("&args[0]"), "got: {out}");
    }

    // ── str return coercion ───────────────────────────────────────────────────

    #[test]
    fn str_return_coerces_literal() {
        let out = transpile("fn label() -> str\n    \"hello\"");
        assert!(out.contains(r#""hello".to_string()"#), "got: {out}");
    }

    #[test]
    fn str_return_coerces_in_if_branch() {
        let out = transpile("fn f(x: bool) -> str\n    if x\n        return \"yes\"\n    \"no\"");
        assert!(out.contains(r#""yes".to_string()"#), "got: {out}");
        assert!(out.contains(r#""no".to_string()"#), "got: {out}");
    }
}
