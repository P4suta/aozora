use std::io::Write;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{CommandFactory, ValueEnum};
use clap_complete::{Shell, generate};
use clap_complete_nushell::Nushell;

use crate::Cli;
use crate::output;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    Zsh,
    Nushell,
}

/// `aozora completions <shell>` arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct CompletionsArgs {
    /// Shell dialect to emit a completion script for.
    #[arg(value_enum)]
    shell: CompletionShell,
}

pub(crate) fn run_completions(args: &CompletionsArgs) -> Result<ExitCode> {
    let mut cmd = Cli::command();
    let mut generated = Vec::new();
    match args.shell {
        CompletionShell::Bash => generate(Shell::Bash, &mut cmd, "aozora", &mut generated),
        CompletionShell::Elvish => generate(Shell::Elvish, &mut cmd, "aozora", &mut generated),
        CompletionShell::Fish => generate(Shell::Fish, &mut cmd, "aozora", &mut generated),
        CompletionShell::PowerShell => {
            generate(Shell::PowerShell, &mut cmd, "aozora", &mut generated);
        }
        CompletionShell::Zsh => generate(Shell::Zsh, &mut cmd, "aozora", &mut generated),
        CompletionShell::Nushell => generate(Nushell, &mut cmd, "aozora", &mut generated),
    }
    output::stdout()
        .write_all(&generated)
        .context("write completion script to stdout")?;
    Ok(ExitCode::SUCCESS)
}
