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
    /// Files or directories to format.
    /// Use `-`, or omit, to read stdin.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Verify inputs are already formatted; exit 1 if any would change.
    #[arg(long, conflicts_with_all = ["write", "list"])]
    check: bool,

    /// Rewrite files in place (no-op when already canonical).
    #[arg(long, short = 'w', conflicts_with_all = ["check", "diff", "json"])]
    write: bool,

    /// Print a unified diff for every file that would change.
    /// Implies --check.
    #[arg(long, conflicts_with_all = ["write", "list", "json"])]
    diff: bool,

    /// List only the paths that would change (gofmt -l).
    /// Combine with -w.
    #[arg(long, short = 'l', conflicts_with_all = ["check", "diff", "json"])]
    list: bool,

    /// Emit the --check result as machine-readable JSON.
    /// Implies --check.
    #[arg(long, conflicts_with_all = ["write", "list", "diff"])]
    json: bool,

    /// Rewrite non-canonical directive near-misses to their canonical spelling (e.g. `［＃字下げ終わり］` → `［＃ここで字下げ終わり］`), the fixes flagged by the `aozora::lint::non_canonical_directive` warnings.
    /// Opt-in: without it every directive round-trips its raw bytes verbatim.
    /// Idempotent, so it composes with every mode.
    #[arg(long)]
    fix: bool,
}

/// When to emit ANSI colour in terminal output (diffs, diagnostics, …).
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

pub(crate) enum Mode {
    Stdout,
    Write { list: bool },
    Check(CheckReport),
    List,
}

impl Mode {
    pub(crate) fn is_machine(&self) -> bool {
        matches!(self, Self::Check(CheckReport::Json))
    }
}

pub(crate) enum CheckReport {
    Plain,
    Diff,
    Json,
}

impl FmtArgs {
    #[must_use]
    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub(crate) fn serialize_options(&self) -> SerializeOptions {
        SerializeOptions::default().directives(if self.fix {
            DirectiveNormalization::Canonical
        } else {
            DirectiveNormalization::Off
        })
    }

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

pub(crate) fn is_stdin(path: &Path) -> bool {
    path == Path::new("-")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            args().serialize_options(),
            SerializeOptions::default().directives(DirectiveNormalization::Off)
        );
        let mut a = args();
        a.fix = true;
        assert_eq!(
            a.serialize_options(),
            SerializeOptions::default().directives(DirectiveNormalization::Canonical)
        );
    }

    #[test]
    fn is_machine_is_exactly_check_json() {
        assert!(Mode::Check(CheckReport::Json).is_machine());
        assert!(!Mode::Check(CheckReport::Plain).is_machine());
        assert!(!Mode::Check(CheckReport::Diff).is_machine());
        assert!(!Mode::Stdout.is_machine());
        assert!(!Mode::Write { list: false }.is_machine());
        assert!(!Mode::Write { list: true }.is_machine());
        assert!(!Mode::List.is_machine());
    }
}
