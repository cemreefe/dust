# Dust Language Design Spec

**Date:** 2026-05-31
**Status:** Approved

---

## Overview

Dust is a language that compiles to Rust. It targets developers who want Rust's performance and safety guarantees without Rust's syntactic weight. The compiler is written in Rust and outputs Rust source, which is then compiled by `rustc` / `cargo`.

Design goals:
- Indentation-based, minimal punctuation
- JS/Python/MoonScript-influenced syntax
- Smart ownership defaults — most code never mentions borrowing
- Full Rust feature parity (traits, generics, async, macros, enums)
- Ugly-but-correct output is fine; readability of generated Rust is not a goal
- Others will use this language, so error messages matter

---

## Architecture

Five-stage pipeline:

```
source.dust
     │
     ▼
  [Lexer]          → token stream
     │
     ▼
  [Parser]         → AST
     │
     ▼
  [Semantic Pass]  → annotated AST (ownership, mut inference, clone sites)
     │
     ▼
  [Emitter]        → Rust source string
     │
     ▼
  rustc / cargo
```

Each stage is a separate module with a clean interface:
- `lex(src: &str) -> Vec<Token>`
- `parse(tokens: Vec<Token>) -> Ast`
- `analyze(ast: Ast) -> AnnotatedAst`
- `emit(ast: AnnotatedAst) -> String`

Source line numbers are tracked through every stage so rustc errors can be attributed back to `.dust` source lines.

The CLI (`dust build file.dust`) runs the pipeline, writes `.rs` output to a temp dir, and hands off to `cargo build`.

---

## Cargo Integration

Dust uses Cargo directly. Users write a standard `Cargo.toml` and import crates by name. The Dust compiler does not wrap or replace Cargo — it is invoked after emission. Crate APIs are called using Dust syntax (which maps to Rust syntax naturally).

---

## Variables

```dust
let x = 5        # inferred mutability — transpiler inserts mut if x is reassigned/mutated
const x = 5      # forced immutable — transpile-time error if mutated
mut x = 5        # forced mutable — always emits `let mut`
```

Emitted Rust:
```rust
let x = 5;           // or let mut x = 5; if later mutated
let x = 5;           // const enforced at transpile time
let mut x = 5;
```

---

## Types

### Strings

One string type: `str`. The transpiler emits the correct Rust type based on context:
- Owned context (variable binding, return value) → `String`
- Borrowed context (function arg without `keep`) → `&str`

```dust
let name: str = "Alice"       →  let name: String = String::from("Alice");
fn greet(name: str) -> str    →  fn greet(name: &str) -> String {
```

### Other types

All Rust primitive types (`i32`, `f64`, `bool`, `u8`, etc.) are used as-is. `Vec<T>`, `Option<T>`, `Result<T>`, etc. are unchanged.

---

## Ownership

### Auto-borrow (default)

Function arguments are automatically borrowed. You never write `&` for normal usage:

```dust
fn print_name(name: str)
  println!("{}", name)
```

Emits:
```rust
fn print_name(name: &str) {
    println!("{}", name);
}
```

### Explicit ownership transfer (`keep`)

When a function needs to take ownership (storing in a struct, sending to a thread, etc.), use `keep`:

```dust
fn store(keep name: str)
  self.name = name
```

Emits:
```rust
fn store(name: String) {
    self.name = name;
}
```

### Cloning

`let x = y` clones by default when `y` is a non-primitive heap type. This keeps the common case simple at the cost of some performance. Users who care about performance use explicit borrows or `keep`.

---

## Functions

```dust
fn add(a: i32, b: i32) -> i32
  a + b
```

- Indentation-based body
- Last expression is implicitly returned
- No `return` keyword needed (but allowed)

---

## Closures

```dust
let double = (x: i32) -> x * 2
let doubled = nums.iter().map((x) -> x * 2)
```

Emits `|x: i32| x * 2` in Rust. Arrow function syntax, same as JS.

---

## Structs

