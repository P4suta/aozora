#![expect(
    clippy::expect_used,
    reason = "the static progress templates are valid"
)]

//! Shows formatter discovery, file progress, and summaries on stderr.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

const SPINNER_TICK_MILLIS: u64 = 120;

use crate::i18n::{FluentArgs, LanguageIdentifier, t, tf};
use indicatif::{ProgressBar, ProgressStyle};

use crate::fmt::Ctx;
use crate::fmt::cli::Mode;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) formatted: usize,
    pub(crate) unchanged: usize,
    pub(crate) errors: usize,
}

impl Tally {
    pub(crate) fn record(&mut self, changed: bool) {
        if changed {
            self.formatted += 1;
        } else {
            self.unchanged += 1;
        }
    }

    pub(crate) fn record_error(&mut self) {
        self.errors += 1;
    }
}

#[derive(Debug)]
pub(crate) struct Printer {
    bar: Option<ProgressBar>,
}

impl Printer {
    pub(crate) fn suspend<T>(&self, f: impl FnOnce() -> T) -> T {
        match &self.bar {
            Some(bar) => bar.suspend(f),
            None => f(),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct Gate {
    pub(crate) stderr_is_terminal: bool,
    pub(crate) quiet: bool,
}

impl Gate {
    pub(crate) fn shows(self) -> bool {
        self.stderr_is_terminal && !self.quiet
    }
}

fn enabled(ctx: &Ctx) -> bool {
    Gate {
        stderr_is_terminal: io::stderr().is_terminal(),
        quiet: ctx.quiet,
    }
    .shows()
}

fn spinner_wanted(enabled: bool, mode: &Mode) -> bool {
    enabled && !mode.is_machine()
}

fn bar_wanted(enabled: bool, total: usize) -> bool {
    enabled && total > 1
}

/// `mutants::skip`: predicate tests cover visibility because the progress widget needs a terminal.
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
    spinner.enable_steady_tick(Duration::from_millis(SPINNER_TICK_MILLIS));
    Some(spinner)
}

/// `mutants::skip`: predicate tests cover visibility because the progress widget needs a terminal.
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

pub(crate) fn printer(bar: Option<&ProgressBar>) -> Printer {
    Printer { bar: bar.cloned() }
}

fn summary_line(lang: &LanguageIdentifier, tally: &Tally) -> String {
    let mut args = FluentArgs::new();
    args.set("formatted", tally.formatted.to_string());
    args.set("unchanged", tally.unchanged.to_string());
    args.set("errors", tally.errors.to_string());
    tf(lang, "fmt-summary", &args)
}

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
