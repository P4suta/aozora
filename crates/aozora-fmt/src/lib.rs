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

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use aozora::Document;
use aozora::render::SerializeOptions;
use aozora_i18n::LanguageIdentifier;

mod cli;
mod discover;
mod encoding;
mod process;
mod progress;
mod report;
mod source;

pub use cli::Cli;

// Public CLI surface, re-exported for the standalone `aozora-fmt` binary
// (`src/main.rs`), the `aozora` CLI's `fmt`/`lint` subcommands, and this
// crate's integration tests.
pub use cli::{ColorChoice, FmtArgs};
pub use discover::{Input, Resolved, resolve};
pub use encoding::{Encoding, decode};
pub use process::{Formatted, Panicked, guard, read_and_format, write_back};
pub use report::auto_stdout;
pub use source::{MAX_SOURCE_BYTES, OversizeInput, is_oversize_input, read_file, read_stdin};

/// Compiles and runs the fenced Rust example in this crate's `README.md` as a
/// doctest, so the documented public API (`format_source`) can't silently
/// drift from the code. `#[cfg(doctest)]` means the item exists only while
/// rustdoc collects doctests — it never reaches a normal build, so neither
/// `dead_code` nor `missing_debug_implementations` fire on it.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

use cli::{CheckReport, Mode};
use progress::{Printer, Tally};
use report::{FileOutcome, Outcome};

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

/// Does `err`, or any error in its `source` chain, carry a broken-pipe
/// [`io::Error`]?
///
/// When a downstream reader closes the pipe early — the canonical
/// `aozora render big.txt | head` — the next write to stdout fails with
/// [`io::ErrorKind::BrokenPipe`]. Both CLI frontends treat that as a normal,
/// silent success (exit 0) rather than an error, matching `ripgrep` and `bat`.
/// The error may be wrapped by `anyhow` context, so the whole chain is
/// searched. See ADR-0029.
#[must_use]
pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io_err| io_err.kind() == io::ErrorKind::BrokenPipe)
    })
}

/// The caller-injected human-output presentation policy for a run.
///
/// Bundles the three presentation choices both frontends inject — how to
/// colourise terminal output (`color`, governing `--diff` hunks and the
/// progress UI), whether to suppress the directory-batch progress UI + summary
/// (`quiet`), and the message language for that localized summary (`lang`) — so
/// there is one shared policy with no per-frontend behaviour. It is a
/// parameter object: grouping these keeps [`run_engine`] within the project's
/// argument-count budget and documents the presentation surface in one place.
#[derive(Clone, Debug)]
pub struct Presentation {
    /// When to emit ANSI colour in `--diff` output (and any future coloured UI).
    pub color: ColorChoice,
    /// Suppress the stderr progress UI and batch summary (mirrors `--quiet`).
    pub quiet: bool,
    /// Message language for the localized batch summary.
    pub lang: LanguageIdentifier,
}

/// Constant-per-run engine context: how to decode inputs, the program name to
/// prefix diagnostics with (`aozora-fmt` for the standalone binary, `aozora
/// fmt` for the `aozora` CLI subcommand), and the caller's [`Presentation`]
/// policy unpacked into flat fields. Threaded through the engine so both
/// frontends share one implementation with no lexical special-casing.
#[derive(Clone, Debug)]
struct Ctx {
    encoding: Encoding,
    color: ColorChoice,
    program: &'static str,
    /// Suppress the stderr progress UI and batch summary.
    quiet: bool,
    /// Message language for the localized batch summary.
    lang: LanguageIdentifier,
}

/// Run the formatter for an already-parsed [`Cli`], returning the exit code.
///
/// The standalone binary's entry point (0 success, 1 `--check` would reformat,
/// 2 error). Resolves the encoding from `-E/--encoding` (else auto) and labels
/// diagnostics `aozora-fmt`. The standalone binary carries no `--quiet` flag,
/// so its TTY-gated batch UI is never suppressed; it has no `--lang` flag or
/// config layer either, so its summary language comes from the two environment
/// sources the `aozora` CLI shares — `AOZORA_LANG`, then `LANG` (message
/// language only, never encoding; ADR-0033) — English otherwise.
#[must_use]
pub fn run(cli: &Cli) -> ExitCode {
    let presentation = Presentation {
        color: cli.color,
        quiet: false,
        lang: aozora_i18n::resolve(
            None,
            env::var("AOZORA_LANG").ok().as_deref(),
            None,
            env::var("LANG").ok().as_deref(),
        ),
    };
    run_engine(
        &cli.args,
        cli.args.encoding().unwrap_or_default(),
        "aozora-fmt",
        &presentation,
    )
}

