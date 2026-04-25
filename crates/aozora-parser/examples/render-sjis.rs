//! Render a Shift_JIS Aozora Bunko source file to HTML on stdout.
//!
//! Run it against an unpacked Aozora Bunko file:
//!
//!     cargo run --example render-sjis -p aozora-parser -- tsumito_batsu.txt

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

use aozora_encoding::decode_sjis;
use aozora_parser::html::render_to_string;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: render-sjis <path/to/input.sjis.txt>");
        return ExitCode::from(2);
    };

    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let utf8 = match decode_sjis(&bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Shift_JIS decode failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let html = render_to_string(&utf8);

    if let Err(e) = io::stdout().write_all(html.as_bytes()) {
        eprintln!("write failed: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
