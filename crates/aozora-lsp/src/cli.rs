//! Command-line surface for the `aozora-lsp` daemon: just enough argv
//! handling for `--version`, `--help`, and the conventional `--stdio` flag
//! editors pass. The server itself only ever speaks LSP over stdio.

use clap::Parser;

const LONG_ABOUT: &str = concat!(
    "Language Server for aozora-flavored-markdown. Speaks LSP over stdio: ",
    "stdout carries the JSON-RPC wire protocol and logs go to stderr.\n\n",
    "Environment variables:\n",
    "  RUST_LOG                  tracing filter, e.g. `aozora_lsp=debug` ",
    "(default: warn).\n",
    "  AOZORA_LSP_SLOW_PARSE_US  per-parse latency in microseconds above which ",
    "a slow-parse warning is logged (default: 100000).",
);

/// `aozora-lsp` — Language Server for aozora-flavored-markdown.
#[derive(Parser, Debug)]
#[command(
    name = "aozora-lsp",
    about = "Language Server for aozora-flavored-markdown (speaks LSP over stdio)",
    long_about = LONG_ABOUT,
    version = aozora_buildstamp::VERSION
)]
pub(crate) struct Cli {
    /// Speak LSP over stdio. Accepted for editor compatibility; this is the
    /// only supported transport, so the flag is a no-op.
    #[arg(long)]
    #[allow(
        dead_code,
        reason = "--stdio is accepted for editor compatibility but never read: the daemon only speaks stdio"
    )]
    stdio: bool,
}

impl Cli {
    /// Parse argv. clap prints and exits for `--help`/`--version` (and on a
    /// usage error) *before* this returns, so the JSON-RPC stdout channel is
    /// never touched when those flags are present.
    #[must_use]
    pub(crate) fn parse_args() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use clap::CommandFactory;

    #[test]
    fn no_args_parse_clean() {
        // `try_parse_from` drives the same derived parser as `parse_args`
        // without touching the test runner's real argv.
        Cli::try_parse_from(["aozora-lsp"]).expect("no-arg invocation parses");
    }

    #[test]
    fn stdio_flag_is_accepted() {
        Cli::try_parse_from(["aozora-lsp", "--stdio"]).expect("--stdio is accepted");
    }

    #[test]
    fn unknown_flag_is_rejected() {
        assert!(
            Cli::try_parse_from(["aozora-lsp", "--definitely-not-a-flag"]).is_err(),
            "an unknown flag must be a usage error",
        );
    }

    #[test]
    fn version_carries_the_channel_stamp() {
        // `--version` reports the channel-aware build identity from
        // `aozora-buildstamp` — `0.4.1-dev+g<sha>` in a checkout, a clean
        // `0.4.1` for a stable / crates.io build — never a bare unstamped
        // triple, so a local build is never mistaken for a release. The string
        // always begins with the workspace crate version.
        let version = Cli::command()
            .get_version()
            .expect("a version string is set")
            .to_owned();
        let base = env!("CARGO_PKG_VERSION");
        assert!(
            version == base || version.starts_with(&format!("{base}-")),
            "version `{version}` should be `{base}` optionally with a channel suffix",
        );
    }

    #[test]
    fn clap_command_is_internally_consistent() {
        // `debug_assert` walks the whole derived command tree and panics on
        // any malformed arg/about/version wiring.
        Cli::command().debug_assert();
    }
}
