pub mod ast;
pub use ast::*;

use crate::error::{DustError, Result};
use crate::lexer::{Spanned, Token};

pub fn parse(tokens: &[Spanned<Token>]) -> Result<Vec<Item>> {
    let mut p = Parser::new(tokens);
    p.parse_program()
}

pub fn parse_expr_str(src: &str) -> Option<Expr> {
    let tokens = crate::lexer::lex(src).ok()?;
    let mut p = Parser::new(&tokens);
    p.parse_expr().ok()
}

struct Parser<'a> {
    tokens: &'a [Spanned<Token>],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Spanned<Token>]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).map(|s| &s.value).unwrap_or(&Token::Eof)
    }

    fn peek_at(&self, offset: usize) -> &Token {
        self.tokens.get(self.pos + offset).map(|s| &s.value).unwrap_or(&Token::Eof)
    }

    fn peek_spanned(&self) -> &Spanned<Token> {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn line(&self) -> usize { self.peek_spanned().line }
    fn col(&self)  -> usize { self.peek_spanned().col  }

    fn advance(&mut self) -> &Spanned<Token> {
        let s = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() { self.pos += 1; }
        s
    }

    fn expect(&mut self, tok: &Token) -> Result<()> {
        if self.peek() == tok {
            self.advance();
            Ok(())
        } else {
            Err(DustError::new(
                format!("expected {:?}, found {:?}", tok, self.peek()),
                self.line(), self.col(),
            ))
        }
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.peek() == tok { self.advance(); true } else { false }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) { self.advance(); }
    }

    // ── Program ──────────────────────────────────────────────────────────────

    fn parse_program(&mut self) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Eof) { break; }
            items.push(self.parse_item()?);
        }
        Ok(items)
    }

    // ── Items ─────────────────────────────────────────────────────────────────

    fn parse_item(&mut self) -> Result<Item> {
        let line = self.line();
        let col  = self.col();
        match self.peek().clone() {
            Token::KwAsync => {
                self.advance();
                self.expect(&Token::KwFn)?;
                self.parse_fn(true, line, col)
            }
            Token::KwFn     => { self.advance(); self.parse_fn(false, line, col) }
            Token::KwStruct => { self.advance(); self.parse_struct(line, col) }
            Token::KwTrait  => { self.advance(); self.parse_trait(line, col) }
            Token::KwEnum   => { self.advance(); self.parse_enum(line, col) }
            Token::KwUse    => { self.advance(); self.parse_use(line, col) }
            tok => Err(DustError::new(
                format!("unexpected token at top level: {:?}", tok),
                line, col,
            )),
        }
    }

    fn parse_fn(&mut self, is_async: bool, line: usize, col: usize) -> Result<Item> {
        let name = self.expect_ident()?;
        let params = self.parse_param_list()?;
        let ret_ty = if self.eat(&Token::Arrow) { Some(self.parse_ty()?) } else { None };
        let body = self.parse_block()?;
        Ok(Item::Fn { name, is_async, params, ret_ty, body, line, col })
    }

    fn parse_struct(&mut self, line: usize, col: usize) -> Result<Item> {
        let name = self.expect_ident()?;
        let traits = if self.eat(&Token::KwIs) {
            self.parse_comma_separated_idents()?
        } else { vec![] };
        self.skip_newlines();
        self.expect(&Token::Indent)?;
        self.skip_newlines();

        let mut fields  = Vec::new();
        let mut methods = Vec::new();

        while !matches!(self.peek(), Token::Dedent | Token::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) { break; }
            let fl = self.line(); let fc = self.col();
            match self.peek().clone() {
                Token::KwAsync | Token::KwFn => {
                    let is_async = if self.eat(&Token::KwAsync) { self.expect(&Token::KwFn)?; true } else { self.advance(); false };
                    methods.push(self.parse_method(is_async, fl, fc)?);
                }
                Token::Ident(_) => {
                    // field: name: Type
                    let fname = self.expect_ident()?;
                    self.expect(&Token::Colon)?;
                    let fty = self.parse_ty()?;
                    fields.push(Field { name: fname, ty: fty, line: fl, col: fc });
                    self.skip_newlines();
                }
                _ => { self.advance(); }
            }
        }
        self.expect(&Token::Dedent)?;
        Ok(Item::Struct { name, traits, fields, methods, line, col })
    }

    fn parse_method(&mut self, is_async: bool, line: usize, col: usize) -> Result<Method> {
        // name may be qualified: TraitName.method_name
        let first = self.expect_ident()?;
        let (trait_qualifier, name) = if self.eat(&Token::Dot) {
            let method_name = self.expect_ident()?;
            (Some(first), method_name)
        } else {
            (None, first)
        };

        let params = self.parse_param_list()?;
        let ret_ty = if self.eat(&Token::Arrow) { Some(self.parse_ty()?) } else { None };
        let body = Some(self.parse_block()?);
        Ok(Method { trait_qualifier, name, is_async, params, ret_ty, body, line, col })
    }

    fn parse_trait(&mut self, line: usize, col: usize) -> Result<Item> {
        let name = self.expect_ident()?;
        self.skip_newlines();
        self.expect(&Token::Indent)?;
        self.skip_newlines();

        let mut methods = Vec::new();
        while !matches!(self.peek(), Token::Dedent | Token::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) { break; }
            let ml = self.line(); let mc = self.col();
            let is_async = if self.eat(&Token::KwAsync) { self.expect(&Token::KwFn)?; true } else { self.expect(&Token::KwFn)?; false };
            let mname = self.expect_ident()?;
            let params = self.parse_param_list()?;
            let ret_ty = if self.eat(&Token::Arrow) { Some(self.parse_ty()?) } else { None };
            self.skip_newlines();
            methods.push(Method {
                trait_qualifier: None,
                name: mname,
                is_async,
                params,
                ret_ty,
                body: None,
                line: ml,
                col: mc,
            });
        }
        self.expect(&Token::Dedent)?;
        Ok(Item::Trait { name, methods, line, col })
    }

    fn parse_enum(&mut self, line: usize, col: usize) -> Result<Item> {
        let name = self.expect_ident()?;
        self.skip_newlines();
        self.expect(&Token::Indent)?;
        self.skip_newlines();

        let mut variants = Vec::new();
        while !matches!(self.peek(), Token::Dedent | Token::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), Token::Dedent | Token::Eof) { break; }
            let vl = self.line(); let vc = self.col();
            let vname = self.expect_ident()?;
            let mut fields = Vec::new();
            if self.eat(&Token::LParen) {
                loop {
                    fields.push(self.parse_ty()?);
                    if !self.eat(&Token::Comma) { break; }
                }
                self.expect(&Token::RParen)?;
            }
            variants.push(Variant { name: vname, fields, line: vl, col: vc });
            self.skip_newlines();
        }
        self.expect(&Token::Dedent)?;
        Ok(Item::Enum { name, variants, line, col })
    }

    fn parse_use(&mut self, line: usize, col: usize) -> Result<Item> {
        let mut path = String::new();
        loop {
            match self.peek().clone() {
                Token::Ident(s) => { path.push_str(&s); self.advance(); }
                Token::ColonColon => { path.push_str("::"); self.advance(); }
                Token::Star => { path.push('*'); self.advance(); break; }
                _ => break,
            }
        }
        self.skip_newlines();
        Ok(Item::Use { path, line, col })
    }

    // ── Params ────────────────────────────────────────────────────────────────

    fn parse_param_list(&mut self) -> Result<Vec<Param>> {
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            params.push(self.parse_param()?);
            if !self.eat(&Token::Comma) { break; }
        }
        self.expect(&Token::RParen)?;
        Ok(params)
    }

    fn parse_param(&mut self) -> Result<Param> {
        let line = self.line(); let col = self.col();
        let keep = self.eat(&Token::KwKeep);
        let mutable = keep && self.eat(&Token::KwMut);
        let name = if matches!(self.peek(), Token::KwSelf) {
            self.advance(); "self".to_string()
        } else {
            self.expect_ident()?
        };
        let ty = if self.eat(&Token::Colon) {
            self.parse_ty()?
        } else if name == "self" {
            Ty::SelfTy
        } else {
            return Err(DustError::new(format!("expected ':' after param '{name}'"), line, col));
        };
        Ok(Param { keep, mutable, name, ty, line, col })
    }

    // ── Types ─────────────────────────────────────────────────────────────────

    fn parse_ty(&mut self) -> Result<Ty> {
        // Optional leading &
        let is_ref = self.eat(&Token::Ampersand);
        let ty = match self.peek().clone() {
            Token::KwSelf => { self.advance(); Ty::SelfTy }
            Token::LParen => {
                self.advance();
                let mut elems = Vec::new();
                while !matches!(self.peek(), Token::RParen | Token::Eof) {
                    elems.push(self.parse_ty()?);
                    if !self.eat(&Token::Comma) { break; }
                }
                self.expect(&Token::RParen)?;
                Ty::Tuple(elems)
            }
            Token::Ident(name) => {
                self.advance();
                if self.eat(&Token::Lt) {
                    let mut args = Vec::new();
                    loop {
                        if matches!(self.peek(), Token::Gt) { break; }
                        args.push(self.parse_ty()?);
                        if !self.eat(&Token::Comma) { break; }
                    }
                    self.expect(&Token::Gt)?;
                    Ty::Generic(name, args)
                } else {
                    Ty::Simple(name)
                }
            }
            tok => return Err(DustError::new(format!("expected type, found {:?}", tok), self.line(), self.col())),
        };
        if is_ref { Ok(Ty::Ref(Box::new(ty))) } else { Ok(ty) }
    }

    // ── Block & Statements ────────────────────────────────────────────────────

    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.skip_newlines();
        self.expect(&Token::Indent)?;
        self.skip_newlines();
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Token::Dedent | Token::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.expect(&Token::Dedent)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt> {
        let line = self.line(); let col = self.col();
        match self.peek().clone() {
            Token::KwLet   => { self.advance(); self.parse_binding(BindKind::Let,   line, col) }
            Token::KwConst => { self.advance(); self.parse_binding(BindKind::Const, line, col) }
            Token::KwMut   => { self.advance(); self.parse_binding(BindKind::Mut,   line, col) }
            Token::KwReturn => {
                self.advance();
                let val = if matches!(self.peek(), Token::Newline | Token::Dedent | Token::Eof) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.skip_newlines();
                Ok(Stmt::Return(val, line, col))
            }
            Token::KwBreak => {
                self.advance(); self.skip_newlines();
                Ok(Stmt::Break(line, col))
            }
            Token::KwContinue => {
                self.advance(); self.skip_newlines();
                Ok(Stmt::Continue(line, col))
            }
            Token::KwFor => {
                self.advance();
                let vars = if self.eat(&Token::LParen) {
                    let mut names = Vec::new();
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        names.push(self.expect_ident()?);
                        if !self.eat(&Token::Comma) { break; }
                    }
                    self.expect(&Token::RParen)?;
                    names
                } else {
                    vec![self.expect_ident()?]
                };
                self.expect(&Token::KwIn)?;
                let iter = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::For { vars, iter, body, line, col })
            }
            Token::KwTry => {
                self.advance();
                let try_block = self.parse_block()?;
                self.skip_newlines();
                self.expect(&Token::KwCatch)?;
                let catch_var = self.expect_ident()?;
                let catch_block = self.parse_block()?;
                Ok(Stmt::TryCatch { try_block, catch_var, catch_block, line, col })
            }
            Token::KwUse => {
                self.advance();
                if let Item::Use { path, line, col } = self.parse_use(line, col)? {
                    Ok(Stmt::Use { path, line, col })
                } else { unreachable!() }
            }
            _ => {
                let expr = self.parse_expr()?;
                // Check for assignment / compound assignment / increment
                let compound_op = match self.peek() {
                    Token::PlusEq      => Some("+"),
                    Token::MinusEq     => Some("-"),
                    Token::StarEq      => Some("*"),
                    Token::SlashEq     => Some("/"),
                    Token::AndAndEq    => Some("&&"),
                    Token::PipePipeEq  => Some("||"),
                    _ => None,
                };
                if let Some(op) = compound_op {
                    let op = op.to_string();
                    self.advance();
                    let value = self.parse_expr()?;
                    self.skip_newlines();
                    Ok(Stmt::CompoundAssign { target: expr, op, value, line, col })
                } else if matches!(self.peek(), Token::PlusPlus) {
                    self.advance();
                    self.skip_newlines();
                    Ok(Stmt::CompoundAssign { target: expr, op: "+".into(), value: Expr::Int(1), line, col })
                } else if matches!(self.peek(), Token::MinusMinus) {
                    self.advance();
                    self.skip_newlines();
                    Ok(Stmt::CompoundAssign { target: expr, op: "-".into(), value: Expr::Int(1), line, col })
                } else if self.eat(&Token::Eq) {
                    let value = self.parse_expr()?;
                    self.skip_newlines();
                    Ok(Stmt::Assign { target: expr, value, line, col })
                } else {
                    self.skip_newlines();
                    Ok(Stmt::Expr(expr))
                }
            }
        }
    }

    fn parse_binding(&mut self, kind: BindKind, line: usize, col: usize) -> Result<Stmt> {
        let name = self.expect_ident()?;
        let ty = if self.eat(&Token::Colon) { Some(self.parse_ty()?) } else { None };
        self.expect(&Token::Eq)?;
        let value = self.parse_expr()?;
        self.skip_newlines();
        Ok(match kind {
            BindKind::Let   => Stmt::Let   { name, ty, value, line, col },
            BindKind::Const => Stmt::Const { name, ty, value, line, col },
            BindKind::Mut   => Stmt::Mut   { name, ty, value, line, col },
        })
    }

    // ── Expressions ───────────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while self.eat(&Token::PipePipe) {
            let line = self.line(); let col = self.col();
            let right = self.parse_and()?;
            left = Expr::BinOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right), line, col };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_cmp()?;
        while self.eat(&Token::AndAnd) {
            let line = self.line(); let col = self.col();
            let right = self.parse_cmp()?;
            left = Expr::BinOp { op: BinOp::And, left: Box::new(left), right: Box::new(right), line, col };
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<Expr> {
        let mut left = self.parse_add()?;
        loop {
            let line = self.line(); let col = self.col();
            let op = match self.peek() {
                Token::EqEq  => BinOp::Eq,
                Token::BangEq => BinOp::NotEq,
                Token::Lt    => BinOp::Lt,
                Token::Gt    => BinOp::Gt,
                Token::LtEq  => BinOp::LtEq,
                Token::GtEq  => BinOp::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_add()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), line, col };
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr> {
        let mut left = self.parse_mul()?;
        loop {
            let line = self.line(); let col = self.col();
            let op = match self.peek() {
                Token::Plus  => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_mul()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), line, col };
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let line = self.line(); let col = self.col();
            let op = match self.peek() {
                Token::Star    => BinOp::Mul,
                Token::Slash   => BinOp::Div,
                Token::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), line, col };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        let line = self.line(); let col = self.col();
        if self.eat(&Token::Minus) {
            return Ok(Expr::UnaryOp { op: UnaryOp::Neg, expr: Box::new(self.parse_postfix()?), line, col });
        }
        if self.eat(&Token::Bang) {
            return Ok(Expr::UnaryOp { op: UnaryOp::Not, expr: Box::new(self.parse_postfix()?), line, col });
        }
        if self.eat(&Token::KwAwait) {
            let e = self.parse_postfix()?;
            return Ok(Expr::Await(Box::new(e), line, col));
        }
        if self.eat(&Token::Ampersand) {
            let is_mut = self.eat(&Token::KwMut);
            let op = if is_mut { UnaryOp::RefMut } else { UnaryOp::Ref };
            return Ok(Expr::UnaryOp { op, expr: Box::new(self.parse_postfix()?), line, col });
        }
        if self.eat(&Token::Star) {
            return Ok(Expr::UnaryOp { op: UnaryOp::Deref, expr: Box::new(self.parse_postfix()?), line, col });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            let line = self.line(); let col = self.col();
            if self.eat(&Token::Dot) {
                // Could be field access, method call, or .unwrap!
                match self.peek().clone() {
                    Token::Ident(s) if s == "unwrap!" || s == "unwrap!()" => {
                        self.advance();
                        expr = Expr::Unwrap(Box::new(expr), line, col);
                    }
                    Token::Int(n) => {
                        // tuple field access: expr.0, expr.1, etc.
                        self.advance();
                        expr = Expr::FieldAccess { obj: Box::new(expr), field: n.to_string(), line, col };
                    }
                    Token::Ident(field) => {
                        self.advance();
                        if self.eat(&Token::LParen) {
                            // method call
                            let args = self.parse_arg_list()?;
                            expr = Expr::Call {
                                func: Box::new(Expr::FieldAccess {
                                    obj: Box::new(expr),
                                    field,
                                    line,
                                    col,
                                }),
                                args,
                                line,
                                col,
                            };
                        } else {
                            expr = Expr::FieldAccess { obj: Box::new(expr), field, line, col };
                        }
                    }
                    // .unwrap! stored as macro token by lexer
                    _ => {
                        expr = Expr::FieldAccess {
                            obj: Box::new(expr),
                            field: "?".into(),
                            line,
                            col,
                        };
                    }
                }
            } else if self.eat(&Token::Question) {
                expr = Expr::Try(Box::new(expr), line, col);
            } else if self.eat(&Token::LParen) {
                // function call
                let args = self.parse_arg_list()?;
                expr = Expr::Call { func: Box::new(expr), args, line, col };
            } else if self.eat(&Token::LBracket) {
                let idx = self.parse_expr()?;
                self.expect(&Token::RBracket)?;
                expr = Expr::Index { obj: Box::new(expr), idx: Box::new(idx), line, col };
            } else if matches!(self.peek(), Token::ColonColon) {
                self.advance();
                // Turbofish: ::<Type, ...>
                if self.eat(&Token::Lt) {
                    let type_args = self.parse_turbofish_args()?;
                    expr = Expr::Turbofish { inner: Box::new(expr), type_args, line, col };
                } else {
                    // Regular path continuation
                    let seg = self.expect_ident()?;
                    expr = match expr {
                        Expr::Ident { name, .. } => Expr::Path { segments: vec![name, seg], line, col },
                        Expr::Path { mut segments, .. } => { segments.push(seg); Expr::Path { segments, line, col } }
                        _ => return Err(DustError::new("invalid path", line, col)),
                    };
                }
            } else if self.eat(&Token::KwAs) {
                let ty = self.parse_ty()?;
                expr = Expr::Cast { expr: Box::new(expr), ty, line, col };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            args.push(self.parse_expr()?);
            if !self.eat(&Token::Comma) { break; }
        }
        self.expect(&Token::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let line = self.line(); let col = self.col();

        // Closure detection: ident (: type)? (, ident (: type)?)* ->
        if self.is_closure_start() {
            return self.parse_closure(line, col);
        }

        match self.peek().clone() {
            Token::Int(n)  => { self.advance(); Ok(Expr::Int(n)) }
            Token::Float(f) => { self.advance(); Ok(Expr::Float(f)) }
            Token::Str(s)  => { self.advance(); Ok(Expr::Str(s)) }
            Token::Char(c) => { self.advance(); Ok(Expr::Char(c)) }
            Token::Bool(b) => { self.advance(); Ok(Expr::Bool(b)) }

            // Macro passthrough: stored as Ident("name!(...)")
            Token::Ident(s) if s.contains('!') => {
                self.advance();
                Ok(Expr::Macro { raw: s, line, col })
            }

            Token::Ident(name) => {
                self.advance();
                // Indented struct literal:
                //   Name
                //     field: value
                //     field: value
                if matches!(self.peek(), Token::Newline)
                    && matches!(self.peek_at(1), Token::Indent)
                    && matches!(self.peek_at(2), Token::Ident(_))
                    && matches!(self.peek_at(3), Token::Colon)
                {
                    self.advance(); // Newline
                    self.advance(); // Indent
                    let mut fields = Vec::new();
                    while !matches!(self.peek(), Token::Dedent | Token::Eof) {
                        self.skip_newlines();
                        if matches!(self.peek(), Token::Dedent) { break; }
                        let fname = self.expect_ident()?;
                        self.expect(&Token::Colon)?;
                        let fval = self.parse_expr()?;
                        fields.push((fname, fval));
                        self.skip_newlines();
                    }
                    self.expect(&Token::Dedent)?;
                    Ok(Expr::StructLit { name, fields, line, col })
                // Inline struct literal: Name { field: value, ... }
                } else if self.eat(&Token::LBrace) {
                    let mut fields = Vec::new();
                    while !matches!(self.peek(), Token::RBrace | Token::Eof) {
                        let fname = self.expect_ident()?;
                        self.expect(&Token::Colon)?;
                        let fval = self.parse_expr()?;
                        fields.push((fname, fval));
                        self.eat(&Token::Comma);
                    }
                    self.expect(&Token::RBrace)?;
                    Ok(Expr::StructLit { name, fields, line, col })
                } else {
                    Ok(Expr::Ident { name, line, col })
                }
            }

            Token::KwSelf => {
                self.advance();
                Ok(Expr::Ident { name: "self".into(), line, col })
            }

            Token::KwBreak    => { self.advance(); Ok(Expr::Ident { name: "break".into(), line, col }) }
            Token::KwContinue => { self.advance(); Ok(Expr::Ident { name: "continue".into(), line, col }) }
            Token::KwReturn   => {
                self.advance();
                // `return` as an expression (e.g. in match arms)
                let val = if matches!(self.peek(), Token::Newline | Token::Dedent | Token::Eof | Token::Comma | Token::RParen) {
                    None
                } else {
                    Some(Box::new(self.parse_expr()?))
                };
                Ok(Expr::Return(val, line, col))
            }

            Token::KwIf | Token::KwElif => {
                self.advance();
                let cond = Box::new(self.parse_expr()?);

                let (then_branch, else_branch) = if self.eat(&Token::KwThen) {
                    // Same-line: if cond then expr else expr
                    let t = Box::new(self.parse_expr()?);
                    self.skip_newlines();
                    let e = if self.eat(&Token::KwElse) {
                        Some(Box::new(self.parse_expr()?))
                    } else { None };
                    (t, e)
                } else {
                    // Multi-line block: check if block starts with `then`
                    self.skip_newlines();
                    if matches!(self.peek(), Token::Indent) {
                        // Peek inside the indent
                        let saved = self.pos;
                        self.advance(); // consume Indent
                        self.skip_newlines();
                        if self.eat(&Token::KwThen) {
                            // Indented `then expr / else expr` form
                            let t = Box::new(self.parse_expr()?);
                            self.skip_newlines();
                            let e = if self.eat(&Token::KwElse) {
                                self.skip_newlines();
                                Some(Box::new(self.parse_expr()?))
                            } else { None };
                            self.skip_newlines();
                            self.expect(&Token::Dedent)?;
                            (t, e)
                        } else {
                            self.pos = saved;
                            // Regular block
                            let stmts = self.parse_block()?;
                            let bl = line; let bc = col;
                            self.skip_newlines();
                                            let e = if matches!(self.peek(), Token::KwElif) {
                                // `elif cond` desugars to `else if cond` — parse as new if expr
                                Some(Box::new(self.parse_expr()?))
                            } else if self.eat(&Token::KwElse) {
                                self.skip_newlines();
                                if matches!(self.peek(), Token::KwIf | Token::KwElif) {
                                    // else if / elif → parse another if expr
                                    Some(Box::new(self.parse_expr()?))
                                } else if matches!(self.peek(), Token::Indent) {
                                    let es = self.parse_block()?;
                                    Some(Box::new(Expr::Block { stmts: es, line: bl, col: bc }))
                                } else {
                                    Some(Box::new(self.parse_expr()?))
                                }
                            } else { None };
                            (Box::new(Expr::Block { stmts, line: bl, col: bc }), e)
                        }
                    } else {
                        return Err(DustError::new("expected `then` or indented block after `if`", self.line(), self.col()));
                    }
                };

                Ok(Expr::If { cond, then_branch, else_branch, line, col })
            }

            Token::KwMatch => {
                self.advance();
                let scrutinee = Box::new(self.parse_expr()?);
                self.skip_newlines();
                self.expect(&Token::Indent)?;
                self.skip_newlines();
                let mut arms = Vec::new();
                while !matches!(self.peek(), Token::Dedent | Token::Eof) {
                    let pattern = self.parse_pattern()?;
                    self.expect(&Token::Arrow)?;
                    let body = if matches!(self.peek(), Token::Newline) {
                        // Multi-line arm: indented block → wrap in Block expr
                        let stmts = self.parse_block()?;
                        let bl = self.line(); let bc = self.col();
                        Expr::Block { stmts, line: bl, col: bc }
                    } else {
                        self.parse_expr()?
                    };
                    arms.push(MatchArm { pattern, body });
                    self.skip_newlines();
                }
                self.expect(&Token::Dedent)?;
                Ok(Expr::Match { scrutinee, arms, line, col })
            }

            Token::LParen => {
                self.advance();
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    return Ok(Expr::Block { stmts: vec![], line, col });
                }
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }

            tok => Err(DustError::new(format!("unexpected token in expression: {:?}", tok), line, col)),
        }
    }

    fn parse_pattern(&mut self) -> Result<Expr> {
        let line = self.line(); let col = self.col();
        match self.peek().clone() {
            // Literal patterns
            Token::Str(s)   => { self.advance(); Ok(Expr::Str(s)) }
            Token::Int(n)   => { self.advance(); Ok(Expr::Int(n)) }
            Token::Float(f) => { self.advance(); Ok(Expr::Float(f)) }
            Token::Bool(b)  => { self.advance(); Ok(Expr::Bool(b)) }
            Token::Char(c)  => { self.advance(); Ok(Expr::Char(c)) }
            // Negative number: -n
            Token::Minus => {
                self.advance();
                match self.peek().clone() {
                    Token::Int(n)   => { self.advance(); Ok(Expr::Int(-n)) }
                    Token::Float(f) => { self.advance(); Ok(Expr::Float(-f)) }
                    tok => Err(DustError::new(format!("expected number after '-' in pattern, found {:?}", tok), line, col)),
                }
            }
            // Ident, enum variant, wildcard
            _ => {
                let name = self.expect_ident()?;
                if self.eat(&Token::LParen) {
                    let mut args = Vec::new();
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        args.push(self.parse_pattern()?);
                        if !self.eat(&Token::Comma) { break; }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call { func: Box::new(Expr::Ident { name, line, col }), args, line, col })
                } else {
                    Ok(Expr::Ident { name, line, col })
                }
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn expect_ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            Token::Ident(s) => { self.advance(); Ok(s) }
            tok => Err(DustError::new(format!("expected identifier, found {:?}", tok), self.line(), self.col())),
        }
    }

    fn parse_comma_separated_idents(&mut self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        names.push(self.expect_ident()?);
        while self.eat(&Token::Comma) {
            names.push(self.expect_ident()?);
        }
        Ok(names)
    }

    // ── Turbofish ─────────────────────────────────────────────────────────────
    // Called after consuming `<` in `::<`
    fn parse_turbofish_args(&mut self) -> Result<String> {
        let mut s = String::new();
        let mut depth = 1usize;
        loop {
            match self.peek().clone() {
                Token::Lt => { depth += 1; s.push('<'); self.advance(); }
                Token::Gt => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 { break; }
                    s.push('>');
                }
                Token::Eof => break,
                Token::Ident(name) => { s.push_str(&name); self.advance(); }
                Token::Comma    => { s.push_str(", "); self.advance(); }
                Token::Ampersand => { s.push('&'); self.advance(); }
                _ => { self.advance(); } // skip unknown tokens inside turbofish
            }
        }
        Ok(s)
    }

    // ── Closure detection & parsing ───────────────────────────────────────────

    /// Peek ahead to see if the current position starts a closure:
    /// `ident (: type)? (, ident (: type)?)* ->`
    fn is_closure_start(&self) -> bool {
        let mut i = self.pos;
        // Must start with ident
        if !matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Ident(_))) {
            return false;
        }
        i += 1;
        // Optional `: type`
        if matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Colon)) {
            i += 1;
            // skip type tokens: ident, optional <...>
            i = self.skip_ty_tokens(i);
        }
        // Optional additional params
        while matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Comma)) {
            i += 1;
            if !matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Ident(_))) {
                return false;
            }
            i += 1;
            if matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Colon)) {
                i += 1;
                i = self.skip_ty_tokens(i);
            }
        }
        // Must end with ->
        matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Arrow))
    }

    fn skip_ty_tokens(&self, mut i: usize) -> usize {
        // Skip optional &
        if matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Ampersand)) { i += 1; }
        // Skip ident
        if matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Ident(_))) {
            i += 1;
            // Skip optional <...>
            if matches!(self.tokens.get(i).map(|s| &s.value), Some(Token::Lt)) {
                i += 1;
                let mut depth = 1usize;
                while i < self.tokens.len() && depth > 0 {
                    match &self.tokens[i].value {
                        Token::Lt => { depth += 1; i += 1; }
                        Token::Gt => { depth -= 1; i += 1; }
                        _ => { i += 1; }
                    }
                }
            }
        }
        i
    }

    fn parse_closure(&mut self, line: usize, col: usize) -> Result<Expr> {
        let mut params = Vec::new();
        loop {
            let pl = self.line(); let pc = self.col();
            let name = self.expect_ident()?;
            let ty = if self.eat(&Token::Colon) { self.parse_ty()? } else { Ty::Simple("_".into()) };
            params.push(Param { keep: false, mutable: false, name, ty, line: pl, col: pc });
            if !self.eat(&Token::Comma) { break; }
        }
        self.expect(&Token::Arrow)?;
        let body = Box::new(self.parse_expr()?);
        Ok(Expr::Closure { params, body, line, col })
    }
}

enum BindKind { Let, Const, Mut }
