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
    struct_fields: HashMap<String, HashMap<String, Ty>>, // struct_name → field_name → Ty
    enum_variants: HashMap<String, String>, // variant_name → enum_name
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
    let mut struct_fields = HashMap::new();
    let mut enum_variants = HashMap::new();
    for item in items {
        match item {
            Item::Fn { name, params, .. } => {
                fns.insert(name.clone(), params.iter().map(should_ref_param).collect());
            }
            Item::Struct { name, fields, methods: meths, .. } => {
                struct_fields.insert(
                    name.clone(),
                    fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect(),
                );
                for m in meths {
                    methods.insert(m.name.clone(), m.params.iter().map(should_ref_param).collect());
                    if m.name == "new" {
                        structs_with_new.insert(name.clone());
                    }
                }
            }
            Item::Enum { name, variants, traits: _, .. } => {
                for v in variants {
                    enum_variants.insert(v.name.clone(), name.clone());
                }
            }
            _ => {}
        }
    }
    SigTable { fns, methods, structs_with_new, struct_fields, enum_variants }
}

fn is_str_ret(ty: Option<&Ty>) -> bool {
    matches!(ty, Some(Ty::Simple(s)) if s == "str")
}

/// Methods on `str`/`&str` that return a `&str` slice — callers need `.to_string()` in owned position.
const STR_SLICE_METHODS: &[&str] = &[
    "trim", "trim_start", "trim_end",
    "trim_matches", "trim_start_matches", "trim_end_matches",
    "as_str",
];

