//! `aozora completions <shell>` — print a shell completion script.
//!
//! The script is generated on demand from the live clap [`Cli`] command
//! tree, so it can never drift from the actual flags. Nothing is
//! committed to the repo (unlike the drift-gated wire schema /
//! TypeScript artefacts); release tarballs ship the generated scripts
//! under `completions/`. See `docs/adr/0012-*`.

use std::io;
use std::process::ExitCode;

use clap::{CommandFactory, ValueEnum};
use clap_complete::{Shell, generate};
use clap_complete_nushell::Nushell;

use crate::Cli;

/// Shells `aozora completions` can target: `clap_complete::Shell`'s
/// built-ins plus Nushell, whose generator lives in a sibling crate and
/// is not a `Shell` variant.
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

/// Write the completion script for `args.shell` to stdout. Infallible:
/// `clap_complete::generate` writes directly and surfaces no error, so
/// this returns a bare [`ExitCode`] (the dispatch wraps it in `Ok`).
pub(crate) fn run_completions(args: &CompletionsArgs) -> ExitCode {
    let mut cmd = Cli::command();
    let mut out = io::stdout().lock();
    // Each arm passes a concrete `Generator`; Nushell's lives in its own
    // crate and is not a `Shell` variant, hence the explicit match.
    match args.shell {
        CompletionShell::Bash => generate(Shell::Bash, &mut cmd, "aozora", &mut out),
        CompletionShell::Elvish => generate(Shell::Elvish, &mut cmd, "aozora", &mut out),
        CompletionShell::Fish => generate(Shell::Fish, &mut cmd, "aozora", &mut out),
        CompletionShell::PowerShell => generate(Shell::PowerShell, &mut cmd, "aozora", &mut out),
        CompletionShell::Zsh => generate(Shell::Zsh, &mut cmd, "aozora", &mut out),
        CompletionShell::Nushell => generate(Nushell, &mut cmd, "aozora", &mut out),
    }
    ExitCode::SUCCESS
}
