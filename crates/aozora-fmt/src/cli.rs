//! Command-line surface for `aozora-fmt`: the clap [`Cli`] plus the [`Mode`]
//! it derives once so the rest of the pipeline never juggles raw booleans.
//!
//! `Cli` is re-exported from the crate root so `src/main.rs` — the thin
//! standalone-binary shim — can parse and run it.

use std::path::{Path, PathBuf};

use aozora::render::{DirectiveNormalization, SerializeOptions};
use clap::{Args, Parser, ValueEnum};

use crate::encoding::Encoding;

const LONG_ABOUT: &str = concat!(
    "Idempotent formatter for aozora-flavored-markdown.\n\n",
    "With no path (or `-`) it reads stdin and writes the canonical form to ",
    "stdout. Given files or directories it can check, rewrite, list, or diff ",
    "them; directories are searched recursively for *.afm, *.aozora, and ",
    "*.aozora.txt files.\n\n",
    "Exit codes: 0 = success / already formatted, 1 = --check found inputs ",
    "that would change, 2 = an error occurred.",
);

/// Idempotent formatter for aozora-flavored-markdown.
///
/// A thin [`Parser`] newtype around [`FmtArgs`] for the standalone
/// `aozora-fmt` binary (`src/main.rs`). The `aozora` CLI's `fmt` subcommand is
/// a separate frontend with its own arguments; it shares this crate's
/// formatting core ([`crate::format_source_with`]), not these flags.
#[derive(Parser, Debug)]
#[command(
    name = "aozora-fmt",
    about = "Idempotent formatter for aozora-flavored-markdown",
    long_about = LONG_ABOUT,
    version
)]
pub struct Cli {
    /// The formatter flags, shared with `aozora fmt`.
    #[command(flatten)]
    pub(crate) args: FmtArgs,

    /// When to colourise --diff output. (The `aozora` CLI supplies this from
    /// its own global `--color`; the standalone binary owns the flag here so
    /// colour stays a single caller-injected policy, not a per-frontend one.)
    #[arg(long, value_name = "WHEN", default_value = "auto")]
    pub(crate) color: ColorChoice,
}

/// The formatter's argument surface for the standalone `aozora-fmt` binary
/// ([`Cli`]). The `aozora fmt` subcommand defines its own arguments.
#[derive(Args, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a clap flag struct: each bool is an independent CLI switch, collapsed into Mode by mode()"
)]
pub struct FmtArgs {
    /// Files or directories to format. Use `-`, or omit, to read stdin.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Verify inputs are already formatted; exit 1 if any would change.
    #[arg(long, conflicts_with_all = ["write", "list"])]
    check: bool,

    /// Rewrite files in place (no-op when already canonical).
    #[arg(long, short = 'w', conflicts_with_all = ["check", "diff", "json"])]
    write: bool,

    /// Print a unified diff for every file that would change. Implies --check.
    #[arg(long, conflicts_with_all = ["write", "list", "json"])]
    diff: bool,

    /// List only the paths that would change (gofmt -l). Combine with -w.
    #[arg(long, short = 'l', conflicts_with_all = ["check", "diff", "json"])]
    list: bool,

    /// Emit the --check result as machine-readable JSON. Implies --check.
    #[arg(long, conflicts_with_all = ["write", "list", "diff"])]
    json: bool,

    /// Source encoding. Falls back to `AOZORA_ENCODING`, then auto-detection
    /// (valid UTF-8 as-is, otherwise Shift_JIS — the Aozora Bunko default).
    #[arg(long, short = 'E', value_name = "ENCODING", env = "AOZORA_ENCODING")]
    encoding: Option<Encoding>,

    /// Rewrite non-canonical directive near-misses to their canonical
    /// spelling (e.g. `［＃字下げ終わり］` → `［＃ここで字下げ終わり］`), the
    /// fixes flagged by the `aozora::lint::non_canonical_directive`
    /// warnings. Opt-in: without it every directive round-trips its raw
    /// bytes verbatim. Idempotent, so it composes with every mode.
    #[arg(long)]
    fix: bool,
}