fn needs_owned_coerce(expr: &Expr) -> bool {
    match expr {
        // Non-interpolated string literals are &str — need .to_string() for String return
        Expr::Str(s) => extract_str_args(s).1.is_empty(),
        // Variables and field accesses may hold &str — coerce conservatively
        Expr::Ident { .. } | Expr::FieldAccess { .. } | Expr::Index { .. } => true,
        // Method calls that return &str slices of their receiver
        Expr::Call { func, .. } => matches!(
            func.as_ref(),
            Expr::FieldAccess { field, .. } if STR_SLICE_METHODS.contains(&field.as_str())
        ),
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
    "println!", "print!", "eprintln!", "eprint!", "format!", "panic!",
];

/// Macros whose second argument is a format string (first is a writer/destination).
const FORMAT_MACROS_WRITER: &[&str] = &["write!", "writeln!"];

/// Stdlib methods whose arguments take &T.
const STDLIB_REF_METHODS: &[&str] = &[
    "cmp", "partial_cmp", "max", "min", "clamp",
    "eq", "ne",
    "contains", "contains_key", "starts_with", "ends_with",
    "find", "rfind", "split_once", "strip_prefix", "strip_suffix",
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
            Item::Fn { name, generics, is_async, params, ret_ty, body, attrs, .. } => {
                let attr_lines = attrs.iter().map(|a| format!("#[{a}]\n")).collect::<String>();
                format!("{}{}", attr_lines, self.emit_fn(name, generics, *is_async, params, ret_ty.as_ref(), body))
            }
            Item::Struct { name, generics, traits, fields, methods, assoc_types, attrs, .. } => {
                self.emit_struct(name, generics, traits, fields, methods, assoc_types, attrs)
            }
            Item::Trait { name, generics, methods, .. } => self.emit_trait(name, generics, methods),
            Item::Enum { name, traits, variants, attrs, .. } => {
                emit_enum(name, traits, variants, attrs)
            }
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
        let generics = &ensure_lifetimes_declared(generics);
        let gp = if generics.is_empty() { String::new() } else { format!("<{generics}>") };
        let params_str = params.iter().map(emit_param).collect::<Vec<_>>().join(", ");
        let ret = ret_ty.map(|t| format!(" -> {}", emit_ty_owned(t))).unwrap_or_default();
        let body_str = if ret_ty.is_some() { self.emit_block(body, ret_ty) } else { self.emit_block_no_tail(body, None) };
        format!("{async_kw}fn {name}{gp}({params_str}){ret} {{\n{body_str}}}\n")
    }

    fn emit_struct(&self, name: &str, generics: &str, traits: &[String], fields: &[Field], methods: &[Method], assoc_types: &[(String, Ty)], attrs: &[String]) -> String {
        const DERIVE_TRAITS: &[&str] = &[
            "Deserialize", "Serialize", "Clone", "Copy", "Hash",
            "PartialEq", "Eq", "PartialOrd", "Ord",
        ];
        let (derive_traits, impl_traits): (Vec<&String>, Vec<&String>) =
            traits.iter().partition(|t| DERIVE_TRAITS.contains(&t.as_str()));

        let mut out = String::new();
        let own_methods: Vec<&Method> = methods.iter().filter(|m| m.trait_qualifier.is_none()).collect();
        let trait_methods: Vec<&Method> = methods.iter().filter(|m| m.trait_qualifier.is_some()).collect();
        let has_no_arg_new = own_methods.iter().any(|m| m.name == "new" && m.params.is_empty());
        let has_any_new   = own_methods.iter().any(|m| m.name == "new");

        // derive comes first, then custom attrs, so serde helper attrs work
        let mut derives = vec![];
        if !has_no_arg_new && !fields.is_empty() && generics.is_empty() {
            derives.push("Default".to_string());
        }
        for t in &derive_traits { derives.push(t.to_string()); }
        if !derives.is_empty() {
            out.push_str(&format!("#[derive({})]\n", derives.join(", ")));
        }
        for a in attrs {
            out.push_str(&format!("#[{a}]\n"));
        }
        let gp     = if generics.is_empty() { String::new() } else { format!("<{generics}> ") };
        let gp_use = if generics.is_empty() { String::new() } else { format!("<{}>", strip_bounds(generics)) };
        out.push_str(&format!("struct {name}{gp} {{\n"));
        for f in fields {
            for a in &f.attrs {
                out.push_str(&format!("    #[{a}]\n"));
            }
            out.push_str(&format!("    {}: {},\n", f.name, emit_ty_owned(&f.ty)));
        }
        out.push_str("}\n");

        let auto_new = if !has_any_new && !fields.is_empty() {
            let params = fields.iter()
                .map(|f| format!("{}: {}", f.name, emit_ty_owned(&f.ty)))
                .collect::<Vec<_>>().join(", ");
            let field_names = fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>();
            let body = if fields.len() <= 4 {
                format!("{name} {{ {} }}", field_names.join(", "))
            } else {
                let inits = field_names.iter().map(|n| format!("        {n},\n")).collect::<String>();
                format!("{name} {{\n{inits}    }}")
            };
            Some(format!("fn new({params}) -> {name} {{\n    {body}\n}}\n"))
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

        for trait_name in &impl_traits {
            let base = trait_name.split('<').next().unwrap_or(trait_name);
            // Extract base for matching (strip module path)
            let base_short = base.rsplit("::").next().unwrap_or(base);
            let tmethods: Vec<&Method> = trait_methods.iter()
                .filter(|m| {
                    let tq = m.trait_qualifier.as_deref().unwrap_or("");
                    tq == base || tq == base_short
                })
                .copied().collect();
            // Collect lifetimes from trait generic args: 'de, 'a, etc.
            let lifetimes = extract_lifetimes_from_str(trait_name);
            let impl_gp = if !generics.is_empty() && !lifetimes.is_empty() {
                format!("<{generics}, {}>", lifetimes.join(", "))
            } else if !generics.is_empty() {
                format!("<{generics}>")
            } else if !lifetimes.is_empty() {
                format!("<{}>", lifetimes.join(", "))
            } else {
                String::new()
            };
            out.push_str(&format!("\nimpl{impl_gp} {trait_name} for {name}{gp_use} {{\n"));
            for (aname, aty) in assoc_types {
                out.push_str(&format!("    type {aname} = {};\n", emit_ty_owned(aty)));
            }
            for m in tmethods {
                out.push_str(&indent_block(&self.emit_method(m)));
            }
            out.push_str("}\n");
        }
        out
    }

    fn emit_method(&self, m: &Method) -> String {
        let async_kw = if m.is_async { "async " } else { "" };
        // Don't auto-declare lifetimes in methods — they come from the enclosing impl<'lt> block
        let gp = if m.generics.is_empty() { String::new() } else { format!("<{}>", m.generics) };
        let params_str = m.params.iter().map(emit_param).collect::<Vec<_>>().join(", ");
        let ret = m.ret_ty.as_ref().map(|t| format!(" -> {}", emit_ty_owned(t))).unwrap_or_default();
        match &m.body {
            Some(body) => {
                let body_str = if m.ret_ty.is_some() { self.emit_block(body, m.ret_ty.as_ref()) } else { self.emit_block_no_tail(body, None) };
                format!("{async_kw}fn {}{gp}({params_str}){ret} {{\n{body_str}}}\n", m.name)
            }
            None => format!("{async_kw}fn {}{gp}({params_str}){ret};\n", m.name),
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
        self.emit_block_tail(stmts, ret_ty, true)
    }

    fn emit_block_no_tail(&self, stmts: &[Stmt], ret_ty: Option<&Ty>) -> String {
        self.emit_block_tail(stmts, ret_ty, false)
    }

    fn emit_block_tail(&self, stmts: &[Stmt], ret_ty: Option<&Ty>, tail: bool) -> String {
        let mut out = String::new();
        let last = stmts.len().saturating_sub(1);
        for (i, stmt) in stmts.iter().enumerate() {
            let line = self.emit_stmt(stmt, tail && i == last, ret_ty);
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
                if name.starts_with('(') {
                    return format!("let {name} = {};", self.emit_expr(value));
                }
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
                format!("for {pat} in {} {{\n{}}}", self.emit_expr(iter), self.emit_block_no_tail(body, ret_ty))
            }
            Stmt::While { cond, body, .. } => {
                format!("while {} {{\n{}}}", self.emit_expr(cond), self.emit_block_no_tail(body, ret_ty))
            }
            Stmt::WhileLet { pattern, value, body, .. } => {
                format!("while let {} = {} {{\n{}}}", self.emit_expr(pattern), self.emit_expr(value), self.emit_block_no_tail(body, ret_ty))
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
            Expr::ByteChar(c) => format!("b'{}'", c.escape_default()),
            Expr::Int(n)   => n.to_string(),
            Expr::Float(f) => { let s = format!("{f}"); if s.contains('.') { s } else { format!("{s}.0") } }
            Expr::Bool(b)  => b.to_string(),
            Expr::Str(s)   => emit_str(s),
            Expr::Ident { name, .. } => name.clone(),
            Expr::Macro { raw, .. }  => self.process_macro_str(raw),
            Expr::Path { segments, .. } => segments.join("::"),

            Expr::BinOp { op, left, right, .. } => {
                if matches!(op, BinOp::Pow) {
                    let base = self.emit_expr(left);
                    let exp  = self.emit_expr(right);
                    // Use powi for integer literals, powf otherwise
                    let method = if matches!(right.as_ref(), Expr::Int(_)) { "powi" } else { "powf" };
                    return format!("{base}.{method}({exp})");
                }
                let op_str = match op {
                    BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                    BinOp::Div => "/", BinOp::Mod => "%",
                    BinOp::Eq  => "==", BinOp::NotEq => "!=",
                    BinOp::Lt  => "<",  BinOp::Gt    => ">",
                    BinOp::LtEq => "<=", BinOp::GtEq => ">=",
                    BinOp::And => "&&", BinOp::Or    => "||",
                    BinOp::Assign => "=",
                    BinOp::Pow => unreachable!(),
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

            Expr::StructLit { name, fields, spread, .. } => {
                let field_types = self.sig.struct_fields.get(name.as_str());
                let mut parts: Vec<String> = fields.iter()
                    .map(|(k, v)| {
                        let ty = field_types.and_then(|m| m.get(k.as_str()));
                        format!("{k}: {}", self.emit_expr_bare_owned(v, ty))
                    })
                    .collect();
                if let Some(sp) = spread {
                    parts.push(format!("..{}", self.emit_expr_bare(sp)));
                }
                let fs = parts.join(", ");
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

            Expr::Closure { params, body, is_move, .. } => {
                let ps = params.iter().map(|p| {
                    if p.ty == Ty::SelfTy { "self".into() }
                    else { format!("{}: {}", p.name, emit_ty_owned(&p.ty)) }
                }).collect::<Vec<_>>().join(", ");
                let move_kw = if *is_move { "move " } else { "" };
                let body_str = match body.as_ref() {
                    Expr::Block { stmts, .. } => format!("{{\n{}}}", self.emit_block(stmts, None)),
                    _ => self.emit_expr_bare(body),
                };
                format!("{move_kw}|{ps}| {body_str}")
            }

            Expr::IfLet { pattern, value, then_branch, else_branch, .. } => {
                let pat = self.emit_expr(pattern);
                let val = self.emit_expr(value);
                let t = self.emit_expr_as_block(then_branch);
                match else_branch {
                    None    => format!("if let {pat} = {val} {t}"),
                    Some(e) => format!("if let {pat} = {val} {t} else {}", self.emit_expr_as_block(e)),
                }
            }

            Expr::Tuple(elems) => {
                let inner = elems.iter().map(|e| self.emit_expr(e)).collect::<Vec<_>>().join(", ");
                format!("({inner})")
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
            Expr::NamespacedVec { ns, items } => {
                let inner = items.iter()
                    .map(|item| {
                        // Emit variant name + args without the auto-prefix from emit_call
                        match item {
                            Expr::Call { func, args, .. } => {
                                if let Expr::Ident { name, .. } = func.as_ref() {
                                    let a = args.iter().map(|a| self.emit_expr(a)).collect::<Vec<_>>().join(", ");
                                    return format!("{ns}::{name}({a})");
                                }
                                format!("{ns}::{}", self.emit_expr(item))
                            }
                            Expr::Ident { name, .. } => format!("{ns}::{name}"),
                            _ => format!("{ns}::{}", self.emit_expr(item)),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("vec![{inner}]")
            }

            Expr::VecLit { items, .. } => {
                // If there are spread items, emit as a block that extends
                // e.g. vec![..a, 1, 2] → { let mut __v = a; __v.extend([1, 2]); __v }
                // If items is just a plain list (no spread), emit vec![...]
                let has_spread = items.iter().any(|i| matches!(i, VecItem::Spread(_)));
                if has_spread {
                    // Find first spread item — collect into it, then extend with trailing items
                    let mut parts: Vec<String> = Vec::new();
                    let mut init_done = false;
                    let mut trailing: Vec<String> = Vec::new();
                    let mut in_trailing = false;
                    for item in items {
                        match item {
                            VecItem::Spread(e) if !init_done => {
                                parts.push(format!("let mut __v = {};", self.emit_expr_bare(e)));
                                init_done = true;
                                in_trailing = true;
                            }
                            VecItem::Spread(e) => {
                                if !trailing.is_empty() {
                                    parts.push(format!("__v.extend([{}]);", trailing.join(", ")));
                                    trailing.clear();
                                }
                                parts.push(format!("__v.extend({});", self.emit_expr_bare(e)));
                            }
                            VecItem::Expr(e) if in_trailing => {
                                trailing.push(self.emit_expr(e));
                            }
                            VecItem::Expr(e) => {
                                // items before first spread — prepend to vec
                                parts.push(format!("let mut __v = vec![{}];", self.emit_expr(e)));
                                init_done = true;
                                in_trailing = true;
                            }
                        }
                    }
                    if !trailing.is_empty() {
                        parts.push(format!("__v.extend([{}]);", trailing.join(", ")));
                    }
                    parts.push("__v".to_string());
                    format!("{{ {} }}", parts.join(" "))
                } else {
                    let inner = items.iter().map(|i| match i {
                        VecItem::Expr(e) => self.emit_expr(e),
                        VecItem::Spread(_) => unreachable!(),
                    }).collect::<Vec<_>>().join(", ");
                    format!("vec![{inner}]")
                }
            }
        }
    }

    fn emit_call(&self, func: &Expr, args: &[Expr]) -> String {
        const ENUM_VARIANTS: &[&str] = &["Some", "None", "Ok", "Err"];

        // Struct constructor: Uppercase(args) → Type::default() / Type::new(args)
        // Also handle `str()` as a type constructor (maps to String)
        if let Expr::Ident { name, .. } = func {
            let name = &(if name == "str" { "String".to_string() } else { name.clone() });
            // Enum variant: Circle(r) → Shape::Circle(r)
            if let Some(enum_name) = self.sig.enum_variants.get(name.as_str()) {
                let a = args.iter().map(|a| self.emit_expr(a)).collect::<Vec<_>>().join(", ");
                return format!("{enum_name}::{name}({a})");
            }
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
        let macro_name = &raw[..=open_idx]; // includes the '('
        let name_only = raw[..open_idx].trim();

        if !inner.starts_with('"') {
            // Writer macros: write!(writer, "fmt {x}") — skip first arg, process rest
            if FORMAT_MACROS_WRITER.contains(&name_only) {
                if let Some(comma_pos) = first_top_level_comma(inner) {
                    let writer_arg = inner[..comma_pos].trim();
                    let rest = inner[comma_pos + 1..].trim();
                    // Delegate by re-processing as if a normal format macro on `rest`
                    let fake_raw = format!("println!({rest})");
                    let processed = self.process_macro_str(&fake_raw);
                    // Extract the inner part from "println!(...)" and rebuild write!(...)
                    if let Some(inner_start) = processed.find('(') {
                        let processed_inner = &processed[inner_start + 1..processed.len() - 1];
                        return format!("{macro_name}{writer_arg}, {processed_inner}{close_ch}");
                    }
                }
            }
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

        // Writer macros with string as first visible arg after writer: write!(f, "fmt {x}")
        // would have been caught above; but if inner starts with '"', it's a normal format macro.
        if FORMAT_MACROS_WRITER.contains(&name_only) {
            // inner starts with '"' means write!("fmt") — unusual but handle gracefully
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
            _ => format!("{{ {} }}", strip_outer_parens(self.emit_expr_ret(expr, ret_ty))),
        }
    }

    /// Like emit_expr but threads ret_ty into block-containing sub-expressions (if, match, block).
    fn emit_expr_ret(&self, expr: &Expr, ret_ty: Option<&Ty>) -> String {
        match expr {
            Expr::IfLet { pattern, value, then_branch, else_branch, .. } => {
                let pat = self.emit_expr(pattern);
                let val = self.emit_expr(value);
                let t = self.emit_expr_as_block_ret(then_branch, ret_ty);
                match else_branch {
                    None    => format!("if let {pat} = {val} {t}"),
                    Some(e) => format!("if let {pat} = {val} {t} else {}", self.emit_expr_as_block_ret(e, ret_ty)),
                }
            }
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
            _ => {
                if is_str_ret(ret_ty) && needs_owned_coerce(expr) {
                    self.coerce_to_owned(expr)
                } else {
                    self.emit_expr(expr)
                }
            }
        }
    }
}

// ── Pure helpers (no sig needed) ──────────────────────────────────────────────

fn emit_enum(name: &str, traits: &[String], variants: &[Variant], attrs: &[String]) -> String {
    const DERIVE_TRAITS: &[&str] = &[
        "Deserialize", "Serialize", "Clone", "Copy", "Hash",
        "PartialEq", "Eq", "PartialOrd", "Ord",
    ];
    let derive_traits: Vec<&String> = traits.iter().filter(|t| DERIVE_TRAITS.contains(&t.as_str())).collect();
    let mut out = String::new();
    // derive comes first so serde helper attrs can follow it
    if !derive_traits.is_empty() {
        out.push_str(&format!("#[derive({})]\n", derive_traits.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    for a in attrs {
        out.push_str(&format!("#[{a}]\n"));
    }
    out.push_str(&format!("enum {name} {{\n"));
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
        return if p.keep {
            "self".into()
        } else if matches!(p.ty, Ty::Ref(ref inner) if **inner == Ty::SelfTy) {
            "&self".into()      // & self → &self
        } else {
            "&mut self".into()  // self / mut self → &mut self
        };
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
            let inner = args.iter().map(|a| emit_ty_generic_arg(name, a)).collect::<Vec<_>>().join(", ");
            format!("{name}<{inner}>")
        }
        Ty::Tuple(elems) => format!("({})", elems.iter().map(emit_ty_owned).collect::<Vec<_>>().join(", ")),
        Ty::Ref(inner)   => format!("&{}", emit_ty_ref(inner)),
        Ty::SelfTy       => "Self".into(),
    }
}

/// Emit a type as a generic argument. `str` inside slice-like containers
/// (Vec, Option, Result, Box, …) becomes `&str`; inside map/set containers
/// it stays `String` so keys remain owned.
fn emit_ty_generic_arg(outer: &str, ty: &Ty) -> String {
    const BORROW_CONTAINERS: &[&str] = &["Vec", "Option", "Result", "Box", "Rc", "Arc", "Cow"];
    match ty {
        Ty::Simple(s) if s == "str" => {
            if BORROW_CONTAINERS.contains(&outer) { "&str".into() } else { "String".into() }
        }
        Ty::Generic(inner_name, inner_args) => {
            let inner = inner_args.iter().map(|a| emit_ty_generic_arg(inner_name, a)).collect::<Vec<_>>().join(", ");
            format!("{inner_name}<{inner}>")
        }
        Ty::Tuple(elems) => format!("({})", elems.iter().map(emit_ty_owned).collect::<Vec<_>>().join(", ")),
        _ => emit_ty_owned(ty),
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
        let mut top_level_comma = false;
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if depth == 0 && i == chars.len() - 1 { matched = true; }
                    else if depth == 0 { break; }
                }
                ',' if depth == 1 => { top_level_comma = true; }
                _ => {}
            }
        }
        // Don't strip if it's a tuple (top-level comma) — parens are structural
        if matched && !top_level_comma { return s[1..s.len()-1].to_string(); }
    }
    s
}

fn first_top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '"' => {
                // skip string literal
                let rest = &s[i + 1..];
                let end = rest.find('"').unwrap_or(rest.len());
                return first_top_level_comma(&s[i + 1 + end + 1..])
                    .map(|p| i + 1 + end + 1 + p);
            }
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
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
    if args.is_empty() {
        // No interpolation — emit raw string without brace escaping
        let escaped: String = s.chars().map(|c| match c {
            '"'  => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            '\n' => "\\n".to_string(),
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            c    => c.to_string(),
        }).collect();
        format!("\"{escaped}\"")
    } else {
        format!("format!(\"{fmt}\", {})", args.join(", "))
    }
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

/// Ensure all lifetimes used in a generic params string are declared at the top level.
/// e.g. `"D: Deserializer<'de>"` → `"'de, D: Deserializer<'de>"`
fn ensure_lifetimes_declared(generics: &str) -> String {
    if generics.is_empty() { return generics.to_string(); }
    // Split top-level comma-separated params (respecting angle bracket depth)
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in generics.chars() {
        match c {
            '<' => { depth += 1; current.push(c); }
            '>' => { depth -= 1; current.push(c); }
            ',' if depth == 0 => { segments.push(current.trim().to_string()); current = String::new(); }
            _ => { current.push(c); }
        }
    }
    if !current.trim().is_empty() { segments.push(current.trim().to_string()); }
    // Collect declared top-level lifetimes (segments starting with ')
    let declared: Vec<String> = segments.iter()
        .filter(|s| s.starts_with('\''))
        .map(|s| s.split_whitespace().next().unwrap_or(s).to_string())
        .collect();
    // Collect all lifetimes used in the full string
    let all_used = extract_lifetimes_from_str(generics);
    // Prepend any used but not declared
    let mut extra: Vec<String> = all_used.into_iter()
        .filter(|lt| !declared.contains(lt))
        .collect();
    if extra.is_empty() { return generics.to_string(); }
    extra.extend(segments);
    extra.join(", ")
}

/// Extract lifetime params from a string: `"de::Visitor<'de>"` → `["'de"]`
fn extract_lifetimes_from_str(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i > start + 1 {
                let lt: String = chars[start..i].iter().collect();
                if !result.contains(&lt) {
                    result.push(lt);
                }
            }
        } else {
            i += 1;
        }
    }
    result
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
    fn vec_str_becomes_vec_ref_str() {
        let out = transpile("fn main()\n    let v: Vec<str> = vec![]");
        assert!(out.contains("Vec<&str>"), "got: {out}");
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
        assert!(out.contains("let mut lines: Vec<&str> = Default::default();"), "got: {out}");
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

    // ── move closures ─────────────────────────────────────────────────────────

    #[test]
    fn move_zero_arg_closure() {
        let out = transpile("fn main()\n    let f = move -> 42");
        assert!(out.contains("move || 42"), "got: {out}");
    }

    #[test]
    fn move_closure_with_param() {
        let out = transpile("fn main()\n    let f = move x -> x + 1");
        assert!(out.contains("move |x"), "got: {out}");
        assert!(out.contains("x + 1"), "got: {out}");
    }

    #[test]
    fn zero_arg_closure_arrow() {
        let out = transpile("fn main()\n    let f = -> 99");
        assert!(out.contains("|| 99"), "got: {out}");
    }

    #[test]
    fn multi_line_closure_body() {
        let out = transpile("fn main()\n    let f = ->\n        let x = 1\n        x");
        assert!(out.contains("|| {"), "got: {out}");
        assert!(out.contains("let x = 1"), "got: {out}");
    }

    // ── if let ────────────────────────────────────────────────────────────────

    #[test]
    fn if_let_some() {
        let out = transpile("fn main()\n    let v = 1\n    if let Some(n) = v\n        println!(\"{n}\")");
        assert!(out.contains("if let Some(n) = v"), "got: {out}");
    }

    #[test]
    fn if_let_with_else() {
        let out = transpile("fn main()\n    let v = 1\n    if let Some(n) = v\n        println!(\"{n}\")\n    else\n        println!(\"none\")");
        assert!(out.contains("if let Some(n) = v"), "got: {out}");
        assert!(out.contains("else"), "got: {out}");
    }

    // ── struct spread ─────────────────────────────────────────────────────────

    #[test]
    fn struct_spread_inline() {
        let out = transpile("struct P\n    x: i32\n    y: i32\nfn main()\n    let base = P { x: 1, y: 2 }\n    let p = P { x: 9, ..base }");
        assert!(out.contains("..base"), "got: {out}");
        assert!(out.contains("x: 9"), "got: {out}");
    }

    #[test]
    fn struct_spread_indented() {
        let out = transpile("struct P\n    x: i32\n    y: i32\nfn main()\n    let base = P { x: 1, y: 2 }\n    let p = P\n        x: 9\n        ..base");
        assert!(out.contains("..base"), "got: {out}");
    }

    // ── vec spread ────────────────────────────────────────────────────────────

    #[test]
    fn vec_no_spread_plain_macro() {
        let out = transpile("fn main()\n    let v = vec![1, 2, 3]");
        assert!(out.contains("vec![1, 2, 3]"), "got: {out}");
    }

    #[test]
    fn vec_spread_emits_extend_block() {
        let out = transpile("fn main()\n    let a = vec![1]\n    let b = vec![..a, 2]");
        assert!(out.contains("__v"), "got: {out}");
        assert!(out.contains("extend"), "got: {out}");
    }
}
