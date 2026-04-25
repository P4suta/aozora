//! Render a UTF-8 Aozora source file to HTML on stdout.
//!
//! Run it (from the workspace root, inside the dev container):
//!
//!     cargo run --example render-utf8 -p aozora-parser -- input.txt

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

use aozora_parser::html::render_to_string;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: render-utf8 <path/to/input.txt>");
        return ExitCode::from(2);
    };

    let input = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Convenience wrapper — runs the lexer and renders the resulting
    // normalized text + registry in one shot.
    let html = render_to_string(&input);

    if let Err(e) = io::stdout().write_all(html.as_bytes()) {
        eprintln!("write failed: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
