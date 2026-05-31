use std::io::{self, BufRead};

fn shift_char(c: char, n: u8) -> char {
    if c >= 'a' && c <= 'z' {
        let shifted = (c as u8 - b'a' + n) % 26 + b'a';
        shifted as char
    } else if c >= 'A' && c <= 'Z' {
        let shifted = (c as u8 - b'A' + n) % 26 + b'A';
        shifted as char
    } else {
        c
    }
}

fn caesar(text: &str, shift: u8) -> String {
    text.chars().map(|c| shift_char(c, shift)).collect()
}

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        println!("{}", caesar(&line, 13));
    }
}
