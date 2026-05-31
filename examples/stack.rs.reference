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
}

fn main() {
    let mut stack = Stack::new();
    stack.push(3.0);
    stack.push(4.0);
    println!("{:?}", stack.pop());
}
