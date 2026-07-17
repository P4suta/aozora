//! Process-wide colour policy for the `aozora` CLI's diagnostics.
//!
//! `aozora check`'s human diagnostics render through miette's graphical
//! [`Report`](miette::Report) handler, and miette resolves colour once — when
//! a `Report` is *constructed* — by consulting a process-global hook. That
//! hook is therefore the single lever governing diagnostic colour for the
//! whole binary. [`install`] sets it before any subcommand runs, mapping the
//! shared [`ColorChoice`] onto miette's own detection; [`resolve`] decides
//! *which* [`ColorChoice`] that is.
//!
//! **Two levels, not one chain.** [`resolve`] runs the config layering
//! (ADR-0013) to pick a [`ColorChoice`]:
//! `--color > project .aozora.toml > global (XDG) > default`. Only then does
//! [`ColorChoice::Auto`] consult the terminal —
//! `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE` and the stderr TTY. Those three
//! are inputs *to* `auto`, not a layer of the chain, so a decided `always` /
//! `never` outranks them no matter which layer decided it: `.aozora.toml`
//! `color = "never"` beats `CLICOLOR_FORCE` exactly as `--color never` does.
//! This is cargo's two-level shape (`--color` > `term.color`, with `NO_COLOR`
//! honoured only on `auto`).
//!
//! **Colour has no `AOZORA_*` variable of its own**, and that is the design:
//! the ecosystem already standardises the colour environment, so the `auto`
//! path defers to `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE` rather than this
//! CLI inventing a fourth spelling beside them. The env rung the other
//! settings carry (`AOZORA_ENCODING` / `AOZORA_FORMAT` / `AOZORA_STRICT`,
//! each backing a *subcommand-local* flag) is simply absent here.
//!
//! Scope and known limitations (intentional, not bugs):
//! - `comfy-table` (`aozora spec kinds`) is built with `default-features = false`,
//!   so its colour / TTY logic is compiled out — those tables are always
//!   monochrome and pipe-safe, and we deliberately do not enable its `tty`
//!   feature.
//! - clap's own `--help` / usage colouring is decided at parse time, before
//!   this value is read; but clap 4 honours `NO_COLOR` / `CLICOLOR`
//!   natively, so env-based consistency still holds.

use crate::fmt::ColorChoice;
use miette::MietteHandlerOpts;

/// The effective colour choice: `--color`, else the `.aozora.toml` `color` key
/// (project over global, settled by
/// [`ConfigFile::merge`](crate::config::ConfigFile)), else
/// [`ColorChoice::Auto`].
///
/// The same `Option::or` fold every other config-backed setting resolves
/// through — [`CommonArgs::resolved_encoding`](crate::CommonArgs) is its twin —
/// so colour carries no precedence rule of its own (ADR-0013), only one rung
/// fewer: no environment layer sits between the flag and the file, for the
/// reason this module documents. Both layers arrive already parsed, which keeps
/// this a pure seam the precedence tests pin without touching the environment.
pub(crate) fn resolve(flag: Option<ColorChoice>, config: Option<ColorChoice>) -> ColorChoice {
    flag.or(config).unwrap_or_default()
}

/// Install a process-wide miette report hook honouring `choice`.
///
/// - [`ColorChoice::Always`] forces colour even when stderr is piped.
/// - [`ColorChoice::Never`] disables colour unconditionally.
/// - [`ColorChoice::Auto`] adds no override, deferring to miette's own
///   detection — which honours `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE`
///   and whether stderr is a TTY.
///
/// Call once, early in `main`, before any [`Report`](miette::Report) is
/// constructed: miette captures the handler at construction time and its
/// hook slot is write-once. The `Result` is ignored for exactly that reason
/// — a redundant install is a harmless no-op, not an error worth surfacing.
pub(crate) fn install(choice: ColorChoice) {
    let _ignored = miette::set_hook(Box::new(move |_| {
        // Rebuilt on every call: `build()` consumes the options and the hook
        // is `Fn`. `choice` is `Copy`, so the `move` closure re-reads it.
        let opts = match choice {
            ColorChoice::Always => MietteHandlerOpts::new().color(true),
            ColorChoice::Never => MietteHandlerOpts::new().color(false),
            // No `.color(_)`: let miette detect colour support (NO_COLOR /
            // CLICOLOR / CLICOLOR_FORCE + stderr-TTY) exactly as it would
            // with no hook installed at all.
            ColorChoice::Auto => MietteHandlerOpts::new(),
        };
        Box::new(opts.build())
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_beats_the_config_key() {
        assert_eq!(
            resolve(Some(ColorChoice::Always), Some(ColorChoice::Never)),
            ColorChoice::Always
        );
        assert_eq!(
            resolve(Some(ColorChoice::Never), Some(ColorChoice::Always)),
            ColorChoice::Never
        );
    }

    #[test]
    fn the_config_decides_when_the_flag_is_absent() {
        // The point of the key being wired at all: an absent flag falls through
        // to `.aozora.toml` rather than short-circuiting to `auto`.
        assert_eq!(resolve(None, Some(ColorChoice::Never)), ColorChoice::Never);
        assert_eq!(
            resolve(None, Some(ColorChoice::Always)),
            ColorChoice::Always
        );
    }

    #[test]
    fn nothing_set_is_auto() {
        assert_eq!(resolve(None, None), ColorChoice::Auto);
    }

    #[test]
    fn an_explicit_flag_auto_still_beats_the_config() {
        // Why the flag is an `Option` rather than a `default_value = "auto"`:
        // an explicit `--color auto` is a real choice and must outrank a
        // config `never`, which a defaulted `Auto` could never express.
        assert_eq!(
            resolve(Some(ColorChoice::Auto), Some(ColorChoice::Never)),
            ColorChoice::Auto
        );
    }
}
