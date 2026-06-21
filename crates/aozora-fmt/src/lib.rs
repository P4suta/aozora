//! `aozora-fmt` library: [`format_source`] runs the `parse ∘ to_source`
//! round-trip that produces an idempotent, canonicalised aozora document.
//! Every consumer — the binary, the `aozora-lsp` formatting handler, CI
//! gates — reaches the same canonical form; the round-trip is a fixed point
//! on the second pass.
//!
//! The CLI itself lives here too ([`Cli`], [`run`]) so `xtask` can reach
//! `Cli::command()` to generate completions and the man page.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use aozora::Document;

mod cli;
mod discover;
mod process;
mod report;

pub use cli::Cli;

// Shared CLI plumbing, re-exported so the `aozora` CLI's `fmt` subcommand can
// reuse the formatter's path discovery, colour policy, and panic guard instead
// of re-implementing them.
pub use cli::{ColorChoice, FmtArgs};
pub use discover::{Input, Resolved, resolve};
pub use process::{Panicked, guard};
pub use report::auto_stdout;

/// Compiles and runs the fenced Rust example in this crate's `README.md` as a
/// doctest, so the documented public API (`format_source`) can't silently
/// drift from the code. `#[cfg(doctest)]` means the item exists only while
/// rustdoc collects doctests — it never reaches a normal build, so neither
/// `dead_code` nor `missing_debug_implementations` fire on it.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

use cli::{CheckReport, Mode};
use report::Outcome;

/// Canonicalise an aozora source string.
///
/// Runs the aozora-lex pipeline and then the inverse serializer.
/// The returned `String` is byte-identical on the second pass.
#[must_use]
pub fn format_source(source: &str) -> String {
    Document::new(source).parse().to_source()
}

/// Run the formatter for an already-parsed [`Cli`] and return the process
/// exit code (0 success, 1 `--check` would reformat, 2 error).
#[must_use]
pub fn run(cli: &Cli) -> ExitCode {
    run_args(&cli.args)
}

