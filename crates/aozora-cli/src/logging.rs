use std::env;
use std::io;

use tracing_subscriber::EnvFilter;

const LOG_ENV: &str = "AOZORA_LOG";

pub(crate) fn init(verbose: u8, quiet: bool) {
    let env_directive = env::var(LOG_ENV).ok();
    let filter = EnvFilter::new(resolve_directive(verbose, quiet, env_directive.as_deref()));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

fn resolve_directive(verbose: u8, quiet: bool, env_directive: Option<&str>) -> &str {
    match env_directive {
        Some(directive) if !directive.trim().is_empty() => directive,
        _ => default_level(verbose, quiet),
    }
}

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

    #[test]
    fn default_level_walks_the_verbosity_axis() {
        assert_eq!(default_level(0, false), "warn");
        assert_eq!(default_level(1, false), "info");
        assert_eq!(default_level(2, false), "debug");
        assert_eq!(default_level(3, false), "trace");
        assert_eq!(default_level(9, false), "trace");
    }

    #[test]
    fn quiet_drops_to_error_only() {
        assert_eq!(default_level(0, true), "error");
    }

    #[test]
    fn verbose_and_quiet_cancel_on_one_axis() {
        assert_eq!(default_level(1, true), "warn");
        assert_eq!(default_level(2, true), "info");
    }

    #[test]
    fn aozora_log_directive_overrides_the_v_q_count() {
        assert_eq!(
            resolve_directive(3, false, Some("aozora_cli=debug")),
            "aozora_cli=debug"
        );
    }

    #[test]
    fn blank_or_unset_aozora_log_falls_back_to_the_count() {
        assert_eq!(resolve_directive(1, false, None), "info");
        assert_eq!(resolve_directive(0, true, Some("   ")), "error");
        assert_eq!(resolve_directive(0, false, Some("")), "warn");
    }
}
