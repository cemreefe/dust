#[derive(Debug, Clone)]
pub struct DustError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl DustError {
    pub fn new(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self { message: message.into(), line, col }
    }
}

impl std::fmt::Display for DustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error at {}:{}: {}", self.line, self.col, self.message)
    }
}

pub type Result<T> = std::result::Result<T, DustError>;