/// When to emit ANSI colour in terminal output (diffs, diagnostics, …).
///
/// Re-exported from the crate root so the `aozora` CLI's `lint`/`render`
/// subcommands share one colour policy with the formatter.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    /// Colour when stdout is a terminal (honours `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE`).
    Auto,
    /// Always colour, even when piped.
    Always,
    /// Never colour.
    Never,
}

/// What a run should do, derived once from the parsed flags.
pub(crate) enum Mode {
    /// Pipe the canonical form to stdout (valid for a single input only).
    Stdout,
    /// Rewrite changed files in place; `list` also prints them (gofmt -l -w).
    Write { list: bool },
    /// Verify formatting; the [`CheckReport`] controls what is printed.
    Check(CheckReport),
    /// Print only the paths that would change (gofmt -l).
    List,
}

/// How `--check` reports files that are not already formatted.
pub(crate) enum CheckReport {
    /// One `<path> would be reformatted` line per file, on stderr.
    Plain,
    /// A coloured unified diff per file, on stdout.
    Diff,
    /// A single JSON object on stdout.
    Json,
}

impl FmtArgs {
    /// The positional path arguments (possibly including `-` for stdin).
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The `-E/--encoding` override, if any (else the caller's default —
    /// `AOZORA_ENCODING`, the `.aozora.toml` key, or auto-detection).
    #[must_use]
    pub fn encoding(&self) -> Option<Encoding> {
        self.encoding
    }

    /// The serialization options derived from the flags — currently just the
    /// `--fix` autofix opt-in, which applies zero-false-positive Tier1 only
    /// (`Canonical`); the lossy Tier2 reductions are render-only (ADR-0026).
    pub(crate) fn serialize_options(&self) -> SerializeOptions {
        SerializeOptions {
            directives: if self.fix {
                DirectiveNormalization::Canonical
            } else {
                DirectiveNormalization::Off
            },
        }
    }

    /// Collapse the mutually-exclusive flags into a single [`Mode`].
    pub(crate) fn mode(&self) -> Mode {
        if self.write {
            Mode::Write { list: self.list }
        } else if self.list {
            Mode::List
        } else if self.diff {
            Mode::Check(CheckReport::Diff)
        } else if self.json {
            Mode::Check(CheckReport::Json)
        } else if self.check {
            Mode::Check(CheckReport::Plain)
        } else {
            Mode::Stdout
        }
    }
}

/// True if `path` is the stdin sentinel `-`.
pub(crate) fn is_stdin(path: &Path) -> bool {
    path == Path::new("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `FmtArgs` with every flag off — the neutral base each test tweaks.
    fn args() -> FmtArgs {
        FmtArgs {
            paths: Vec::new(),
            check: false,
            write: false,
            diff: false,
            list: false,
            json: false,
            encoding: None,
            fix: false,
        }
    }

    #[test]
    fn encoding_passes_the_override_through() {
        // No `-E`: defer to the caller (None), not a defaulted `Auto`.
        assert_eq!(args().encoding(), None);
        // An explicit override is surfaced verbatim — and `Sjis` is *not*
        // `Encoding::default()` (`Auto`), so this also rejects a
        // `Some(Default::default())` stand-in.
        let mut a = args();
        a.encoding = Some(Encoding::Sjis);
        assert_eq!(a.encoding(), Some(Encoding::Sjis));
    }

    #[test]
    fn serialize_options_opts_into_canonical_only_with_fix() {
        // Without `--fix` directives round-trip verbatim (`Off` = the default).
        assert_eq!(
            args().serialize_options().directives,
            DirectiveNormalization::Off
        );
        // `--fix` opts into the Tier1 canonical rewrite — distinct from the
        // `SerializeOptions::default()` (`Off`) a body-drop would yield.
        let mut a = args();
        a.fix = true;
        assert_eq!(
            a.serialize_options().directives,
            DirectiveNormalization::Canonical
        );
    }
}
