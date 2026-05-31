fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

fn main() {
    let msg = greet("Alice");
    println!("{}", msg)
}

