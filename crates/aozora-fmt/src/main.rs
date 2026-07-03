//! `aozora-fmt` — CLI formatter for aozora-flavored-markdown documents.
//!
//! All behaviour lives in the library ([`aozora_fmt::run`]); this binary is a
//! thin shim that parses the shared clap [`Cli`] and runs it. See `--help` for
//! the full surface (modes, multi-file/directory input, `--diff`, `--list`,
//! `--json`, `--color`) and exit codes.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use aozora_fmt::Cli;
use clap::Parser;

fn main() -> ExitCode {
    aozora_fmt::run(&Cli::parse())
}