/// Run the formatter engine for already-parsed [`FmtArgs`] under an explicit
/// `encoding`, `color`, and `program` label, returning the exit code
/// (0 / 1 / 2).
///
/// This is the single entry both frontends share: `encoding`, `program`, and
/// the [`Presentation`] policy (colour + `--quiet` + language) are
/// caller-injected so there is one implementation and no per-frontend policy.
/// The `aozora` CLI's `fmt` subcommand calls it after folding `.aozora.toml`,
/// passing its config-resolved encoding, `"aozora fmt"`, and a `Presentation`
/// built from its global `--color` / `--quiet` and the resolved language; the
/// standalone binary uses [`run`].
#[must_use]
pub fn run_engine(
    args: &FmtArgs,
    encoding: Encoding,
    program: &'static str,
    presentation: &Presentation,
) -> ExitCode {
    let ctx = Ctx {
        encoding,
        color: presentation.color,
        program,
        quiet: presentation.quiet,
        lang: presentation.lang.clone(),
    };
    match dispatch(args, &ctx) {
        Ok(outcome) => outcome.exit_code(),
        Err(err) => ExitCode::from(err_exit_code(&err, program)),
    }
}

/// Map a failed dispatch to its numeric process exit code, applying the
/// broken-pipe policy. Split out of the `ExitCode`-returning [`run_engine`] so
/// the decision is unit-testable against a plain, comparable `u8`.
///
/// A reader that closed our stdout pipe early (`aozora fmt … | head`) is a
/// normal, silent success — exit 0, not an error; see ADR-0029. Every other
/// error is a genuine failure: logged to stderr (prefixed with `program`) and
/// mapped to exit 2.
fn err_exit_code(err: &anyhow::Error, program: &str) -> u8 {
    if is_broken_pipe(err) {
        0
    } else {
        eprintln!("{program}: {err:#}");
        2
    }
}

fn dispatch(args: &FmtArgs, ctx: &Ctx) -> Result<Outcome> {
    let mode = args.mode();
    // Directory discovery can walk a large tree; show an indeterminate spinner
    // while it runs (auto-hidden for fast walks by the tick delay). Clear it
    // before touching the result so no spinner residue precedes the output.
    let spinner = progress::discovery_spinner(ctx, &mode);
    let input = resolve(args.paths());
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }
    match input? {
        Input::Stdin => run_stdin(args, ctx, &mode),
        Input::Files(resolved) => run_files(args, ctx, &mode, &resolved),
    }
}

/// Single-source path: read stdin once, then apply the mode.
fn run_stdin(args: &FmtArgs, ctx: &Ctx, mode: &Mode) -> Result<Outcome> {
    let raw = read_stdin()?;
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
                writeln!(io::stdout(), "<stdin>")?;
            }
            Ok(Outcome::Ok)
        }
        Mode::Check(report) => stdin_check(report, ctx, &old, &new),
    }
}