Methods and trait implementations live inside the struct body. No separate `impl` blocks.

```dust
struct Dog is Animal, Swimmer
  name: str

  fn bark(self)
    println!("{}", self.name)

  fn speak(self) -> str         # satisfies Animal
    "woof"
```

Emits:
```rust
struct Dog {
    name: String,
}

impl Dog {
    fn bark(self) {
        println!("{}", self.name);
    }
}

impl Animal for Dog {
    fn speak(self) -> &str {
        "woof"
    }
}
```

The transpiler routes methods to the correct `impl` block based on which trait requires them.

### Name collisions across traits

When two traits require a method with the same name, use qualified syntax:

```dust
struct Foo is Bar, Baz
  fn Bar::shared(self) -> str
    "from bar"
  fn Baz::shared(self) -> str
    "from baz"
```

---

## Traits

```dust
trait Animal
  fn speak(self) -> str
```

Emits:
```rust
trait Animal {
    fn speak(self) -> &str;
}
```

---

## Enums

```dust
enum Shape
  Circle(f64)
  Rect(f64, f64)
  Empty
```

Emits:
```rust
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Empty,
}
```

---

## Pattern Matching

```dust
match shape
  Circle(r)  => pi * r * r
  Rect(w, h) => w * h
  Empty      => 0.0
```

Emits standard Rust `match` block.

---

## String Interpolation

```dust
let s = "Hello, {name}!"
```

Emits:
```rust
let s = format!("Hello, {name}!");
```

Uses Rust 1.58+ captured identifier syntax — no transformation of string contents needed.

---

## If Expressions

```dust
let label = if x > 0 then "pos" else "neg"
```

Multi-line:
```dust
let label = if x > 0
  "pos"
else
  "neg"
```

---

## Error Handling

```dust
# ? operator passes through unchanged
fn read_file(path: str) -> Result<str>
  let content = fs::read_to_string(path)?
  content

# try/catch sugar (emits match on Result, not exceptions)
try
  let data = fetch(url)?
  process(data)
catch e
  log_error(e)

# unwrap shorthand
let x = maybe.unwrap!
```

`try/catch` emits a `match` block wrapping a closure — no runtime overhead, no exceptions.

---

## Generics

```dust
fn first<T>(list: Vec<T>) -> Option<T>
  list.into_iter().next()
```

Generic syntax is unchanged from Rust.

---

## Async

```dust
async fn fetch(url: str) -> Result<str>
  let body = await http.get(url)
  body.text()
```

`await` is a prefix keyword (vs Rust's postfix `.await`). Emits `.await` in output.

---

## Macros

Rust macros pass through unchanged:

```dust
println!("hi")
vec![1, 2, 3]
format!("{}", x)
```

---

## Modules

One file = one module. Filename = module name. `use` statements pass through unchanged. No new module system — Cargo's existing module system is used as-is.

---

## File Extension

`.dust`

---

## Error Messages

Dust tracks source line numbers through all pipeline stages. When rustc reports an error, the Dust CLI maps it back to the originating `.dust` line and reports both:

```
error[E0382]: use of moved value
  --> src/main.dust:12:5
```

Transpile-time errors (e.g. mutating a `const`) are reported before rustc is invoked, with clear messages referencing Dust concepts (not Rust internals).

---

## Summary Table

| Dust | Rust |
|------|------|
| `let x = y` | `let x = y.clone();` (non-primitive) |
| `const x = 5` | `let x = 5;` (enforced immutable) |
| `mut x = 5` | `let mut x = 5;` |
| `fn f(x: str)` | `fn f(x: &str)` |
| `fn f(keep x: str)` | `fn f(x: String)` |
| `"Hello, {name}!"` | `format!("Hello, {name}!")` |
| `await expr` | `expr.await` |
| `x.unwrap!` | `x.unwrap()` |
| `struct Foo is Bar` | `struct Foo {}` + `impl Bar for Foo {}` |
| `(x) -> x * 2` | `\|x\| x * 2` |
