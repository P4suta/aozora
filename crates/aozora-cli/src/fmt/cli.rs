//! Command-line surface for `aozora fmt`: the clap [`FmtArgs`] flattened into
//! the CLI's `fmt` subcommand, plus the [`Mode`] it derives once so the rest of
//! the pipeline never juggles raw booleans.

use std::path::{Path, PathBuf};

use aozora::{DirectiveNormalization, SerializeOptions};
use clap::{Args, ValueEnum};

/// The formatter's argument surface, flattened into the `aozora fmt` subcommand.
#[derive(Args, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a clap flag struct: each bool is an independent CLI switch, collapsed into Mode by mode()"
)]
pub(crate) struct FmtArgs {
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
/// subcommands share one colour policy with the formatter. `Deserialize`
/// (lowercase, mirroring [`Encoding`]) lets it back the `color` key in
/// `.aozora.toml`.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ColorChoice {
    /// Colour when stdout is a terminal (honours `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE`).
    #[default]
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

impl Mode {
    /// True for the machine-readable output mode (`--check --json`) — the one
    /// mode whose stdout is a byte-stable contract, so the human batch progress
    /// UI must never draw for it (the discovery spinner is skipped; the bar and
    /// summary already never reach it, as `--json` bypasses `fold_files`).
    pub(crate) fn is_machine(&self) -> bool {
        matches!(self, Self::Check(CheckReport::Json))
    }
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
    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The serialization options derived from the flags — currently just the
    /// `--fix` autofix opt-in, which applies zero-false-positive Tier1 only
    /// (`Canonical`); the lossy Tier2 reductions are render-only (ADR-0026).
    pub(crate) fn serialize_options(&self) -> SerializeOptions {
        SerializeOptions::default().directives(if self.fix {
            DirectiveNormalization::Canonical
        } else {
            DirectiveNormalization::Off
        })
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
            fix: false,
        }
    }

    #[test]
    fn serialize_options_opts_into_canonical_only_with_fix() {
        // Without `--fix` directives round-trip verbatim (`Off` = the default).
        // `directives` is private on the collapsed `SerializeOptions`, so compare
        // against the builder-constructed values through the derived `PartialEq`.
        assert_eq!(
            args().serialize_options(),
            SerializeOptions::default().directives(DirectiveNormalization::Off)
        );
        // `--fix` opts into the Tier1 canonical rewrite — distinct from the
        // `SerializeOptions::default()` (`Off`) a body-drop would yield.
        let mut a = args();
        a.fix = true;
        assert_eq!(
            a.serialize_options(),
            SerializeOptions::default().directives(DirectiveNormalization::Canonical)
        );
    }

    #[test]
    fn is_machine_is_exactly_check_json() {
        // The machine-readable mode is `--check --json` alone: its stdout is a
        // byte-stable contract, so it is the one mode that reports `true` and
        // suppresses the human batch UI. Pinning `Json` true and every other
        // mode false kills the `-> true` / `-> false` body replacements.
        assert!(Mode::Check(CheckReport::Json).is_machine());
        assert!(!Mode::Check(CheckReport::Plain).is_machine());
        assert!(!Mode::Check(CheckReport::Diff).is_machine());
        assert!(!Mode::Stdout.is_machine());
        assert!(!Mode::Write { list: false }.is_machine());
        assert!(!Mode::Write { list: true }.is_machine());
        assert!(!Mode::List.is_machine());
    }
}
