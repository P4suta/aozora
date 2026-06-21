//! `aozora-fmt` — CLI formatter for aozora-flavored-markdown documents.
//!
//! All behaviour lives in the library ([`aozora_fmt::run`]); this binary is
//! a thin shim so `xtask` can reach the same clap [`Cli`] definition to
//! generate shell completions and the man page. See `--help` for the full
//! surface (modes, multi-file/directory input, `--diff`, `--list`, `--json`,
//! `--color`) and exit codes.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use aozora_fmt::Cli;
use clap::Parser;

fn main() -> ExitCode {
    aozora_fmt::run(&Cli::parse())
}
