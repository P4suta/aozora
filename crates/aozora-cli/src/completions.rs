//! `aozora completions <shell>` — print a shell completion script.
//!
//! The script is generated on demand from the live clap [`Cli`] command
//! tree, so it can never drift from the actual flags. Nothing is
//! committed to the repo (unlike the drift-gated wire schema /
//! TypeScript artefacts); release tarballs ship the generated scripts
//! under `completions/`. See `docs/adr/0012-*`.

use std::io;
use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::Cli;

/// `aozora completions <shell>` arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct CompletionsArgs {
    /// Shell dialect to emit a completion script for.
    #[arg(value_enum)]
    shell: Shell,
}

/// Write the completion script for `args.shell` to stdout. Infallible:
/// `clap_complete::generate` writes directly and surfaces no error, so
/// this returns a bare [`ExitCode`] (the dispatch wraps it in `Ok`).
pub(crate) fn run_completions(args: &CompletionsArgs) -> ExitCode {
    let mut cmd = Cli::command();
    let mut out = io::stdout().lock();
    generate(args.shell, &mut cmd, "aozora", &mut out);
    ExitCode::SUCCESS
}
