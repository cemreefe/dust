pub mod token;
pub use token::Token;

use crate::error::{DustError, Result};

#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub value: T,
    pub line: usize,
    pub col: usize,
}

impl<T> Spanned<T> {
    pub fn new(value: T, line: usize, col: usize) -> Self {
        Self { value, line, col }
    }
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Self { chars: src.chars().collect(), pos: 0, line: 1, col: 1 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' { self.line += 1; self.col = 1; } else { self.col += 1; }
        Some(ch)
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t')) {
            self.advance();
        }
    }

    fn lex_char(&mut self, line: usize, col: usize) -> Result<Token> {
        let ch = match self.advance() {
            None => return Err(DustError::new("unterminated char literal", line, col)),
            Some('\\') => match self.advance() {
                Some('n')  => '\n',
                Some('r')  => '\r',
                Some('t')  => '\t',
                Some('\'') => '\'',
                Some('\\') => '\\',
                Some('0')  => '\0',
                _ => return Err(DustError::new("invalid char escape", line, col)),
            },
            Some(c) => c,
        };
        match self.advance() {
            Some('\'') => Ok(Token::Char(ch)),
            _ => Err(DustError::new("unterminated char literal", line, col)),
        }
    }

    /// `'...'` — single-line raw string. `"` allowed freely, no interpolation.
    /// Called after the opening `'` has been consumed.
    fn lex_single_quoted_str(&mut self, line: usize, col: usize) -> Result<Token> {
        let mut s = String::new();
        loop {
            match self.advance() {
                None | Some('\n') => return Err(DustError::new("unterminated string literal", line, col)),
                Some('\'') => break,
                Some('\\') => {
                    let esc = match self.advance() {
                        Some('n')  => '\n',
                        Some('r')  => '\r',
                        Some('t')  => '\t',
                        Some('\'') => '\'',
                        Some('\\') => '\\',
                        Some('0')  => '\0',
                        _ => return Err(DustError::new("invalid escape", line, col)),
                    };
                    s.push(esc);
                }
                Some(c) => s.push(c),
            }
        }
        Ok(Token::Str(s))
    }

    /// `"..."` — single-line string with `{interpolation}`. Errors on unescaped newline.
    /// Called after the opening `"` has been consumed.
    fn lex_string(&mut self, line: usize, col: usize) -> Result<Token> {
        let mut s = String::new();
        loop {
            match self.advance() {
                None | Some('\n') => return Err(DustError::new("unterminated string (use \"\"\" for multiline)", line, col)),
                Some('"') => break,
                Some('\\') => match self.advance() {
                    Some('n')  => s.push('\n'),
                    Some('r')  => s.push('\r'),
                    Some('t')  => s.push('\t'),
                    Some('"')  => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('0')  => s.push('\0'),
                    Some('x')  => {
                        let hi = self.advance().and_then(|c| c.to_digit(16)).unwrap_or(0);
                        let lo = self.advance().and_then(|c| c.to_digit(16)).unwrap_or(0);
                        s.push(char::from_u32(hi * 16 + lo).unwrap_or('\0'));
                    }
                    _ => return Err(DustError::new("invalid escape sequence", self.line, self.col)),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(Token::Str(s))
    }

    /// `"""..."""` or `'''...'''` — multiline string with indent stripping.
    /// `quote` is the delimiter character (`"` or `'`).
    /// Called after all three opening quotes have been consumed.
    fn lex_triple_string(&mut self, quote: char, line: usize, col: usize) -> Result<Token> {
        let mut s = String::new();
        let base_indent = col.saturating_sub(1);
        // Strip optional leading newline + first-line indent
        if self.peek() == Some('\n') {
            self.advance();
            let mut stripped = 0;
            while stripped < base_indent && self.peek() == Some(' ') {
                self.advance();
                stripped += 1;
            }
        }
        loop {
            match self.advance() {
                None => return Err(DustError::new("unterminated triple-quoted string", line, col)),
                Some(c) if c == quote => {
                    // Check for closing triple quote
                    if self.peek() == Some(quote) && self.chars.get(self.pos + 1).copied() == Some(quote) {
                        self.advance(); self.advance();
                        break;
                    }
                    s.push(c);
                }
                Some('\n') => {
                    s.push('\n');
                    let mut stripped = 0;
                    while stripped < base_indent && self.peek() == Some(' ') {
                        self.advance();
                        stripped += 1;
                    }
                }
                Some(c) => s.push(c),
            }
        }
        // Strip trailing newline before closing quotes
        if s.ends_with('\n') { s.pop(); }
        Ok(Token::Str(s))
    }

    fn lex_number(&mut self, first: char) -> Token {
        let mut s = String::from(first);
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c); self.advance();
            } else if c == '.' && !is_float && matches!(self.peek2(), Some(d) if d.is_ascii_digit()) {
                is_float = true; s.push(c); self.advance();
            } else { break; }
        }
        if is_float { Token::Float(s.parse().unwrap()) } else { Token::Int(s.parse().unwrap()) }
    }

    fn lex_ident(&mut self, first: char) -> Token {
        let mut s = String::from(first);
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' { s.push(c); self.advance(); } else { break; }
        }
        Token::keyword_from_str(&s).unwrap_or(Token::Ident(s))
    }

    // Lex a macro call: name! or name![...] — emit as Ident("name!{...}")
    // Special case: vec![ is NOT collapsed — emitted as Ident("vec!") so the parser can
    // handle spread items (..expr) individually.
    fn lex_macro(&mut self, name: String) -> Result<Token> {
        // consume the !
        self.advance();
        // vec![ is handled by the parser token-by-token
        if name == "vec" && self.peek() == Some('[') {
            return Ok(Token::Ident("vec!".into()));
        }
        let open = self.peek();
        let (open_ch, close_ch) = match open {
            Some('(') => ('(', ')'),
            Some('[') => ('[', ']'),
            Some('{') => ('{', '}'),
            _ => return Ok(Token::Ident(format!("{name}!"))),
        };
        self.advance();
        let mut body = String::new();
        let mut depth = 1usize;
        loop {
            match self.advance() {
                None => break,
                Some(c) if c == open_ch => { depth += 1; body.push(c); }
                Some(c) if c == close_ch => {
                    depth -= 1;
                    if depth == 0 { break; }
                    body.push(c);
                }
                Some(c) => body.push(c),
            }
        }
        Ok(Token::Ident(format!("{name}!{open_ch}{body}{close_ch}")))
    }

    fn tokenize_flat(&mut self) -> Result<Vec<Spanned<Token>>> {
        let mut tokens: Vec<Spanned<Token>> = Vec::new();
        loop {
            // Only skip inline spaces here — newlines are significant
            self.skip_spaces();
            let line = self.line;
            let col  = self.col;
            match self.peek() {
                None => { tokens.push(Spanned::new(Token::Eof, line, col)); break; }
                Some('#') => { while !matches!(self.peek(), None | Some('\n')) { self.advance(); } }
                Some('@') => {
                    self.advance();
                    let mut content = String::new();
                    while !matches!(self.peek(), None | Some('\n')) {
                        content.push(self.advance().unwrap());
                    }
                    tokens.push(Spanned::new(Token::Attr(content.trim().to_string()), line, col));
                }
                Some('\r') => { self.advance(); }
                Some('\n') => { self.advance(); tokens.push(Spanned::new(Token::Newline, line, col)); }
                Some('"') => {
                    self.advance();
                    // """ multiline or " single-line
                    if self.peek() == Some('"') && self.peek2() == Some('"') {
                        self.advance(); self.advance();
                        tokens.push(Spanned::new(self.lex_triple_string('"', line, col)?, line, col));
                    } else {
                        tokens.push(Spanned::new(self.lex_string(line, col)?, line, col));
                    }
                }
                Some('\'') => {
                    self.advance(); // consume opening '
                    // ''' multiline raw string
                    if self.peek() == Some('\'') && self.peek2() == Some('\'') {
                        self.advance(); self.advance();
                        tokens.push(Spanned::new(self.lex_triple_string('\'', line, col)?, line, col));
                        continue;
                    }
                    // Disambiguate: lifetime vs single-quoted string
                    // Rule: if alphanumeric run is NOT followed by closing ' on same line → lifetime
                    if matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
                        // Lookahead: scan the alphanumeric run, check what follows
                        let saved_pos = self.pos;
                        let saved_line = self.line;
                        let saved_col = self.col;
                        let mut ident = String::new();
                        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
                            ident.push(self.advance().unwrap());
                        }
                        if self.peek() == Some('\'') {
                            // 'abc' — single-quoted string (all alphanumeric, closed immediately)
                            self.advance();
                            tokens.push(Spanned::new(Token::Str(ident), line, col));
                        } else {
                            // 'de, 'a — lifetime
                            tokens.push(Spanned::new(Token::Lifetime(ident), line, col));
                            let _ = (saved_pos, saved_line, saved_col); // consumed correctly
                        }
                    } else {
                        // Starts with non-alphanumeric: always a raw string
                        tokens.push(Spanned::new(self.lex_single_quoted_str(line, col)?, line, col));
                    }
                }
                Some(c) if c.is_ascii_digit() => {
                    self.advance();
                    tokens.push(Spanned::new(self.lex_number(c), line, col));
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    self.advance();
                    // c'x' char literal, b'x' byte literal
                    if (c == 'c' || c == 'b') && self.peek() == Some('\'') {
                        self.advance(); // consume '
                        let tok = self.lex_char(line, col)?;
                        let tok = if c == 'b' {
                            match tok { Token::Char(ch) => Token::ByteChar(ch), _ => tok }
                        } else { tok };
                        tokens.push(Spanned::new(tok, line, col));
                        continue;
                    }
                    let tok = self.lex_ident(c);
                    // Check for macro: ident immediately followed by !
                    if self.peek() == Some('!') {
                        if let Token::Ident(name) = tok {
                            tokens.push(Spanned::new(self.lex_macro(name)?, line, col));
                            continue;
                        }
                    }
                    tokens.push(Spanned::new(tok, line, col));
                }
                Some(_) => {
                    let c = self.advance().unwrap();
                    let tok = match c {
                        '+' => if self.peek() == Some('+') { self.advance(); Token::PlusPlus }
                               else if self.peek() == Some('=') { self.advance(); Token::PlusEq }
                               else { Token::Plus },
                        '*' => if self.peek() == Some('*') { self.advance(); Token::StarStar }
                               else if self.peek() == Some('=') { self.advance(); Token::StarEq }
                               else { Token::Star },
                        '/' => if self.peek() == Some('=') { self.advance(); Token::SlashEq } else { Token::Slash },
                        '%' => Token::Percent,
                        '~' => Token::Tilde,
                        '(' => Token::LParen,
                        ')' => Token::RParen,
                        '[' => Token::LBracket,
                        ']' => Token::RBracket,
                        '{' => Token::LBrace,
                        '}' => Token::RBrace,
                        ',' => Token::Comma,
                        '?' => Token::Question,
                        '.' => if self.peek() == Some('.') { self.advance(); Token::DotDot } else { Token::Dot },
                        '-' => if self.peek() == Some('>') { self.advance(); Token::Arrow }
                               else if self.peek() == Some('-') { self.advance(); Token::MinusMinus }
                               else if self.peek() == Some('=') { self.advance(); Token::MinusEq }
                               else { Token::Minus },
                        '=' => if self.peek() == Some('=') { self.advance(); Token::EqEq } else { Token::Eq },
                        '!' => if self.peek() == Some('=') { self.advance(); Token::BangEq } else { Token::Bang },
                        '<' => if self.peek() == Some('=') { self.advance(); Token::LtEq } else { Token::Lt },
                        '>' => if self.peek() == Some('=') { self.advance(); Token::GtEq } else { Token::Gt },
                        ':' => if self.peek() == Some(':') { self.advance(); Token::ColonColon } else { Token::Colon },
                        '&' => if self.peek() == Some('&') {
                                   self.advance();
                                   if self.peek() == Some('=') { self.advance(); Token::AndAndEq } else { Token::AndAnd }
                               } else { Token::Ampersand },
                        '|' => if self.peek() == Some('|') {
                                   self.advance();
                                   if self.peek() == Some('=') { self.advance(); Token::PipePipeEq } else { Token::PipePipe }
                               } else { Token::Pipe },
                        c => return Err(DustError::new(format!("unexpected character '{c}'"), line, col)),
                    };
                    tokens.push(Spanned::new(tok, line, col));
                }
            }
        }
        Ok(tokens)
    }
}

