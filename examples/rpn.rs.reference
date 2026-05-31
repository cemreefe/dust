use std::io::{self, BufRead};

struct Stack {
    values: Vec<f64>,
}

impl Stack {
    fn new() -> Stack {
        Stack { values: vec![] }
    }

    fn push(&mut self, n: f64) {
        self.values.push(n);
    }

    fn pop(&mut self) -> Option<f64> {
        self.values.pop()
    }

    fn apply(&mut self, op: &str) -> bool {
        let b = match self.pop() {
            Some(v) => v,
            None => return false,
        };
        let a = match self.pop() {
            Some(v) => v,
            None => return false,
        };
        let result = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            "%" => a % b,
            _ => return false,
        };
        self.push(result);
        true
    }

    fn peek(&self) -> Option<f64> {
        self.values.last().copied()
    }
}

fn eval(line: &str) -> String {
    let mut stack = Stack::new();
    for token in line.split_whitespace() {
        match token.parse::<f64>() {
            Ok(n) => stack.push(n),
            Err(_) => {
                if !stack.apply(token) {
                    return "error: invalid token or not enough operands".to_string();
                }
            }
        }
    }
    match stack.peek() {
        Some(result) => result.to_string(),
        None => "error: empty expression".to_string(),
    }
}

fn main() {
    println!("Rust RPN calculator. Examples:  3 4 +   |   2 3 4 * +   |   10 2 /");
    println!("Press Ctrl+C to exit.");
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        println!("= {}", eval(line));
    }
}
