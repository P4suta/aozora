//! `aozora man [COMMAND]` — emit a roff man page (hidden subcommand).
//!
//! Generated from the live clap command tree (`clap_mangen`), so it
//! never drifts from the actual flags. Like completions, man pages are
//! produced at release time and shipped in the tarball under
//! `man/man1/`, never committed — see `docs/adr/0012-*`.
//!
//! Hidden from `--help`: a man page is a packaged artefact, not a
//! command users invoke by hand. It stays reachable for the release job
//! and for power users who want to install a page locally.

use std::io;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::CommandFactory;

use crate::Cli;

/// `aozora man [COMMAND]` arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct ManArgs {
    /// Subcommand to render the page for; omit for the top-level page.
    command: Option<String>,
}

/// Render a roff man page to stdout: the top-level page, or `COMMAND`'s
/// page when a subcommand is named.
pub(crate) fn run_man(args: &ManArgs) -> Result<ExitCode> {
    let root = Cli::command();
    let cmd = match &args.command {
        Some(name) => root
            .find_subcommand(name)
            .cloned()
            .with_context(|| format!("unknown subcommand `{name}`; run `aozora --help`"))?,
        None => root,
    };
    let mut out = io::stdout().lock();
    clap_mangen::Man::new(cmd)
        .render(&mut out)
        .context("render man page")?;
    Ok(ExitCode::SUCCESS)
}
