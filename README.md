<p align="center">
  <img src="logo.png" width="160" alt="Dust logo">
</p>

# Dust

A programming language that compiles to Rust. Python-influenced syntax, indentation-based blocks, smart ownership defaults. Ugly-but-correct output.

```dust
struct Stack
  values: Vec<f64>

  fn new() -> Stack
    Stack
      values: vec![]

  fn push(self, n: f64)
    self.values.push(n)

  fn pop(self) -> Option<f64>
    self.values.pop()

fn main()
  mut stack = Stack::new()
  stack.push(3.0)
  stack.push(4.0)
  println!("{stack.pop()}")
```

## Install

Requires Rust / Cargo.

```sh
git clone https://github.com/cemreefe/dust
cd dust
cargo install --path .
```

## Usage

```sh
dust run     hello.dust        # compile and run
dust compile hello.dust        # compile to binary
dust build   hello.dust        # emit hello.rs
```

## Syntax

### Variables

```dust
let x = compute()   # inferred mutability — compiler decides
mut x = compute()   # explicitly mutable
const X: i32 = 1    # explicitly immutable, value known at compile time
```

`let` is the default — use it when you don't care about mutability. `mut` opts into mutability explicitly. `const` requires a type annotation and a compile-time value.

### Functions

```dust
fn add(a: i32, b: i32) -> i32
  a + b
```

Auto-borrows string params (`str` → `&str`). Use `keep` to take ownership:

```dust
fn consume(keep name: str)     # name: String  — owns the value
fn read(name: str)             # name: &str    — borrows (default)
```

### Structs & Methods

```dust
struct Point
  x: f64
  y: f64

  fn new(x: f64, y: f64) -> Point
    Point
      x: x
      y: y

  fn distance(self) -> f64
    (self.x * self.x + self.y * self.y).sqrt()
```

### Control Flow

```dust
if x > 0
  println!("positive")
elif x < 0
  println!("negative")
else
  println!("zero")
```

Inline form:

```dust
let label = if x > 0 then "pos" else "neg"
```

### Match

```dust
match status
  "ok"  -> println!("good")
  "err" -> println!("bad")
  _     -> println!("unknown")

match result
  Ok(v)  -> v
  Err(e) -> return "failed".to_string()
```

### Loops

```dust
for item in collection.iter()
  println!("{item}")

for line in stdin.lock().lines()
  let line = line.unwrap!
  println!("{line}")
```

### Operators

```dust
x++   x--          # increment / decrement
x += n   x -= n    # compound assign
x ||= y  x &&= y   # logical assign
```

### Error handling

```dust
let val = risky().unwrap!      # .unwrap()
let val = risky()?             # propagate
```

### String interpolation

Any expression works inside `{}`:

```dust
println!("Hello, {name}!")
println!("result: {stack.pop()}")
println!("x squared: {x * x}")
let msg = "uppercase: {name.to_uppercase()}"
```

### Ownership

```dust
fn consume(keep buf: Vec<u8>)   # takes ownership
fn read(data: str)              # borrows (&str, default)
```

### Enums

```dust
enum Shape
  Circle(f64)
  Rect(f64, f64)
  Point
```

### Traits

```dust
struct Dog
  is Display

  fn Display.fmt(self, f: &mut Formatter) -> Result
    write!(f, "Dog")
```

## Examples

| File | Description |
|------|-------------|
| `examples/rpn.dust` | RPN calculator (stdin) |
| `examples/todo_server.dust` | HTTP todo server with frontend |
| `examples/hello.dust` | Hello world |

## How it works

Dust is a source-to-source transpiler:

```
.dust → Lexer → Parser → Semantic pass → Emitter → .rs → rustc → binary
```

- **Lexer** — indentation-aware (INDENT/DEDENT tokens), handles macros, char literals, escape sequences
- **Parser** — recursive descent, produces a typed AST
- **Semantic pass** — auto-borrows params, promotes `let` → `let mut` when reassigned, catches `const` mutation
- **Emitter** — walks AST, emits Rust source

The output is valid Rust. You can inspect it with `dust build`.
