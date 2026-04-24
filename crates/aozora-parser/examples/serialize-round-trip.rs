//! Parse an afm source and confirm `serialize ∘ parse ≡ id` on the
//! lexer-normalised input. This is the I3 invariant from the 17 k-work
//! corpus sweep (ADR-0007), demonstrated on a single file.
//!
//! Run:
//!
//!     cargo run --example serialize-round-trip -p afm-parser -- input.md

use std::env;
use std::fs;
use std::process::ExitCode;

use afm_parser::{Options, parse, serialize};
use comrak::Arena;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: serialize-round-trip <path/to/input.md>");
        return ExitCode::from(2);
    };

    let input = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let arena = Arena::new();
    let options = Options::afm_default();
    let parsed = parse(&arena, &input, &options);
    let serialised = serialize(&parsed);

    // Mirror the lexer's sanitize phase: strip UTF-8 BOM, CRLF → LF.
    // Anything the lexer normalises cannot round-trip byte-for-byte to
    // the *original* input; the contract is fixed-point on the
    // *normalised* input, which is what I3 checks in the corpus sweep.
    let expected = input
        .strip_prefix('\u{feff}')
        .unwrap_or(&input)
        .replace("\r\n", "\n");

    if serialised == expected {
        println!("round-trip OK ({} bytes)", serialised.len());
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "round-trip drift: {} bytes in (post-sanitize) → {} bytes out",
            expected.len(),
            serialised.len(),
        );
        ExitCode::from(3)
    }
}
