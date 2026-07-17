//! Batch UX for directory `fmt`: an [`indicatif`] discovery spinner and
//! per-file progress bar, plus the localized end-of-run summary.
//!
//! Everything here is strictly gated to an *interactive stderr*: the UI draws
//! only when stderr is a terminal and `--quiet` is off, and never over the
//! machine `--json` output (whose path does not reach [`crate::fmt::fold_files`],
//! and whose discovery is excluded here via [`Mode::is_machine`]). The bar and
//! summary go to **stderr**, so stdout — the canonical form, the `--json`
//! envelope, the `--diff` hunks, the `--list` paths — stays byte-identical
//! whether or not a human is watching. See the UI/UX wave plan §9.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crate::i18n::{FluentArgs, LanguageIdentifier, t, tf};
use indicatif::{ProgressBar, ProgressStyle};

use crate::fmt::Ctx;
use crate::fmt::cli::Mode;

/// Per-run tally feeding the batch summary: how many files the formatter
/// changed (or would change), left untouched, and failed on.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// Files the formatter changed (write) or would change (check / list).
    pub(crate) formatted: usize,
    /// Files already in canonical form.
    pub(crate) unchanged: usize,
    /// Files that could not be read or formatted.
    pub(crate) errors: usize,
}

impl Tally {
    /// Fold one successfully processed file into the tally: a `changed` file
    /// counts as formatted, an unchanged one as unchanged.
    pub(crate) fn record(&mut self, changed: bool) {
        if changed {
            self.formatted += 1;
        } else {
            self.unchanged += 1;
        }
    }

    /// Fold one failed file into the tally.
    pub(crate) fn record_error(&mut self) {
        self.errors += 1;
    }
}

/// A print gate that keeps the formatter's own stdout / stderr lines from
/// colliding with a live progress bar. When a bar is active every line the
/// per-file work emits is written *through* [`ProgressBar::suspend`], which
/// clears the bar, runs the print, and redraws — the indicatif-sanctioned way
/// to interleave output with a bar. With no bar (the gated-out common case) it
/// is a zero-cost pass-through, so the non-interactive byte stream is untouched.
#[derive(Debug)]
pub(crate) struct Printer {
    bar: Option<ProgressBar>,
}

impl Printer {
    /// Run `f`, suspending the bar around it when one is active.
    pub(crate) fn suspend<T>(&self, f: impl FnOnce() -> T) -> T {
        match &self.bar {
            Some(bar) => bar.suspend(f),
            None => f(),
        }
    }
}

/// The inputs to the batch-UI gate as a parameter object: the live terminal
/// probe and the user's `--quiet` choice. A struct (not two `bool` params) so
/// the decision is unit-testable without a real terminal — mirroring
/// `timing.rs` / `diagnostics_render.rs` — while keeping the call site honest.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Gate {
    /// Whether the human output stream (stderr) is a terminal.
    pub(crate) stderr_is_terminal: bool,
    /// Whether `--quiet` asked to suppress the batch UI.
    pub(crate) quiet: bool,
}

impl Gate {
    /// The gate decision: the progress UI and summary draw only on an
    /// interactive stderr with `--quiet` off.
    pub(crate) fn shows(self) -> bool {
        self.stderr_is_terminal && !self.quiet
    }
}

/// Whether the batch UI may draw for this run: an interactive stderr with
/// `--quiet` off. Probes the live terminal state through the pure [`Gate`].
fn enabled(ctx: &Ctx) -> bool {
    Gate {
        stderr_is_terminal: io::stderr().is_terminal(),
        quiet: ctx.quiet,
    }
    .shows()
}

/// Whether the discovery spinner should draw: the batch UI is [`enabled`] *and*
/// the mode is not the machine-readable one (`--json`), whose stdout must stay
/// byte-pure. The pure decision behind [`discovery_spinner`]'s real-terminal
/// guard, split out so the "enabled and not machine" rule is unit-tested rather
/// than resting on the untestable live-tty probe — mirroring `tui::all_terminals`.
fn spinner_wanted(enabled: bool, mode: &Mode) -> bool {
    enabled && !mode.is_machine()
}

/// Whether the determinate file bar should draw: the batch UI is [`enabled`]
/// *and* the batch holds more than one file — a one-file bar is noise, not
/// progress. The pure decision behind [`file_bar`]'s real-terminal guard, split
/// out for the same unit-testability reason as [`spinner_wanted`].
fn bar_wanted(enabled: bool, total: usize) -> bool {
    enabled && total > 1
}

/// A spinner shown while directory discovery walks the tree — indeterminate,
/// because the file count is not yet known. `None` (a no-op) when gated out or
/// when the mode is machine-readable (`--json`), whose stdout must stay pure.
///
/// The steady tick starts after a short delay, so trivially fast discovery
/// (a single file, a small tree) completes before the first frame draws and
/// the user sees nothing — the spinner surfaces only for genuinely slow walks.
///
/// Real-terminal only past the [`spinner_wanted`] guard (it builds and steady-
/// ticks a live [`ProgressBar`]), so the sweep cannot exercise it; its one
/// decision is the pure predicate, unit-tested below.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn discovery_spinner(ctx: &Ctx, mode: &Mode) -> Option<ProgressBar> {
    if !spinner_wanted(enabled(ctx), mode) {
        return None;
    }
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg}")
            .expect("the static discovery-spinner template is valid"),
    );
    spinner.set_message(t(&ctx.lang, "fmt-progress-discovering"));
    spinner.enable_steady_tick(Duration::from_millis(120));
    Some(spinner)
}

