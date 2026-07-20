//! Phase timing for the document subcommands (`--timing`).
//!
//! CLI-side instrumentation only: the parser stays a pure, hook-free
//! function (ADR-0001). We wrap the coarse stages a document subcommand
//! runs — `read` (I/O + decode), `parse`, and the command's `output` step
//! (render / inspect / pandoc) — in [`std::time::Instant`] and print the
//! result to **stderr**, so a `render` / `inspect` pipeline's stdout stays
//! byte-identical with or without `--timing`. `fmt` is the exception: it
//! delegates to the shared `crate::fmt::format_source_with` core, which
//! fuses parse and serialize, so it reports `read` + a single `format`
//! stage instead of a separate `parse`.
//!
//! Finer, per-lex-phase numbers (sanitize / tokenize / pair / build)
//! are a parser-development concern served by `xtask samply`, `just
//! bench`, and `just dhat` — not this user-facing flag. They also can't
//! be surfaced uniformly here: `inspect <nodes|pairs>` and `pandoc` render
//! from an `Tree`, whose internals the CLI cannot reconstruct from
//! a hand-driven `Pipeline`, so a common three-stage view is the honest
//! one.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use crate::wire::Envelope;
#[cfg(test)]
use aozora::json::SCHEMA_VERSION;

/// Accumulates phase durations and renders them to stderr.
///
/// When disabled (`--timing` absent) every [`Self::measure`] call runs
/// its closure with no surrounding bookkeeping and records nothing, so
/// the hot path is untouched and [`Self::report`] is a no-op.
pub(crate) struct Timer {
    enabled: bool,
    phases: Vec<(&'static str, Duration)>,
}

impl Timer {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
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
    /// when nothing was measured (`--timing` off, or no phases ran).
    ///
    /// The view auto-selects on the same rule as `check`'s diagnostics: the
    /// aligned `human` report when stderr is a terminal, the machine `json`
    /// envelope when it is piped — so an agent capturing stderr gets a
    /// parseable stream without a flag.
    pub(crate) fn report(&self) -> io::Result<()> {
        if !self.should_report() {
            return Ok(());
        }
        let human = io::stderr().is_terminal();
        let mut stderr = io::stderr().lock();
        if human {
            self.report_human(&mut stderr)
        } else {
            self.report_json(&mut stderr)
        }
    }

    fn should_report(&self) -> bool {
        self.enabled && !self.phases.is_empty()
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

    /// The machine view uses the common wire envelope. `data` carries the
    /// per-phase nanosecond durations and the total.
    fn report_json(&self, w: &mut impl Write) -> io::Result<()> {
        let total: Duration = self.phases.iter().map(|(_, d)| *d).sum();
        let phases: Vec<_> = self
            .phases
            .iter()
            .map(|(name, dur)| serde_json::json!({ "name": name, "nanos": nanos(*dur) }))
            .collect();
        let envelope = Envelope::new(serde_json::json!({
            "phases": phases,
            "totalNanos": nanos(total),
        }));
        serde_json::to_writer(&mut *w, &envelope)?;
        writeln!(w)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "exact f64 pin — the expected values are exactly representable and every mutation (constant swap or * -> + / /) yields a value far from them"
    )]
    fn ms_is_secs_times_1000() {
        // 1500 ms -> 1.5 s -> 1.5 * 1000.0 == 1500.0 exactly.
        // Kills body->0.0/1.0/-1.0 (all differ from 1500.0) and
        // `*`->`+` (1.5 + 1000.0 == 1001.5) / `*`->`/` (1.5 / 1000.0 == 0.0015).
        assert_eq!(ms(Duration::from_millis(1500)), 1500.0);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "exact f64 pin — the expected values are exactly representable and every mutation yields a value far from them"
    )]
    fn ms_scales_with_input() {
        // A second fixed point pins the operator: 250 ms -> 250.0.
        assert_eq!(ms(Duration::from_millis(250)), 250.0);
        assert_eq!(ms(Duration::ZERO), 0.0);
    }

    #[test]
    fn nanos_is_exact_nanoseconds() {
        // Kills body->0 and body->1: 2500 differs from both.
        assert_eq!(nanos(Duration::from_nanos(2500)), 2500);
        assert_eq!(nanos(Duration::from_millis(1)), 1_000_000);
    }

    /// A `Timer` primed with fixed phase durations — bypasses `measure` so
    /// the render seams can be pinned deterministically (wall-clock timing is
    /// otherwise non-reproducible).
    fn primed() -> Timer {
        Timer {
            enabled: true,
            phases: vec![
                ("read", Duration::from_nanos(5)),
                ("parse", Duration::from_nanos(7)),
            ],
        }
    }

    #[test]
    fn report_requires_both_enablement_and_a_measured_phase() {
        let mut timer = Timer::new(false);
        assert!(!timer.should_report());
        timer.phases.push(("read", Duration::ZERO));
        assert!(!timer.should_report());
        timer.enabled = true;
        assert!(timer.should_report());
        timer.phases.clear();
        assert!(!timer.should_report());
    }

    #[test]
    fn report_json_uses_the_two_key_data_envelope() {
        let mut buf = Vec::new();
        primed().report_json(&mut buf).expect("write json");
        let v: serde_json::Value = serde_json::from_slice(&buf).expect("parse json");
        assert_eq!(v["schemaVersion"], SCHEMA_VERSION, "wire version: {v}");
        assert_eq!(v["data"]["totalNanos"], 12, "5 + 7 nanos under data: {v}");
        assert_eq!(v["data"]["phases"][0]["name"], "read");
        assert_eq!(v["data"]["phases"][0]["nanos"], 5);
        assert_eq!(v["data"]["phases"][1]["name"], "parse");
        assert_eq!(v["data"]["phases"][1]["nanos"], 7);
        // The old top-level `phases` / `totalNanos` must be gone — the payload
        // now lives exclusively under `data`.
        assert!(v.get("phases").is_none(), "phases must be nested: {v}");
        assert!(v.get("totalNanos").is_none(), "total must be nested: {v}");
    }

    #[test]
    fn report_human_aligns_phase_lines_and_a_total() {
        let mut buf = Vec::new();
        primed().report_human(&mut buf).expect("write human");
        let text = String::from_utf8(buf).expect("utf8");
        for label in ["read", "parse", "total"] {
            assert!(text.contains(label), "human names {label:?}: {text:?}");
        }
        assert!(
            text.contains("ms"),
            "human carries millisecond units: {text:?}"
        );
    }
}
