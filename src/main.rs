mod error;
mod lexer;
mod parser;
mod semantic;
mod emitter;

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] != "build" {
        eprintln!("Usage: dust build <file.dust>");
        std::process::exit(1);
    }

    let input_path = Path::new(&args[2]);
    if input_path.extension().and_then(|e| e.to_str()) != Some("dust") {
        eprintln!("error: file must have .dust extension");
        std::process::exit(1);
    }

    let src = match std::fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not read {}: {e}", input_path.display()); std::process::exit(1); }
    };

    // Pipeline
    let tokens = match lexer::lex(&src) {
        Ok(t) => t,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };

    let ast = match parser::parse(&tokens) {
        Ok(a) => a,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };

    let ast = match semantic::analyze(ast) {
        Ok(a) => a,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };

    let rust_src = emitter::emit(&ast);

    // Write to a .rs file next to the .dust file
    let output_path = input_path.with_extension("rs");
    if let Err(e) = std::fs::write(&output_path, &rust_src) {
        eprintln!("error: could not write {}: {e}", output_path.display());
        std::process::exit(1);
    }

    println!("Emitted {}", output_path.display());
    println!("--- generated Rust ---");
    println!("{rust_src}");
}