/// A determinate progress bar for a known file count. `None` (a no-op) when
/// gated out or when the batch is a single file — a one-file bar is noise, not
/// progress. The count is already resolved by the time [`crate::fmt::fold_files`]
/// runs, so a bar (not a spinner) is the right widget here.
///
/// Real-terminal only past the [`bar_wanted`] guard (it builds a live
/// [`ProgressBar`] the caller drives), so the sweep cannot exercise it; its one
/// decision is the pure predicate, unit-tested below.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn file_bar(ctx: &Ctx, total: usize) -> Option<ProgressBar> {
    if !bar_wanted(enabled(ctx), total) {
        return None;
    }
    let bar = ProgressBar::new(total as u64);
    bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} {wide_msg}",
        )
        .expect("the static file-bar template is valid")
        .progress_chars("=>-"),
    );
    Some(bar)
}

/// Wrap `bar` in a [`Printer`] so per-file lines interleave cleanly, cloning
/// the cheap `Arc`-backed handle. A `None` bar yields a pass-through printer.
pub(crate) fn printer(bar: Option<&ProgressBar>) -> Printer {
    Printer { bar: bar.cloned() }
}

/// Build the localized one-line batch summary — `"N formatted, M unchanged, K
/// errors"` in the run's language — from the [`Tally`]. Pure over the tally and
/// language, so the count-to-message mapping is unit-tested apart from the
/// terminal-gated [`summary`] that writes it.
fn summary_line(lang: &LanguageIdentifier, tally: &Tally) -> String {
    let mut args = FluentArgs::new();
    args.set("formatted", tally.formatted.to_string());
    args.set("unchanged", tally.unchanged.to_string());
    args.set("errors", tally.errors.to_string());
    tf(lang, "fmt-summary", &args)
}

/// Print the localized one-line batch summary to stderr, gated exactly like the
/// bar. Silent when gated out, so a piped run emits nothing extra.
///
/// The terminal-gated write shell around the pure [`summary_line`]: skipped by
/// the sweep because the headless host gates it out (nothing to observe), while
/// [`summary_line`] carries the count-to-message assertions below.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn summary(ctx: &Ctx, tally: &Tally) {
    if !enabled(ctx) {
        return;
    }
    let line = summary_line(&ctx.lang, tally);
    let _drop = writeln!(io::stderr(), "{line}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::resolve;

    #[test]
    fn tally_record_splits_formatted_from_unchanged() {
        // The changed/unchanged split the batch summary reports: `record(true)`
        // is a formatted file, `record(false)` an unchanged one, and neither
        // touches the error count. Pinning each bucket kills the mutants that
        // swap the branches or increment the wrong field.
        let mut tally = Tally::default();
        tally.record(true);
        tally.record(true);
        tally.record(false);
        tally.record_error();
        assert_eq!(
            tally,
            Tally {
                formatted: 2,
                unchanged: 1,
                errors: 1,
            },
        );
    }

    #[test]
    fn gate_shows_only_on_an_interactive_stderr_without_quiet() {
        // The full truth table of the gate: only an interactive stderr with
        // `--quiet` off draws. Pinning all four cases kills the mutants that
        // drop either conjunct or flip the `!quiet`.
        let gate = |stderr_is_terminal, quiet| {
            Gate {
                stderr_is_terminal,
                quiet,
            }
            .shows()
        };
        assert!(gate(true, false), "interactive, not quiet → shown");
        assert!(!gate(true, true), "--quiet suppresses even on a tty");
        assert!(!gate(false, false), "piped stderr → never shown");
        assert!(!gate(false, true), "piped and quiet → never shown");
    }

    #[test]
    fn spinner_wanted_needs_enabled_and_a_human_mode() {
        // The discovery spinner draws only when the batch UI is enabled AND the
        // mode is not machine-readable (`--json`), whose stdout must stay pure.
        // The full truth table — over the real `Mode` its live guard passes —
        // kills the mutants that flip either conjunct or drop the `!is_machine`.
        use crate::fmt::cli::CheckReport;
        let human = Mode::Stdout;
        let machine = Mode::Check(CheckReport::Json);
        assert!(spinner_wanted(true, &human), "enabled, human mode → spins");
        assert!(
            !spinner_wanted(true, &machine),
            "--json suppresses the spinner"
        );
        assert!(!spinner_wanted(false, &human), "gated-out UI → no spinner");
        assert!(
            !spinner_wanted(false, &machine),
            "gated out and machine → none"
        );
    }

    #[test]
    fn bar_wanted_needs_enabled_and_more_than_one_file() {
        // The determinate bar draws only when the batch UI is enabled AND more
        // than one file is in play — a one-file (or empty) batch is noise, not
        // progress. Cases straddling the `> 1` threshold with both `enabled`
        // values kill the mutants that flip the conjunct or move the boundary.
        assert!(bar_wanted(true, 2), "enabled, many files → bar");
        assert!(!bar_wanted(true, 1), "a single file is noise, not progress");
        assert!(!bar_wanted(true, 0), "an empty batch draws no bar");
        assert!(
            !bar_wanted(false, 5),
            "gated-out UI → no bar even for many files"
        );
    }

    #[test]
    fn summary_line_maps_each_count_to_its_slot() {
        // The English summary places the three counts in a fixed order; distinct
        // values pin each to its slot, killing a body-drop (empty / stub string)
        // and any swap of the formatted / unchanged / errors arguments.
        let lang = resolve(None, None, None, None);
        let tally = Tally {
            formatted: 2,
            unchanged: 1,
            errors: 3,
        };
        assert_eq!(
            summary_line(&lang, &tally),
            "2 formatted, 1 unchanged, 3 errors",
        );
    }
}
