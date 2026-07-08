//! `aozora-fmt` library: [`format_source`] runs the `parse ∘ to_source`
//! round-trip that produces an idempotent, canonicalised aozora document.
//! Every consumer — the standalone `aozora-fmt` binary, the `aozora` CLI's
//! `fmt` subcommand, and the CI/test gates that cross-check against it —
//! reaches the same canonical form; the round-trip is a fixed point on the
//! second pass.
//!
//! The standalone binary's clap surface ([`Cli`], [`run`]) lives here too so
//! `src/main.rs` stays a thin shim over it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use aozora::Document;
use aozora::render::SerializeOptions;

mod cli;
mod discover;
mod encoding;
mod process;
mod report;

pub use cli::Cli;

// Public CLI surface, re-exported for the standalone `aozora-fmt` binary
// (`src/main.rs`), the `aozora` CLI's `fmt`/`lint` subcommands, and this
// crate's integration tests.
pub use cli::{ColorChoice, FmtArgs};
pub use discover::{Input, Resolved, resolve};
pub use encoding::{Encoding, decode};
pub use process::{Formatted, Panicked, guard, read_and_format, write_back};
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
    format_source_with(source, SerializeOptions::default())
}

/// Canonicalise an aozora source string under explicit [`SerializeOptions`].
///
/// With the default options this equals [`format_source`]. With
/// `DirectiveNormalization::Canonical` it additionally rewrites non-canonical
/// directive near-misses to their canonical spelling — the `--fix` autofix —
/// which stays a second-pass fixed point (the canonical form parses to a
/// recognized node and is not rewritten again).
#[must_use]
pub fn format_source_with(source: &str, opts: SerializeOptions) -> String {
    Document::new(source).parse().to_source_with(opts)
}

/// Constant-per-run engine context: how to decode inputs and the program name
/// to prefix diagnostics with (`aozora-fmt` for the standalone binary,
/// `aozora fmt` for the `aozora` CLI subcommand). Threaded through the engine
/// so both frontends share one implementation with no lexical special-casing.
#[derive(Copy, Clone)]
struct Ctx {
    encoding: Encoding,
    color: ColorChoice,
    program: &'static str,
}

/// Run the formatter for an already-parsed [`Cli`], returning the exit code.
///
/// The standalone binary's entry point (0 success, 1 `--check` would reformat,
/// 2 error). Resolves the encoding from `-E/--encoding` (else auto) and labels
/// diagnostics `aozora-fmt`.
#[must_use]
pub fn run(cli: &Cli) -> ExitCode {
    run_engine(
        &cli.args,
        cli.args.encoding().unwrap_or_default(),
        cli.color,
        "aozora-fmt",
    )
}

