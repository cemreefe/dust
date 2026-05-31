pub mod ast;
pub use ast::*;

use crate::error::{DustError, Result};
use crate::lexer::{Spanned, Token};

pub fn parse(tokens: &[Spanned<Token>]) -> Result<Vec<Item>> {
    let mut p = Parser::new(tokens);
    p.parse_program()
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
        Ok(Param { keep, name, ty, line, col })
    }

    // ── Types ─────────────────────────────────────────────────────────────────

    fn parse_ty(&mut self) -> Result<Ty> {
        match self.peek().clone() {
            Token::KwSelf => { self.advance(); Ok(Ty::SelfTy) }
            Token::Ident(name) => {
                self.advance();
                if self.eat(&Token::Lt) {
                    let mut args = Vec::new();
                    loop {
                        args.push(self.parse_ty()?);
                        if !self.eat(&Token::Comma) { break; }
                    }
                    self.expect(&Token::Gt)?;
                    Ok(Ty::Generic(name, args))
                } else {
                    Ok(Ty::Simple(name))
                }
            }
            tok => Err(DustError::new(format!("expected type, found {:?}", tok), self.line(), self.col())),
        }
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
                // Check for assignment: expr = rhs
                if self.eat(&Token::Eq) {
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
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            let line = self.line(); let col = self.col();
            if self.eat(&Token::Dot) {
                // Could be field access, method call, or .unwrap!
                match self.peek().clone() {
                    Token::Ident(s) if s.ends_with("!()") || s == "unwrap!()" => {
                        // .unwrap!
                        self.advance();
                        expr = Expr::Unwrap(Box::new(expr), line, col);
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
                // index
                let idx = self.parse_expr()?;
                self.expect(&Token::RBracket)?;
                expr = Expr::Call {
                    func: Box::new(Expr::FieldAccess {
                        obj: Box::new(expr),
                        field: "__index__".into(),
                        line,
                        col,
                    }),
                    args: vec![idx],
                    line,
                    col,
                };
            } else if matches!(self.peek(), Token::ColonColon) {
                // path continuation
                self.advance();
                let seg = self.expect_ident()?;
                expr = match expr {
                    Expr::Ident { name, .. } => Expr::Path { segments: vec![name, seg], line, col },
                    Expr::Path { mut segments, .. } => { segments.push(seg); Expr::Path { segments, line, col } }
                    _ => return Err(DustError::new("invalid path", line, col)),
                };
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
        match self.peek().clone() {
            Token::Int(n)  => { self.advance(); Ok(Expr::Int(n)) }
            Token::Float(f) => { self.advance(); Ok(Expr::Float(f)) }
            Token::Str(s)  => { self.advance(); Ok(Expr::Str(s)) }
            Token::Bool(b) => { self.advance(); Ok(Expr::Bool(b)) }

            // Macro passthrough: stored as Ident("name!(...)")
            Token::Ident(s) if s.contains('!') => {
                self.advance();
                Ok(Expr::Macro { raw: s, line, col })
            }

            Token::Ident(name) => {
                self.advance();
                // Struct literal: Name { ... }
                if self.eat(&Token::LBrace) {
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

            Token::KwIf => {
                self.advance();
                let cond = Box::new(self.parse_expr()?);
                // `then` keyword or indented block
                let then_branch = if self.eat(&Token::KwThen) {
                    Box::new(self.parse_expr()?)
                } else {
                    let stmts = self.parse_block()?;
                    Box::new(Expr::Block { stmts, line, col })
                };
                self.skip_newlines();
                let else_branch = if self.eat(&Token::KwElse) {
                    if matches!(self.peek(), Token::Indent) {
                        let stmts = self.parse_block()?;
                        Some(Box::new(Expr::Block { stmts, line, col }))
                    } else {
                        Some(Box::new(self.parse_expr()?))
                    }
                } else { None };
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
                    let body = self.parse_expr()?;
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
        // Patterns look like expressions for our purposes
        let line = self.line(); let col = self.col();
        let name = self.expect_ident()?;
        if self.eat(&Token::LParen) {
            let mut args = Vec::new();
            while !matches!(self.peek(), Token::RParen | Token::Eof) {
                args.push(self.parse_pattern()?);
                if !self.eat(&Token::Comma) { break; }
            }
            self.expect(&Token::RParen)?;
            Ok(Expr::Call {
                func: Box::new(Expr::Ident { name, line, col }),
                args,
                line,
                col,
            })
        } else {
            Ok(Expr::Ident { name, line, col })
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
}

enum BindKind { Let, Const, Mut }
