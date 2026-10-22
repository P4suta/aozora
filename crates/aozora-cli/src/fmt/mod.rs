use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use aozora::SerializeOptions;

use crate::i18n::LanguageIdentifier;
use crate::{
    DiagnosticPolicy, DiagnosticReport, DocumentOutcome, diagnostics_render, report_diagnostics,
};
use crate::{output, output::is_broken_pipe};

mod cli;
mod discover;
mod encoding;
mod process;
mod progress;
mod report;
mod source;

pub(crate) use cli::{ColorChoice, FmtArgs};
pub(crate) use discover::{Input, Resolved, resolve};
pub(crate) use encoding::{Encoding, decode};
pub(crate) use process::{read_and_format, write_back};
pub(crate) use source::{read_file, read_stdin};

#[cfg(test)]
pub(crate) use source::{MAX_SOURCE_BYTES, OversizeInput};

use cli::{CheckReport, Mode};
use progress::{Printer, Tally};
use report::{FileOutcome, Outcome, auto_stdout};

#[derive(Clone, Debug)]
pub(crate) struct Presentation {
    pub color: ColorChoice,
    pub quiet: bool,
    pub lang: LanguageIdentifier,
    pub diagnostic_format: diagnostics_render::DiagFormat,
    pub strict: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Ctx {
    encoding: Encoding,
    color: ColorChoice,
    program: &'static str,
    quiet: bool,
    lang: LanguageIdentifier,
    diagnostic_policy: DiagnosticPolicy,
}

#[must_use]
pub(crate) fn run_engine(
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
        diagnostic_policy: DiagnosticPolicy {
            format: presentation.diagnostic_format,
            strict: presentation.strict,
        },
    };
    match dispatch(args, &ctx) {
        Ok(outcome) => outcome.exit_code(),
        Err(err) => ExitCode::from(err_exit_code(&err, program)),
    }
}

// ADR-0029 treats a closed output pipe as successful completion.
fn err_exit_code(err: &anyhow::Error, program: &str) -> u8 {
    if is_broken_pipe(err) {
        0
    } else {
        let _drop = writeln!(io::stderr(), "{program}: {err:#}");
        2
    }
}

fn dispatch(args: &FmtArgs, ctx: &Ctx) -> Result<Outcome> {
    let mode = args.mode();
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

fn run_stdin(args: &FmtArgs, ctx: &Ctx, mode: &Mode) -> Result<Outcome> {
    let raw = read_stdin()?;
    let old = decode(&raw, ctx.encoding).context("decoding stdin")?;
    let (new, diagnostics) = process::format_with_diagnostics(&old, args.serialize_options())?;
    let diagnostic_outcome = diagnostics_outcome(ctx, Path::new("-"), &old, &diagnostics)?;
    if diagnostic_outcome == Outcome::Internal {
        return Ok(diagnostic_outcome);
    }

    let mode_outcome = match mode {
        Mode::Stdout => {
            output::stdout().write_all(new.as_bytes())?;
            Ok(Outcome::Ok)
        }
        Mode::Write { .. } => bail!("--write requires a file path, not stdin"),
        Mode::List => {
            if old != new {
                writeln!(output::stdout(), "<stdin>")?;
            }
            Ok(Outcome::Ok)
        }
        Mode::Check(report) => stdin_check(report, ctx, &old, &new),
    }?;
    Ok(diagnostic_outcome.max(mode_outcome))
}

fn diagnostics_outcome(
    ctx: &Ctx,
    path: &Path,
    source: &str,
    diagnostics: &[aozora::Diagnostic],
) -> Result<Outcome> {
    report_diagnostics(
        ctx.diagnostic_policy,
        DiagnosticReport {
            path,
            source,
            diagnostics,
            lang: &ctx.lang,
        },
    )
    .map(|outcome| match outcome {
        DocumentOutcome::Success => Outcome::Ok,
        DocumentOutcome::Strict => Outcome::Diagnostics,
        DocumentOutcome::Internal => Outcome::Internal,
    })
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
                writeln!(
                    io::stderr(),
                    "{}: <stdin> would be reformatted",
                    ctx.program
                )?;
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
            report::emit_json(vec![file])?;
        }
    }
    Ok(outcome)
}

