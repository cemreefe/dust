use std::collections::HashMap;
use std::io::{self, BufRead};

struct WordCounter {
    counts: HashMap<String, usize>,
}

impl WordCounter {
    fn new() -> WordCounter {
        WordCounter { counts: HashMap::new() }
    }

    fn add(&mut self, word: &str) {
        *self.counts.entry(word.to_lowercase()).or_insert(0) += 1;
    }

    fn top(&self, n: usize) -> Vec<(String, usize)> {
        let mut pairs: Vec<_> = self.counts.iter().map(|(w, c)| (w.clone(), *c)).collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1));
        pairs.into_iter().take(n).collect()
    }
}

fn main() {
    let mut counter = WordCounter::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        for word in line.unwrap().split_whitespace() {
            counter.add(word);
        }
    }
    for (word, count) in counter.top(10).iter() {
        println!("{count:>6}  {word}");
    }
}