pub fn lex(src: &str) -> Result<Vec<Spanned<Token>>> {
    let mut lexer = Lexer::new(src);
    let flat = lexer.tokenize_flat()?;
    inject_indentation(src, flat)
}

fn line_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn inject_indentation(src: &str, flat: Vec<Spanned<Token>>) -> Result<Vec<Spanned<Token>>> {
    let indents: Vec<usize> = src.lines().map(line_indent).collect();
    let indent_of = |line: usize| -> usize {
        indents.get(line.saturating_sub(1)).copied().unwrap_or(0)
    };

    let mut out: Vec<Spanned<Token>> = Vec::new();
    let mut stack: Vec<usize> = vec![0];
    let mut i = 0;

    while i < flat.len() {
        match &flat[i].value {
            Token::Newline => {
                // Skip consecutive newlines; find the next real token
                let mut j = i + 1;
                while j < flat.len() && matches!(flat[j].value, Token::Newline) {
                    j += 1;
                }

                // Emit the newline
                out.push(flat[i].clone());

                if j < flat.len() && !matches!(flat[j].value, Token::Eof) {
                    let next_line = flat[j].line;
                    let indent = indent_of(next_line);
                    let curr = *stack.last().unwrap();

                    if indent > curr {
                        stack.push(indent);
                        out.push(Spanned::new(Token::Indent, next_line, 1));
                    } else if indent < curr {
                        while *stack.last().unwrap() > indent {
                            stack.pop();
                            out.push(Spanned::new(Token::Dedent, next_line, 1));
                        }
                    }
                }

                i = j;
                continue;
            }
            Token::Eof => {
                // Close any open indents
                while stack.len() > 1 {
                    stack.pop();
                    out.push(Spanned::new(Token::Dedent, flat[i].line, 1));
                }
                out.push(flat[i].clone());
                break;
            }
            _ => out.push(flat[i].clone()),
        }
        i += 1;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|s| s.value).collect()
    }

    #[test]
    fn lex_int() {
        assert_eq!(tokens("42"), vec![Token::Int(42), Token::Eof]);
    }

    #[test]
    fn lex_float() {
        assert_eq!(tokens("3.14"), vec![Token::Float(3.14), Token::Eof]);
    }

    #[test]
    fn lex_keyword_and_ident() {
        assert_eq!(tokens("let x = 1"), vec![Token::KwLet, Token::Ident("x".into()), Token::Eq, Token::Int(1), Token::Eof]);
    }

    #[test]
    fn lex_arrow() {
        assert_eq!(tokens("->"), vec![Token::Arrow, Token::Eof]);
    }

    #[test]
    fn lex_colon_colon() {
        assert_eq!(tokens("::"), vec![Token::ColonColon, Token::Eof]);
    }

    #[test]
    fn lex_string() {
        assert_eq!(tokens("\"hello\""), vec![Token::Str("hello".into()), Token::Eof]);
    }

    #[test]
    fn lex_single_quoted_raw_string() {
        assert_eq!(tokens(r#"'{"key": "val"}'"#), vec![Token::Str("{\"key\": \"val\"}".into()), Token::Eof]);
    }

    #[test]
    fn lex_triple_double_multiline() {
        let src = "\"\"\"line one\nline two\"\"\"";
        assert_eq!(tokens(src), vec![Token::Str("line one\nline two".into()), Token::Eof]);
    }

    #[test]
    fn lex_triple_double_strips_indent() {
        let src = "let s = \"\"\"\n  hello\n  world\n\"\"\"";
        let toks = tokens(src);
        assert!(toks.contains(&Token::Str("hello\nworld".into())), "got: {:?}", toks);
    }

    #[test]
    fn lex_triple_single_multiline() {
        let src = "'''line one\nline two'''";
        assert_eq!(tokens(src), vec![Token::Str("line one\nline two".into()), Token::Eof]);
    }

    #[test]
    fn lex_char_literal() {
        assert_eq!(tokens("c'x'"), vec![Token::Char('x'), Token::Eof]);
    }

    #[test]
    fn lex_lifetime() {
        assert_eq!(tokens("'de"), vec![Token::Lifetime("de".into()), Token::Eof]);
    }

    #[test]
    fn lex_comment_skipped() {
        assert_eq!(tokens("# comment"), vec![Token::Eof]);
    }

    #[test]
    fn lex_macro() {
        assert_eq!(tokens("println!(\"hi\")"), vec![Token::Ident("println!(\"hi\")".into()), Token::Eof]);
    }

    #[test]
    fn lex_indentation() {
        let src = "fn foo\n  let x = 1\n  x";
        let toks = tokens(src);
        assert!(toks.contains(&Token::Indent), "expected Indent in {:?}", toks);
        assert!(toks.contains(&Token::Dedent), "expected Dedent in {:?}", toks);
    }

    #[test]
    fn lex_bool() {
        assert_eq!(tokens("true"), vec![Token::Bool(true), Token::Eof]);
        assert_eq!(tokens("false"), vec![Token::Bool(false), Token::Eof]);
    }
}
