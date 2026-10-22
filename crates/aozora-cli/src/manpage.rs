use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::CommandFactory;

use crate::Cli;
use crate::output;

/// `aozora man [COMMAND]` arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct ManArgs {
    /// Subcommand to render the page for; omit for the top-level page.
    command: Option<String>,
}

pub(crate) fn run_man(args: &ManArgs) -> Result<ExitCode> {
    let root = Cli::command();
    let cmd = match &args.command {
        Some(name) => root
            .find_subcommand(name)
            .cloned()
            .with_context(|| format!("unknown subcommand `{name}`; run `aozora --help`"))?,
        None => root,
    };
    let mut out = output::stdout();
    clap_mangen::Man::new(cmd)
        .render(&mut out)
        .context("render man page")?;
    Ok(ExitCode::SUCCESS)
}
