//! Collects document-command timings for stderr reports.
//! Records timings in the CLI, preserving the parser boundary described by ADR-0001.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use crate::wire::Envelope;
#[cfg(test)]
use aozora::json::SCHEMA_VERSION;

/// Collects optional timings for stderr reports.
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

    /// Records `f`'s duration when timing runs.
    pub(crate) fn measure<T>(&mut self, name: &'static str, f: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return f();
        }
        let start = Instant::now();
        let out = f();
        self.phases.push((name, start.elapsed()));
        out
    }

    /// Writes collected timings to stderr only when enabled and nonempty.
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

fn ms(d: Duration) -> f64 {
    const MILLIS_PER_SECOND: f64 = 1000.0;
    d.as_secs_f64() * MILLIS_PER_SECOND
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_is_secs_times_1000() {
        assert_eq!(ms(Duration::from_millis(1500)), 1500.0);
    }

    #[test]
    fn ms_scales_with_input() {
        assert_eq!(ms(Duration::from_millis(250)), 250.0);
        assert_eq!(ms(Duration::ZERO), 0.0);
    }

    #[test]
    fn nanos_is_exact_nanoseconds() {
        assert_eq!(nanos(Duration::from_nanos(2500)), 2500);
        assert_eq!(nanos(Duration::from_millis(1)), 1_000_000);
    }

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
