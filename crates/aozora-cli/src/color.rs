//! Process-wide colour policy for the `aozora` CLI's diagnostics.
//!
//! `aozora check`'s human diagnostics render through miette's graphical
//! [`Report`](miette::Report) handler, and miette resolves colour once — when
//! a `Report` is *constructed* — by consulting a process-global hook. That
//! hook is therefore the single lever governing diagnostic colour for the
//! whole binary. [`install`] sets it from the global `--color` flag before
//! any subcommand runs, mapping the shared [`ColorChoice`] onto miette's own
//! detection.
//!
//! Scope and known limitations (intentional, not bugs):
//! - `comfy-table` (`aozora spec kinds`) is built with `default-features = false`,
//!   so its colour / TTY logic is compiled out — those tables are always
//!   monochrome and pipe-safe, and we deliberately do not enable its `tty`
//!   feature.
//! - clap's own `--help` / usage colouring is decided at parse time, before
//!   this value is read; but clap 4 honours `NO_COLOR` / `CLICOLOR`
//!   natively, so env-based consistency still holds.

use aozora_fmt::ColorChoice;
use miette::MietteHandlerOpts;

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
