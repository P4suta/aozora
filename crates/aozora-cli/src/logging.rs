//! stderr-only `tracing` setup for the `aozora` CLI shell.
//!
//! The CLI installs one `tracing_subscriber::fmt` subscriber writing to
//! **stderr** (never stdout — `render` / `inspect` / `pandoc` pipelines and
//! the machine diagnostic streams must stay byte-identical regardless of
//! verbosity). The default level is `warn`; the global `-v`/`-q` flags move it
//! along one axis (`-v` info, `-vv` debug, `-vvv` trace; `-q` errors only), and
//! an explicit `AOZORA_LOG` filter directive overrides the flag-derived level
//! outright.
//!
//! `AOZORA_LOG` accepts the full [`tracing_subscriber::EnvFilter`] grammar
//! (e.g. `AOZORA_LOG=aozora_cli=debug`). `RUST_LOG` is deliberately **not**
//! consulted — the CLI's logging is namespaced to its own variable, so a
//! `RUST_LOG` set for some other tool never changes `aozora`'s output.
//!
//! Mirrors the LSP idiom (`EnvFilter` + `fmt` to stderr); the only
//! differences are the `AOZORA_LOG` env var name and the `-v`/`-q` default,
//! since the daemon has no verbosity flags to fold in.

use std::env;
use std::io;

use tracing_subscriber::EnvFilter;

/// The CLI's own log-filter environment variable — namespaced on purpose so
/// `RUST_LOG` is never consulted.
const LOG_ENV: &str = "AOZORA_LOG";

/// Install the process-wide stderr tracing subscriber. Call once, early in
/// `main`, before any subcommand runs. `verbose` is the `-v` repeat count and
/// `quiet` the `-q` flag; a non-blank `AOZORA_LOG` directive overrides both.
pub(crate) fn init(verbose: u8, quiet: bool) {
    let env_directive = env::var(LOG_ENV).ok();
    let filter = EnvFilter::new(resolve_directive(verbose, quiet, env_directive.as_deref()));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        // stderr only — stdout stays byte-identical for the render / inspect /
        // pandoc pipelines and the machine diagnostic streams.
        .with_writer(io::stderr)
        .init();
}

/// Choose the effective filter directive: an explicit, non-blank `AOZORA_LOG`
/// wins outright (the full `EnvFilter` grammar); otherwise the `-v`/`-q` count
/// maps to a bare level. Split out — and taking the env value as a parameter
/// rather than reading the process environment — so the precedence is
/// unit-testable.
fn resolve_directive(verbose: u8, quiet: bool, env_directive: Option<&str>) -> &str {
    match env_directive {
        Some(directive) if !directive.trim().is_empty() => directive,
        _ => default_level(verbose, quiet),
    }
}

/// Map the `-v`/`-q` axis to a bare log level. `-v` and `-q` are opposite
/// directions of one axis (ripgrep-style): the net step is `verbose - quiet`,
/// so `-qv` cancels back to the `warn` default. Default `warn`; `-v` info,
/// `-vv` debug, `-vvv`+ trace; a net-negative step (`-q`) errors only.
fn default_level(verbose: u8, quiet: bool) -> &'static str {
    match i32::from(verbose) - i32::from(quiet) {
        i32::MIN..=-1 => "error",
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- default_level: the -v/-q axis -> bare level mapping ---

    #[test]
    fn default_level_walks_the_verbosity_axis() {
        assert_eq!(default_level(0, false), "warn");
        assert_eq!(default_level(1, false), "info");
        assert_eq!(default_level(2, false), "debug");
        assert_eq!(default_level(3, false), "trace");
        // Saturates at trace: -vvvv and beyond stay trace, not something louder.
        assert_eq!(default_level(9, false), "trace");
    }

    #[test]
    fn quiet_drops_to_error_only() {
        assert_eq!(default_level(0, true), "error");
    }

    #[test]
    fn verbose_and_quiet_cancel_on_one_axis() {
        // ripgrep-style: -qv nets back to the warn default, -qvv up to info.
        assert_eq!(default_level(1, true), "warn");
        assert_eq!(default_level(2, true), "info");
    }

    // --- resolve_directive: AOZORA_LOG-over-count precedence ---

    #[test]
    fn aozora_log_directive_overrides_the_v_q_count() {
        // A set, non-blank directive wins outright, ignoring even -vvv.
        assert_eq!(
            resolve_directive(3, false, Some("aozora_cli=debug")),
            "aozora_cli=debug"
        );
    }

    #[test]
    fn blank_or_unset_aozora_log_falls_back_to_the_count() {
        // Unset -> the flag-derived level.
        assert_eq!(resolve_directive(1, false, None), "info");
        // Set-but-blank (whitespace only, or empty) is treated as unset.
        assert_eq!(resolve_directive(0, true, Some("   ")), "error");
        assert_eq!(resolve_directive(0, false, Some("")), "warn");
    }
}
