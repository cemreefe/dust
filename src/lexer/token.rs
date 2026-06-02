#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Bool(bool),

    // Identifier
    Ident(String),

    // Keywords
    KwLet,
    KwConst,
    KwMut,
    KwFn,
    KwStruct,
    KwTrait,
    KwIs,
    KwMatch,
    KwIf,
    KwThen,
    KwElse,
    KwElif,
    KwAsync,
    KwAwait,
    KwTry,
    KwCatch,
    KwKeep,
    KwReturn,
    KwUse,
    KwEnum,
    KwFor,
    KwIn,
    KwWhile,
    KwLoop,
    KwBreak,
    KwContinue,
    KwSelf,
    KwAs,

    // Operators
    Arrow,       // ->
    Question,    // ?
    Bang,        // !
    Eq,          // =
    EqEq,        // ==
    BangEq,      // !=
    Lt,          // <
    Gt,          // >
    LtEq,        // <=
    GtEq,        // >=
    Plus,        // +
    Minus,       // -
    Star,        // *
    StarStar,    // **
    Slash,       // /
    Percent,     // %
    PlusPlus,      // ++
    MinusMinus,    // --
    PlusEq,        // +=
    MinusEq,       // -=
    StarEq,        // *=
    SlashEq,       // /=
    AndAndEq,      // &&=
    PipePipeEq,    // ||=
    ColonColon,  // ::
    Dot,         // .
    Comma,       // ,
    Colon,       // :
    Ampersand,   // &
    Pipe,        // |
    AndAnd,      // &&
    PipePipe,    // ||
    Tilde,       // ~  (uninitialized binding marker)

    // Delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // Significant whitespace
    Indent,
    Dedent,
    Newline,

    Eof,
}

impl Token {
    pub fn keyword_from_str(s: &str) -> Option<Token> {
        match s {
            "let"      => Some(Token::KwLet),
            "const"    => Some(Token::KwConst),
            "mut"      => Some(Token::KwMut),
            "fn"       => Some(Token::KwFn),
            "struct"   => Some(Token::KwStruct),
            "trait"    => Some(Token::KwTrait),
            "is"       => Some(Token::KwIs),
            "match"    => Some(Token::KwMatch),
            "if"       => Some(Token::KwIf),
            "then"     => Some(Token::KwThen),
            "else"     => Some(Token::KwElse),
            "elif"     => Some(Token::KwElif),
            "async"    => Some(Token::KwAsync),
            "await"    => Some(Token::KwAwait),
            "try"      => Some(Token::KwTry),
            "catch"    => Some(Token::KwCatch),
            "keep"     => Some(Token::KwKeep),
            "return"   => Some(Token::KwReturn),
            "use"      => Some(Token::KwUse),
            "enum"     => Some(Token::KwEnum),
            "for"      => Some(Token::KwFor),
            "in"       => Some(Token::KwIn),
            "while"    => Some(Token::KwWhile),
            "loop"     => Some(Token::KwLoop),
            "break"    => Some(Token::KwBreak),
            "continue" => Some(Token::KwContinue),
            "self"     => Some(Token::KwSelf),
            "as"       => Some(Token::KwAs),
            "true"     => Some(Token::Bool(true)),
            "false"    => Some(Token::Bool(false)),
            _          => None,
        }
    }
}
