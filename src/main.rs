mod error;
mod lexer;
mod parser;
mod semantic;
mod emitter;

use std::path::Path;

fn compile(input_path: &Path) -> String {
    let src = match std::fs::read_to_string(input_path) {
        Ok(s) => s,
        Err(e) => { eprintln!("error: could not read {}: {e}", input_path.display()); std::process::exit(1); }
    };

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
    emitter::emit(&ast)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "build" => {
            let path = args.get(2).map(Path::new).unwrap_or_else(|| {
                eprintln!("Usage: dust build <file.dust>"); std::process::exit(1);
            });
            let rust_src = compile(path);
            let out = path.with_extension("rs");
            std::fs::write(&out, &rust_src).unwrap_or_else(|e| {
                eprintln!("error: {e}"); std::process::exit(1);
            });
            eprintln!("Emitted {}", out.display());
        }
        "compile" => {
            let path = args.get(2).map(Path::new).unwrap_or_else(|| {
                eprintln!("Usage: dust compile <file.dust>"); std::process::exit(1);
            });
            let rust_src = compile(path);
            let rs_path = path.with_extension("rs");
            std::fs::write(&rs_path, &rust_src).unwrap_or_else(|e| {
                eprintln!("error: {e}"); std::process::exit(1);
            });
            let bin_path = path.with_extension("");
            let status = std::process::Command::new("rustc")
                .arg(&rs_path)
                .arg("-o").arg(&bin_path)
                .arg("--edition=2021")
                .stderr(std::process::Stdio::inherit())
                .status()
                .expect("rustc not found");
            if !status.success() { std::process::exit(1); }
            eprintln!("Compiled {}", bin_path.display());
        }
        "run" => {
            let path = args.get(2).map(Path::new).unwrap_or_else(|| {
                eprintln!("Usage: dust run <file.dust> [args...]"); std::process::exit(1);
            });
            let rust_src = compile(path);

            // Write to temp .rs file
            let tmp_rs  = std::env::temp_dir().join("_dust_run.rs");
            let tmp_bin = std::env::temp_dir().join("_dust_run");
            std::fs::write(&tmp_rs, &rust_src).unwrap();

            // Compile with rustc
            let status = std::process::Command::new("rustc")
                .arg(&tmp_rs)
                .arg("-o").arg(&tmp_bin)
                .arg("--edition=2021")
                .stderr(std::process::Stdio::inherit())
                .status()
                .expect("rustc not found");

            if !status.success() { std::process::exit(1); }

            // Run, forwarding remaining args and exit code
            let status = std::process::Command::new(&tmp_bin)
                .args(&args[3..])
                .status()
                .expect("failed to run binary");

            std::process::exit(status.code().unwrap_or(1));
        }
        _ => {
            eprintln!("Usage:");
            eprintln!("  dust build   <file.dust>        emit .rs file");
            eprintln!("  dust compile <file.dust>        compile to binary");
            eprintln!("  dust run     <file.dust> [args] compile and run");
            std::process::exit(1);
        }
    }
}
