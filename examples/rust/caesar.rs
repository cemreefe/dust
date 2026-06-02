use std::io::{self, BufRead};

fn shift_char(ch: char, shift: u8) -> char {
    if ch >= 'a' && ch <= 'z' {
        let shifted = (ch as u8 - b'a' + shift) % 26 + b'a';
        shifted as char
    } else if ch >= 'A' && ch <= 'Z' {
        let shifted = (ch as u8 - b'A' + shift) % 26 + b'A';
        shifted as char
    } else {
        ch
    }
}

fn caesar(text: &str, shift: u8) -> String {
    text.chars().map(|ch| shift_char(ch, shift)).collect()
}

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        println!("{}", caesar(&line, 13));
    }
}
