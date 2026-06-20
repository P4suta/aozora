//! Phase timing for the document subcommands (`--timing`).
//!
//! CLI-side instrumentation only: the parser stays a pure, hook-free
//! function (ADR-0001). We wrap the three coarse stages every document
//! subcommand shares — `read` (I/O + decode), `parse`, and the
//! command's `output` step (render / serialize / wire / pandoc) — in
//! [`std::time::Instant`] and print the result to **stderr**, so a
//! `render` / `wire` pipeline's stdout stays byte-identical with or
//! without `--timing`.
//!
//! Finer, per-lex-phase numbers (sanitize / tokenize / pair / build)
//! are a parser-development concern served by `xtask samply`, `just
//! bench`, and `just dhat` — not this user-facing flag. They also can't
//! be surfaced uniformly here: `wire <nodes|pairs>` and `pandoc` render
//! from an `AozoraTree`, whose internals the CLI cannot reconstruct from
//! a hand-driven `Pipeline`, so a common three-stage view is the honest
//! one.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use clap::ValueEnum;

/// How `--timing` renders its report.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum TimingFormat {
    /// Aligned `name  duration` lines plus a total. The default.
    #[default]
    Human,
    /// `{"schema_version":1,"phases":[{"name","nanos"}],"total_nanos"}`
    /// — the agent / scripting view.
    Json,
}

/// Accumulates phase durations and renders them to stderr.
///
/// When disabled (`--timing` absent) every [`Self::measure`] call runs
/// its closure with no surrounding bookkeeping and records nothing, so
/// the hot path is untouched and [`Self::report`] is a no-op.
pub(crate) struct Timer {
    enabled: bool,
    format: TimingFormat,
    phases: Vec<(&'static str, Duration)>,
}

impl Timer {
    pub(crate) fn new(enabled: bool, format: TimingFormat) -> Self {
        Self {
            enabled,
            format,
            phases: Vec::new(),
        }
    }

    /// Run `f`, recording its wall-clock duration under `name` when
    /// timing is enabled. Returns `f`'s value untouched either way.
    pub(crate) fn measure<T>(&mut self, name: &'static str, f: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return f();
        }
        let start = Instant::now();
        let out = f();
        self.phases.push((name, start.elapsed()));
        out
    }

    /// Write the collected timings to stderr. A no-op when disabled or
    /// when nothing was measured (e.g. `wire slugs`, which neither reads
    /// nor parses).
    pub(crate) fn report(&self) -> io::Result<()> {
        if !self.enabled || self.phases.is_empty() {
            return Ok(());
        }
        let mut stderr = io::stderr().lock();
        match self.format {
            TimingFormat::Human => self.report_human(&mut stderr),
            TimingFormat::Json => self.report_json(&mut stderr),
        }
    }

    fn report_human(&self, w: &mut impl Write) -> io::Result<()> {
        let width = self
            .phases
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0)
            .max("total".len());
        let total: Duration = self.phases.iter().map(|(_, d)| *d).sum();
        for (name, dur) in &self.phases {
            writeln!(w, "{name:<width$}  {:>9.3} ms", ms(*dur))?;
        }
        writeln!(w, "{:<width$}  {:>9.3} ms", "total", ms(total))
    }

    fn report_json(&self, w: &mut impl Write) -> io::Result<()> {
        let total: Duration = self.phases.iter().map(|(_, d)| *d).sum();
        let phases: Vec<_> = self
            .phases
            .iter()
            .map(|(name, dur)| serde_json::json!({ "name": name, "nanos": nanos(*dur) }))
            .collect();
        let envelope = serde_json::json!({
            "schema_version": 1,
            "phases": phases,
            "total_nanos": nanos(total),
        });
        writeln!(w, "{envelope}")
    }
}

/// Whole milliseconds as an `f64`, for the aligned human report.
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Nanoseconds clamped to `u64` — `serde_json::Number` has no `u128`
/// arm without `arbitrary_precision`, and `u64` nanoseconds span ~584
/// years, far past any single parse.
fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}