fn run_files(args: &FmtArgs, ctx: &Ctx, mode: &Mode, resolved: &Resolved) -> Result<Outcome> {
    let opts = args.serialize_options();
    match mode {
        Mode::Stdout => run_stdout(ctx, resolved, opts),
        Mode::Write { list } => {
            Ok(discovery_base(ctx, resolved).max(run_write(ctx, &resolved.files, *list, opts)?))
        }
        Mode::List => Ok(discovery_base(ctx, resolved).max(run_list(ctx, &resolved.files, opts)?)),
        Mode::Check(CheckReport::Json) => report::run_check_json(ctx, resolved, opts),
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

fn run_stdout(ctx: &Ctx, resolved: &Resolved, opts: SerializeOptions) -> Result<Outcome> {
    let base = discovery_base(ctx, resolved);
    match resolved.files.as_slice() {
        [] => Ok(base),
        [path] => {
            let fmt = read_and_format(path, opts, ctx.encoding)?;
            let diagnostics = diagnostics_outcome(ctx, path, &fmt.old, &fmt.diagnostics)?;
            if diagnostics == Outcome::Internal {
                return Ok(base.max(diagnostics));
            }
            output::stdout().write_all(fmt.new.as_bytes())?;
            Ok(base.max(diagnostics))
        }
        files => bail!(
            "refusing to write {} files to stdout; use --write, --check, or --list",
            files.len()
        ),
    }
}

fn run_write(ctx: &Ctx, files: &[PathBuf], list: bool, opts: SerializeOptions) -> Result<Outcome> {
    fold_files(ctx, files, opts, |path, fmt, printer| {
        let diagnostics = diagnostics_outcome(ctx, path, &fmt.old, &fmt.diagnostics)?;
        if diagnostics == Outcome::Internal {
            return Ok(FileOutcome::new(diagnostics, false));
        }
        write_back(path, &fmt, opts)?;
        let changed = fmt.changed();
        if list && changed {
            printer.suspend(|| writeln!(output::stdout(), "{}", path.display()))?;
        }
        Ok(FileOutcome::new(diagnostics, changed))
    })
}

fn run_list(ctx: &Ctx, files: &[PathBuf], opts: SerializeOptions) -> Result<Outcome> {
    fold_files(ctx, files, opts, |path, fmt, printer| {
        let diagnostics = diagnostics_outcome(ctx, path, &fmt.old, &fmt.diagnostics)?;
        if diagnostics == Outcome::Internal {
            return Ok(FileOutcome::new(diagnostics, false));
        }
        let changed = fmt.changed();
        if changed {
            printer.suspend(|| writeln!(output::stdout(), "{}", path.display()))?;
        }
        Ok(FileOutcome::new(diagnostics, changed))
    })
}

fn run_check(ctx: &Ctx, files: &[PathBuf], diff: bool, opts: SerializeOptions) -> Result<Outcome> {
    if !diff {
        return fold_files(ctx, files, opts, |path, fmt, printer| {
            let diagnostics = diagnostics_outcome(ctx, path, &fmt.old, &fmt.diagnostics)?;
            if diagnostics == Outcome::Internal {
                return Ok(FileOutcome::new(diagnostics, false));
            }
            let changed = fmt.changed();
            if changed {
                printer.suspend(|| {
                    writeln!(
                        io::stderr(),
                        "{}: {} would be reformatted",
                        ctx.program,
                        path.display()
                    )
                })?;
            }
            Ok(FileOutcome::new(
                diagnostics.max(check_outcome(changed)),
                changed,
            ))
        });
    }
    let mut out = auto_stdout(ctx.color);
    let outcome = fold_files(ctx, files, opts, |path, fmt, printer| {
        let diagnostics = diagnostics_outcome(ctx, path, &fmt.old, &fmt.diagnostics)?;
        if diagnostics == Outcome::Internal {
            return Ok(FileOutcome::new(diagnostics, false));
        }
        let changed = fmt.changed();
        if changed {
            printer.suspend(|| {
                report::write_diff(&mut out, &path.display().to_string(), &fmt.old, &fmt.new)
            })?;
        }
        Ok(FileOutcome::new(
            diagnostics.max(check_outcome(changed)),
            changed,
        ))
    })?;
    out.flush()?;
    Ok(outcome)
}

fn check_outcome(changed: bool) -> Outcome {
    if changed {
        Outcome::WouldReformat
    } else {
        Outcome::Ok
    }
}

// ADR-0029 propagates broken pipes because every later stdout write would fail too.
fn fold_files<F>(
    ctx: &Ctx,
    files: &[PathBuf],
    opts: SerializeOptions,
    mut per_file: F,
) -> Result<Outcome>
where
    F: FnMut(&Path, process::Formatted, &Printer) -> Result<FileOutcome>,
{
    let bar = progress::file_bar(ctx, files.len());
    let printer = progress::printer(bar.as_ref());
    let mut outcome = Outcome::Ok;
    let mut tally = Tally::default();
    let mut remaining = files;
    while !remaining.is_empty() {
        let (chunk, rest) = remaining.split_at(process::batch_len(remaining));
        let formatted = process::read_and_format_batch(chunk, opts, ctx.encoding);
        for (path, formatted) in chunk.iter().zip(formatted) {
            if let Some(bar) = &bar {
                bar.set_message(path.display().to_string());
            }
            let one = match formatted.and_then(|fmt| per_file(path, fmt, &printer)) {
                Ok(one) => {
                    tally.record(one.changed);
                    one.outcome
                }
                Err(err) if is_broken_pipe(&err) => return Err(err),
                Err(err) => {
                    printer.suspend(|| writeln!(io::stderr(), "{}: {err:#}", ctx.program))?;
                    tally.record_error();
                    Outcome::Error
                }
            };
            outcome = outcome.max(one);
            if let Some(bar) = &bar {
                bar.inc(1);
            }
        }
        remaining = rest;
    }
    if let Some(bar) = bar {
        bar.finish_and_clear();
    }
    progress::summary(ctx, &tally);
    Ok(outcome)
}

fn discovery_base(ctx: &Ctx, resolved: &Resolved) -> Outcome {
    let mut outcome = Outcome::Ok;
    for err in &resolved.errors {
        let _drop = writeln!(io::stderr(), "{}: {err}", ctx.program);
        outcome = Outcome::Error;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use std::{fs, io};

    use aozora::DirectiveNormalization;
    use aozora::fmt::{format_source, format_source_with};

    use super::*;
    use crate::i18n::resolve;

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    fn stdout_broken_pipe_error() -> anyhow::Error {
        let mut stdout = output::guard(BrokenPipeWriter);
        anyhow::Error::new(stdout.write_all(b"x").unwrap_err())
    }

    #[test]
    fn empty_input_formats_to_empty() {
        assert_eq!(format_source(""), "");
    }

    #[test]
    fn is_broken_pipe_finds_marked_epipe_through_context() {
        let err = stdout_broken_pipe_error().context("failed to write to stdout");
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
        let pipe = stdout_broken_pipe_error();
        assert_eq!(
            err_exit_code(&pipe, "aozora fmt"),
            0,
            "a broken output pipe is a silent success (exit 0)",
        );
    }

    #[test]
    fn err_exit_code_is_two_for_unmarked_broken_pipe() {
        let pipe = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe));
        assert_eq!(err_exit_code(&pipe, "aozora fmt"), 2);
    }

    #[test]
    fn err_exit_code_is_two_for_other_errors() {
        let other = anyhow::anyhow!("boom");
        assert_eq!(
            err_exit_code(&other, "aozora fmt"),
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
        let fix = SerializeOptions::default().directives(DirectiveNormalization::Canonical);
        let near_miss = "あ［＃字下げ終わり］";
        assert!(
            format_source(near_miss).contains("［＃字下げ終わり］"),
            "default fmt must not rewrite notation"
        );
        let fixed = format_source_with(near_miss, fix);
        assert!(
            fixed.contains("［＃ここで字下げ終わり］"),
            "fix should canonicalise the directive; got {fixed:?}"
        );
        let editorial = "あ［＃底本では「蒼空」］";
        assert!(
            format_source_with(editorial, fix).contains("［＃底本では「蒼空」］"),
            "fix must not touch genuine editorial Unknowns"
        );
    }

    fn ctx() -> Ctx {
        Ctx {
            encoding: Encoding::default(),
            color: ColorChoice::Never,
            program: "aozora fmt",
            quiet: false,
            lang: resolve(None, None, None, None),
            diagnostic_policy: DiagnosticPolicy {
                format: diagnostics_render::DiagFormat::Short,
                strict: false,
            },
        }
    }

    fn fold_fixture_files(contents: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = contents
            .iter()
            .enumerate()
            .map(|(index, content)| {
                let path = dir.path().join(format!("{index}.aozora"));
                fs::write(&path, content).expect("write fixture");
                path
            })
            .collect();
        (dir, files)
    }

    #[test]
    fn fold_files_propagates_broken_pipe_as_err() {
        let (_dir, files) = fold_fixture_files(&["first", "second"]);
        let mut calls = 0_u32;
        let result = fold_files(
            &ctx(),
            &files,
            SerializeOptions::default(),
            |_path, _formatted, _printer| {
                calls += 1;
                Err(stdout_broken_pipe_error())
            },
        );
        let err = result.expect_err("broken pipe must propagate as Err, not fold into an Outcome");
        assert!(
            is_broken_pipe(&err),
            "the propagated error must still carry the broken pipe"
        );
        assert_eq!(calls, 1, "a broken pipe is terminal for the whole run");
    }

    #[test]
    fn fold_files_downgrades_non_pipe_errors_to_outcome_error() {
        let (_dir, files) = fold_fixture_files(&["only"]);
        let result = fold_files(
            &ctx(),
            &files,
            SerializeOptions::default(),
            |_path, _formatted, _printer| {
                Err(anyhow::anyhow!("permission denied or parse failure"))
            },
        );
        let outcome = result.expect("a non-pipe error must be downgraded, not propagated");
        assert_eq!(outcome, Outcome::Error);
    }
}
