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

    fn lex_string(&mut self, line: usize, col: usize) -> Result<Token> {
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err(DustError::new("unterminated string", line, col)),
                Some('"') => break,
                Some('\\') => match self.advance() {
                    Some('n')  => s.push('\n'),
                    Some('r')  => s.push('\r'),
                    Some('t')  => s.push('\t'),
                    Some('"')  => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('0')  => s.push('\0'),
                    _ => return Err(DustError::new("invalid escape sequence", self.line, self.col)),
                },
                Some(c) => s.push(c),
            }
        }
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
    fn lex_macro(&mut self, name: String) -> Result<Token> {
        // consume the !
        self.advance();
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
                Some('\r') => { self.advance(); }
                Some('\n') => { self.advance(); tokens.push(Spanned::new(Token::Newline, line, col)); }
                Some('"') => {
                    self.advance();
                    tokens.push(Spanned::new(self.lex_string(line, col)?, line, col));
                }
                Some('\'') => {
                    self.advance();
                    tokens.push(Spanned::new(self.lex_char(line, col)?, line, col));
                }
                Some(c) if c.is_ascii_digit() => {
                    self.advance();
                    tokens.push(Spanned::new(self.lex_number(c), line, col));
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    self.advance();
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
                        '*' => if self.peek() == Some('=') { self.advance(); Token::StarEq } else { Token::Star },
                        '/' => if self.peek() == Some('=') { self.advance(); Token::SlashEq } else { Token::Slash },
                        '%' => Token::Percent,
                        '(' => Token::LParen,
                        ')' => Token::RParen,
                        '[' => Token::LBracket,
                        ']' => Token::RBracket,
                        '{' => Token::LBrace,
                        '}' => Token::RBrace,
                        ',' => Token::Comma,
                        '?' => Token::Question,
                        '.' => Token::Dot,
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
        assert_eq!(tokens("let x"), vec![Token::KwLet, Token::Ident("x".into()), Token::Eof]);
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