/// Run the formatter for already-parsed [`FmtArgs`] — the entry point the
/// `aozora fmt` subcommand calls. Returns the same exit codes as [`run`].
#[must_use]
pub fn run_args(args: &FmtArgs) -> ExitCode {
    match dispatch(args) {
        Ok(outcome) => outcome.exit_code(),
        Err(err) => {
            eprintln!("aozora-fmt: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &FmtArgs) -> Result<Outcome> {
    let mode = args.mode();
    match resolve(args.paths())? {
        Input::Stdin => run_stdin(args, &mode),
        Input::Files(resolved) => run_files(args, &mode, &resolved),
    }
}

/// Single-source path: read stdin once, then apply the mode.
fn run_stdin(args: &FmtArgs, mode: &Mode) -> Result<Outcome> {
    let mut old = String::new();
    io::stdin()
        .read_to_string(&mut old)
        .context("reading stdin")?;
    let new = process::format_guarded(&old)?;

    match mode {
        Mode::Stdout => {
            io::stdout().write_all(new.as_bytes())?;
            Ok(Outcome::Ok)
        }
        Mode::Write { .. } => bail!("--write requires a file path, not stdin"),
        Mode::List => {
            if old != new {
                println!("<stdin>");
            }
            Ok(Outcome::Ok)
        }
        Mode::Check(report) => stdin_check(report, args.color(), &old, &new),
    }
}

fn stdin_check(report: &CheckReport, color: ColorChoice, old: &str, new: &str) -> Result<Outcome> {
    let changed = old != new;
    let outcome = if changed {
        Outcome::WouldReformat
    } else {
        Outcome::Ok
    };
    match report {
        CheckReport::Plain => {
            if changed {
                eprintln!("aozora-fmt: <stdin> would be reformatted");
            }
        }
        CheckReport::Diff if changed => {
            let mut out = auto_stdout(color);
            report::write_diff(&mut out, "<stdin>", old, new)?;
            out.flush()?;
        }
        CheckReport::Diff => {}
        CheckReport::Json => {
            let file = if changed {
                report::JsonFile::would_reformat("<stdin>".to_owned())
            } else {
                report::JsonFile::ok("<stdin>".to_owned())
            };
            report::emit_json(outcome, vec![file])?;
        }
    }
    Ok(outcome)
}

/// Multi-source path: dispatch the resolved file set by mode.
fn run_files(args: &FmtArgs, mode: &Mode, resolved: &Resolved) -> Result<Outcome> {
    match mode {
        Mode::Stdout => run_stdout(resolved),
        Mode::Write { list } => Ok(discovery_base(resolved).max(run_write(&resolved.files, *list))),
        Mode::List => Ok(discovery_base(resolved).max(run_list(&resolved.files))),
        Mode::Check(CheckReport::Json) => report::run_check_json(resolved),
        Mode::Check(CheckReport::Diff) => {
            let base = discovery_base(resolved);
            Ok(base.max(run_check(args.color(), &resolved.files, true)?))
        }
        Mode::Check(CheckReport::Plain) => {
            let base = discovery_base(resolved);
            Ok(base.max(run_check(args.color(), &resolved.files, false)?))
        }
    }
}

/// Default stdout mode only makes sense for a single input.
fn run_stdout(resolved: &Resolved) -> Result<Outcome> {
    let base = discovery_base(resolved);
    match resolved.files.as_slice() {
        [] => Ok(base),
        [path] => {
            let fmt = process::read_and_format(path)?;
            io::stdout().write_all(fmt.new.as_bytes())?;
            Ok(base)
        }
        files => bail!(
            "refusing to write {} files to stdout; use --write, --check, or --list",
            files.len()
        ),
    }
}

fn run_write(files: &[PathBuf], list: bool) -> Outcome {
    fold_files(files, |path| {
        let fmt = process::read_and_format(path)?;
        process::write_back(path, &fmt)?;
        if list && fmt.changed() {
            println!("{}", path.display());
        }
        Ok(Outcome::Ok)
    })
}

fn run_list(files: &[PathBuf]) -> Outcome {
    fold_files(files, |path| {
        let fmt = process::read_and_format(path)?;
        if fmt.changed() {
            println!("{}", path.display());
        }
        // gofmt -l is informational: a clean exit even when files are listed.
        Ok(Outcome::Ok)
    })
}

fn run_check(color: ColorChoice, files: &[PathBuf], diff: bool) -> Result<Outcome> {
    if !diff {
        return Ok(fold_files(files, |path| {
            let fmt = process::read_and_format(path)?;
            Ok(if fmt.changed() {
                eprintln!("aozora-fmt: {} would be reformatted", path.display());
                Outcome::WouldReformat
            } else {
                Outcome::Ok
            })
        }));
    }
    let mut out = auto_stdout(color);
    let outcome = fold_files(files, |path| {
        let fmt = process::read_and_format(path)?;
        Ok(if fmt.changed() {
            report::write_diff(&mut out, &path.display().to_string(), &fmt.old, &fmt.new)?;
            Outcome::WouldReformat
        } else {
            Outcome::Ok
        })
    });
    out.flush()?;
    Ok(outcome)
}

/// Run `per_file` over every file, folding outcomes and turning a per-file
/// error into [`Outcome::Error`] (reported to stderr) without aborting the
/// rest of the run.
fn fold_files<F>(files: &[PathBuf], mut per_file: F) -> Outcome
where
    F: FnMut(&Path) -> Result<Outcome>,
{
    let mut outcome = Outcome::Ok;
    for path in files {
        let one = per_file(path).unwrap_or_else(|err| {
            eprintln!("aozora-fmt: {err:#}");
            Outcome::Error
        });
        outcome = outcome.max(one);
    }
    outcome
}

/// Report accumulated discovery errors and seed the run outcome with them.
fn discovery_base(resolved: &Resolved) -> Outcome {
    let mut outcome = Outcome::Ok;
    for err in &resolved.errors {
        eprintln!("aozora-fmt: {err}");
        outcome = Outcome::Error;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_formats_to_empty() {
        assert_eq!(format_source(""), "");
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let input = "hello world\n";
        assert_eq!(format_source(input), input);
    }

    #[test]
    fn format_is_idempotent_on_ruby() {
        let input = "｜青梅《おうめ》へ";
        let once = format_source(input);
        let twice = format_source(&once);
        assert_eq!(once, twice, "second pass must be byte-identical");
    }

    #[test]
    fn format_is_idempotent_on_bouten() {
        let input = "彼は可哀想［＃「可哀想」に傍点］と言った";
        let once = format_source(input);
        let twice = format_source(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn format_is_idempotent_on_page_break() {
        let input = "前\n［＃改ページ］\n後\n";
        let once = format_source(input);
        let twice = format_source(&once);
        assert_eq!(once, twice);
    }
}