/// Run the formatter engine for already-parsed [`FmtArgs`] under an explicit
/// `encoding`, `color`, and `program` label, returning the exit code
/// (0 / 1 / 2).
///
/// This is the single entry both frontends share: `encoding`, `color`, and
/// `program` are caller-injected so there is one implementation and no
/// per-frontend policy. The `aozora` CLI's `fmt` subcommand calls it after
/// folding `.aozora.toml`, passing its config-resolved encoding, its global
/// `--color`, and `"aozora fmt"`; the standalone binary uses [`run`].
#[must_use]
pub fn run_engine(
    args: &FmtArgs,
    encoding: Encoding,
    color: ColorChoice,
    program: &'static str,
) -> ExitCode {
    let ctx = Ctx {
        encoding,
        color,
        program,
    };
    match dispatch(args, ctx) {
        Ok(outcome) => outcome.exit_code(),
        Err(err) => {
            eprintln!("{program}: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &FmtArgs, ctx: Ctx) -> Result<Outcome> {
    let mode = args.mode();
    match resolve(args.paths())? {
        Input::Stdin => run_stdin(args, ctx, &mode),
        Input::Files(resolved) => run_files(args, ctx, &mode, &resolved),
    }
}

/// Single-source path: read stdin once, then apply the mode.
fn run_stdin(args: &FmtArgs, ctx: Ctx, mode: &Mode) -> Result<Outcome> {
    let mut raw = Vec::new();
    io::stdin().read_to_end(&mut raw).context("reading stdin")?;
    let old = decode(&raw, ctx.encoding).context("decoding stdin")?;
    let new = process::format_guarded(&old, args.serialize_options())?;

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
        Mode::Check(report) => stdin_check(report, ctx, &old, &new),
    }
}

fn stdin_check(report: &CheckReport, ctx: Ctx, old: &str, new: &str) -> Result<Outcome> {
    let changed = old != new;
    let outcome = if changed {
        Outcome::WouldReformat
    } else {
        Outcome::Ok
    };
    match report {
        CheckReport::Plain => {
            if changed {
                eprintln!("{}: <stdin> would be reformatted", ctx.program);
            }
        }
        CheckReport::Diff if changed => {
            let mut out = auto_stdout(ctx.color);
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
fn run_files(args: &FmtArgs, ctx: Ctx, mode: &Mode, resolved: &Resolved) -> Result<Outcome> {
    let opts = args.serialize_options();
    match mode {
        Mode::Stdout => run_stdout(ctx, resolved, opts),
        Mode::Write { list } => {
            Ok(discovery_base(ctx, resolved).max(run_write(ctx, &resolved.files, *list, opts)))
        }
        Mode::List => Ok(discovery_base(ctx, resolved).max(run_list(ctx, &resolved.files, opts))),
        Mode::Check(CheckReport::Json) => report::run_check_json(ctx.encoding, resolved, opts),
        Mode::Check(CheckReport::Diff) => {
            let base = discovery_base(ctx, resolved);
            Ok(base.max(run_check(ctx, &resolved.files, true, opts)?))
        }
        Mode::Check(CheckReport::Plain) => {
            let base = discovery_base(ctx, resolved);
            Ok(base.max(run_check(ctx, &resolved.files, false, opts)?))
        }
    }
}

/// Default stdout mode only makes sense for a single input.
fn run_stdout(ctx: Ctx, resolved: &Resolved, opts: SerializeOptions) -> Result<Outcome> {
    let base = discovery_base(ctx, resolved);
    match resolved.files.as_slice() {
        [] => Ok(base),
        [path] => {
            let fmt = read_and_format(path, opts, ctx.encoding)?;
            io::stdout().write_all(fmt.new.as_bytes())?;
            Ok(base)
        }
        files => bail!(
            "refusing to write {} files to stdout; use --write, --check, or --list",
            files.len()
        ),
    }
}

fn run_write(ctx: Ctx, files: &[PathBuf], list: bool, opts: SerializeOptions) -> Outcome {
    fold_files(ctx, files, |path| {
        let fmt = read_and_format(path, opts, ctx.encoding)?;
        write_back(path, &fmt, opts)?;
        if list && fmt.changed() {
            println!("{}", path.display());
        }
        Ok(Outcome::Ok)
    })
}

fn run_list(ctx: Ctx, files: &[PathBuf], opts: SerializeOptions) -> Outcome {
    fold_files(ctx, files, |path| {
        let fmt = read_and_format(path, opts, ctx.encoding)?;
        if fmt.changed() {
            println!("{}", path.display());
        }
        // gofmt -l is informational: a clean exit even when files are listed.
        Ok(Outcome::Ok)
    })
}

fn run_check(ctx: Ctx, files: &[PathBuf], diff: bool, opts: SerializeOptions) -> Result<Outcome> {
    if !diff {
        return Ok(fold_files(ctx, files, |path| {
            let fmt = read_and_format(path, opts, ctx.encoding)?;
            Ok(if fmt.changed() {
                eprintln!("{}: {} would be reformatted", ctx.program, path.display());
                Outcome::WouldReformat
            } else {
                Outcome::Ok
            })
        }));
    }
    let mut out = auto_stdout(ctx.color);
    let outcome = fold_files(ctx, files, |path| {
        let fmt = read_and_format(path, opts, ctx.encoding)?;
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
fn fold_files<F>(ctx: Ctx, files: &[PathBuf], mut per_file: F) -> Outcome
where
    F: FnMut(&Path) -> Result<Outcome>,
{
    let mut outcome = Outcome::Ok;
    for path in files {
        let one = per_file(path).unwrap_or_else(|err| {
            eprintln!("{}: {err:#}", ctx.program);
            Outcome::Error
        });
        outcome = outcome.max(one);
    }
    outcome
}

/// Report accumulated discovery errors and seed the run outcome with them.
fn discovery_base(ctx: Ctx, resolved: &Resolved) -> Outcome {
    let mut outcome = Outcome::Ok;
    for err in &resolved.errors {
        eprintln!("{}: {err}", ctx.program);
        outcome = Outcome::Error;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora::render::DirectiveNormalization;

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

    #[test]
    fn fix_rewrites_flagged_near_miss_only_when_opted_in() {
        let fix = SerializeOptions {
            directives: DirectiveNormalization::Canonical,
        };
        let near_miss = "あ［＃字下げ終わり］";
        // Default fmt keeps the flagged near-miss verbatim.
        assert!(
            format_source(near_miss).contains("［＃字下げ終わり］"),
            "default fmt must not rewrite notation"
        );
        // Opt-in rewrites it to the canonical spelling.
        let fixed = format_source_with(near_miss, fix);
        assert!(
            fixed.contains("［＃ここで字下げ終わり］"),
            "fix should canonicalise the directive; got {fixed:?}"
        );
        // A genuine editorial Unknown is left untouched even with the flag.
        let editorial = "あ［＃底本では「蒼空」］";
        assert!(
            format_source_with(editorial, fix).contains("［＃底本では「蒼空」］"),
            "fix must not touch genuine editorial Unknowns"
        );
    }
}