fn stdin_check(report: &CheckReport, ctx: &Ctx, old: &str, new: &str) -> Result<Outcome> {
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
fn run_files(args: &FmtArgs, ctx: &Ctx, mode: &Mode, resolved: &Resolved) -> Result<Outcome> {
    let opts = args.serialize_options();
    match mode {
        Mode::Stdout => run_stdout(ctx, resolved, opts),
        Mode::Write { list } => {
            Ok(discovery_base(ctx, resolved).max(run_write(ctx, &resolved.files, *list, opts)?))
        }
        Mode::List => Ok(discovery_base(ctx, resolved).max(run_list(ctx, &resolved.files, opts)?)),
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
fn run_stdout(ctx: &Ctx, resolved: &Resolved, opts: SerializeOptions) -> Result<Outcome> {
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

fn run_write(ctx: &Ctx, files: &[PathBuf], list: bool, opts: SerializeOptions) -> Result<Outcome> {
    fold_files(ctx, files, |path, printer| {
        let fmt = read_and_format(path, opts, ctx.encoding)?;
        write_back(path, &fmt, opts)?;
        let changed = fmt.changed();
        if list && changed {
            printer.suspend(|| writeln!(io::stdout(), "{}", path.display()))?;
        }
        Ok(FileOutcome::new(Outcome::Ok, changed))
    })
}

fn run_list(ctx: &Ctx, files: &[PathBuf], opts: SerializeOptions) -> Result<Outcome> {
    fold_files(ctx, files, |path, printer| {
        let fmt = read_and_format(path, opts, ctx.encoding)?;
        let changed = fmt.changed();
        if changed {
            printer.suspend(|| writeln!(io::stdout(), "{}", path.display()))?;
        }
        // gofmt -l is informational: a clean exit even when files are listed.
        Ok(FileOutcome::new(Outcome::Ok, changed))
    })
}

fn run_check(ctx: &Ctx, files: &[PathBuf], diff: bool, opts: SerializeOptions) -> Result<Outcome> {
    if !diff {
        return fold_files(ctx, files, |path, printer| {
            let fmt = read_and_format(path, opts, ctx.encoding)?;
            let changed = fmt.changed();
            if changed {
                printer.suspend(|| {
                    eprintln!("{}: {} would be reformatted", ctx.program, path.display());
                });
            }
            Ok(FileOutcome::new(check_outcome(changed), changed))
        });
    }
    let mut out = auto_stdout(ctx.color);
    let outcome = fold_files(ctx, files, |path, printer| {
        let fmt = read_and_format(path, opts, ctx.encoding)?;
        let changed = fmt.changed();
        if changed {
            printer.suspend(|| {
                report::write_diff(&mut out, &path.display().to_string(), &fmt.old, &fmt.new)
            })?;
        }
        Ok(FileOutcome::new(check_outcome(changed), changed))
    })?;
    out.flush()?;
    Ok(outcome)
}

/// A `--check` file's outcome: a changed file is a would-reformat (exit 1),
/// an already-canonical one is clean.
fn check_outcome(changed: bool) -> Outcome {
    if changed {
        Outcome::WouldReformat
    } else {
        Outcome::Ok
    }
}

/// Run `per_file` over every file, folding outcomes and turning a per-file
/// error into [`Outcome::Error`] (reported to stderr) without aborting the
/// rest of the run.
///
/// This is the directory-batch loop and it owns the batch UI: a TTY-gated
/// [`indicatif`](progress) progress bar advanced per file (with the current
/// file as its message), and the localized "N formatted, M unchanged, K
/// errors" summary printed to stderr on completion. The bar and summary are
/// no-ops off an interactive stderr, so a piped run's byte stream is untouched.
/// The per-file closure receives a [`Printer`] so its own stdout/stderr lines
/// interleave cleanly with the live bar.
///
/// A broken output pipe is the one exception: it is terminal for the whole run
/// (every later stdout write would fail too), so it propagates as `Err` for
/// [`run_engine`] to turn into a quiet exit 0 rather than being logged per-file
/// and downgraded to [`Outcome::Error`] (exit 2). See ADR-0029. On that early
/// return neither the bar's tail nor the summary is drawn — the pipe is gone.
fn fold_files<F>(ctx: &Ctx, files: &[PathBuf], mut per_file: F) -> Result<Outcome>
where
    F: FnMut(&Path, &Printer) -> Result<FileOutcome>,
{
    let bar = progress::file_bar(ctx, files.len());
    let printer = progress::printer(bar.as_ref());
    let mut outcome = Outcome::Ok;
    let mut tally = Tally::default();
    for path in files {
        if let Some(bar) = &bar {
            bar.set_message(path.display().to_string());
        }
        let one = match per_file(path, &printer) {
            Ok(one) => {
                tally.record(one.changed);
                one.outcome
            }
            Err(err) if is_broken_pipe(&err) => return Err(err),
            Err(err) => {
                printer.suspend(|| eprintln!("{}: {err:#}", ctx.program));
                tally.record_error();
                Outcome::Error
            }
        };
        outcome = outcome.max(one);
        if let Some(bar) = &bar {
            bar.inc(1);
        }
    }
    if let Some(bar) = bar {
        bar.finish_and_clear();
    }
    progress::summary(ctx, &tally);
    Ok(outcome)
}

/// Report accumulated discovery errors and seed the run outcome with them.
fn discovery_base(ctx: &Ctx, resolved: &Resolved) -> Outcome {
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
    fn is_broken_pipe_finds_epipe_through_context() {
        // The io::Error is wrapped in an anyhow context, mirroring the CLI's
        // `write_all(..).context("failed to write to stdout")?` path.
        let err = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe))
            .context("failed to write to stdout");
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn is_broken_pipe_rejects_other_errors() {
        let other = anyhow::Error::new(io::Error::from(io::ErrorKind::PermissionDenied))
            .context("failed to write to stdout");
        assert!(!is_broken_pipe(&other));
        assert!(!is_broken_pipe(&anyhow::anyhow!("plain non-io error")));
    }

    #[test]
    fn err_exit_code_is_zero_for_broken_pipe() {
        // ADR-0029: a reader that closed our stdout pipe early is a silent
        // success, so `err_exit_code` must map a broken-pipe error to 0 — never
        // the error code 2. With the `is_broken_pipe` guard forced false the
        // pipe error would fall to the logging arm and return 2, so pinning 0
        // kills that mutant (and the whole-body mutant that returns 1).
        let pipe = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe));
        assert_eq!(
            err_exit_code(&pipe, "aozora-fmt"),
            0,
            "a broken output pipe is a silent success (exit 0)",
        );
    }

    #[test]
    fn err_exit_code_is_two_for_other_errors() {
        // Any non-pipe error is a genuine failure: logged to stderr and mapped
        // to exit 2. With the `is_broken_pipe` guard forced true this would
        // instead return 0, so pinning 2 kills that mutant (and the whole-body
        // mutant that returns 0).
        let other = anyhow::anyhow!("boom");
        assert_eq!(
            err_exit_code(&other, "aozora-fmt"),
            2,
            "a non-pipe error is a genuine failure (exit 2)",
        );
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

    /// The neutral engine context each `fold_files` test threads through.
    /// The tests run off a terminal, so the batch UI is gated out regardless
    /// of `quiet`; `lang` is the English default.
    fn ctx() -> Ctx {
        Ctx {
            encoding: Encoding::default(),
            color: ColorChoice::Never,
            program: "aozora-fmt",
            quiet: false,
            lang: aozora_i18n::resolve(None, None, None, None),
        }
    }

    #[test]
    fn fold_files_propagates_broken_pipe_as_err() {
        // A broken output pipe (`aozora fmt … | head`) is terminal for the whole
        // run: `fold_files` must abort by returning the `Err` — which
        // `run_engine` turns into a quiet exit 0 — never swallowing it into
        // `Outcome::Error`. With the `is_broken_pipe` guard replaced by `false`
        // the pipe error would instead fall to the logging arm and fold into
        // `Ok(Outcome::Error)`, so pinning `Err`/broken-pipe kills that mutant.
        let files = [PathBuf::from("first"), PathBuf::from("second")];
        let mut calls = 0_u32;
        let result = fold_files(&ctx(), &files, |_path, _printer| {
            calls += 1;
            Err(anyhow::Error::new(io::Error::from(
                io::ErrorKind::BrokenPipe,
            )))
        });
        let err = result.expect_err("broken pipe must propagate as Err, not fold into an Outcome");
        assert!(
            is_broken_pipe(&err),
            "the propagated error must still carry the broken pipe"
        );
        // It aborts on the first file rather than continuing the fold.
        assert_eq!(calls, 1, "a broken pipe is terminal for the whole run");
    }

    #[test]
    fn fold_files_downgrades_non_pipe_errors_to_outcome_error() {
        // A per-file error that is *not* a broken pipe must be logged to stderr
        // and downgraded to `Outcome::Error` without aborting the rest of the
        // run — returned as `Ok(Outcome::Error)`, not `Err`. With the
        // `is_broken_pipe` guard replaced by `true` every error would propagate
        // as `Err`, so pinning the `Ok(Outcome::Error)` return kills that mutant.
        let files = [PathBuf::from("only")];
        let result = fold_files(&ctx(), &files, |_path, _printer| {
            Err(anyhow::anyhow!("permission denied or parse failure"))
        });
        let outcome = result.expect("a non-pipe error must be downgraded, not propagated");
        assert_eq!(outcome, Outcome::Error);
    }
}
