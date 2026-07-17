//! Per-document observability metrics.
//!
//! # Why this exists
//!
//! When a problem shows up in production, the developer who wakes up
//! at 3 AM has only the logs. A snapshot of "what was this document
//! doing" — edit count, cache hit rate, parse-latency tails — turns
//! "the LSP was slow" into "this specific document had a 99th-
//! percentile parse latency of 320 ms with a 12% cache hit rate
//! after the user pasted a 2 MB block".
//!
//! # Design
//!
//! - **Atomic counters** (`AtomicU64`) for monotonically-increasing
//!   tallies (edit count, cache hits/misses, etc.). Cheap to bump
//!   from any thread, no lock contention on the hot path.
//! - **`hdrhistogram::Histogram<u64>`** for parse latency. Mean alone
//!   hides tail behaviour; `HDRHistogram` gives constant-memory
//!   percentile tracking with a bounded relative error (we use 3
//!   significant digits, max value 1e9 µs = 1000 s).
//! - **`Metrics::snapshot()`** returns a [`MetricsSnapshot`] with the
//!   current counter values + the histogram percentiles. `MetricsSnapshot`
//!   is `Serialize` so future telemetry exports (Prometheus, OTLP,
//!   etc.) can be added behind a feature flag without changing the
//!   recording sites.
//! - **`did_close` lifecycle dump** — when a document is closed the
//!   backend logs `Metrics::snapshot()` at INFO level under the
//!   `aozora_cli::lsp::metrics` target so a third party reading the log
//!   can reconstruct the document's history.
//!
//! Histogram lock contention is bounded: every parse takes the lock
//! once for an O(1) record. For an LSP keystroke rate (≤ 100 / s)
//! this is well below the lock's overhead floor.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;
use serde::Serialize;

/// `HDRHistogram` configuration: 3 significant digits, max 1 e9 µs
/// (= 1000 s). A parse that takes longer than 1000 seconds is
/// itself a bug — the upper bound exists to keep memory bounded.
const HIST_MIN: u64 = 1;
const HIST_MAX: u64 = 1_000_000_000;
const HIST_SIGFIG: u8 = 3;

/// Per-document metrics.
///
/// All counter mutations are `Relaxed`-ordered: counts are
/// observation-only and we don't synchronise other state through
/// them. The histogram has its own lock; we never hold two of these
/// fields' locks simultaneously.
#[derive(Debug)]
pub(super) struct Metrics {
    /// Number of `did_change` events observed.
    pub edit_count: AtomicU64,
    /// Number of times the document has been re-parsed (every
    /// edit + initial open + full-replacement triggers one).
    pub parse_count: AtomicU64,
    /// Cumulative cache hits across all reparses.
    pub cache_hit_total: AtomicU64,
    /// Cumulative cache misses.
    pub cache_miss_total: AtomicU64,
    /// Live entries in the parse cache after the last reparse.
    pub cache_entries: AtomicU64,
    /// Approximate cache memory: sum of segment text bytes that
    /// remained in the cache after the last reparse. Useful for
    /// catching runaway growth on long-lived documents.
    pub cache_bytes_estimate: AtomicU64,
    /// Unix epoch ms of the last successful edit. Zero if no edits
    /// have happened yet.
    pub last_edit_at_unix_ms: AtomicU64,
    /// Parse latency distribution in microseconds. `Mutex` is fine
    /// here: at LSP keystroke rates the lock is uncontended in
    /// practice.
    pub parse_latency_us: Mutex<Histogram<u64>>,
}

impl Default for Metrics {
    fn default() -> Self {
        let hist = Histogram::<u64>::new_with_bounds(HIST_MIN, HIST_MAX, HIST_SIGFIG)
            .expect("`HDRHistogram` bounds are valid constants");
        Self {
            edit_count: AtomicU64::new(0),
            parse_count: AtomicU64::new(0),
            cache_hit_total: AtomicU64::new(0),
            cache_miss_total: AtomicU64::new(0),
            cache_entries: AtomicU64::new(0),
            cache_bytes_estimate: AtomicU64::new(0),
            last_edit_at_unix_ms: AtomicU64::new(0),
            parse_latency_us: Mutex::new(hist),
        }
    }
}

/// One parse sample handed to [`Metrics::record_parse`].
///
/// Bundling these five fields keeps the call signature under the
/// workspace `too-many-arguments-threshold` and gives every call
/// site a self-documenting `ParseSample { … }` literal.
#[derive(Debug, Copy, Clone)]
pub(super) struct ParseSample {
    pub latency_us: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_entries: u64,
    pub cache_bytes_estimate: u64,
}

impl Metrics {
    /// Record one parse: bump counters, record histogram, update
    /// last-edit timestamp.
    pub(super) fn record_parse(&self, sample: ParseSample) {
        self.parse_count.fetch_add(1, Ordering::Relaxed);
        self.cache_hit_total
            .fetch_add(sample.cache_hits, Ordering::Relaxed);
        self.cache_miss_total
            .fetch_add(sample.cache_misses, Ordering::Relaxed);
        self.cache_entries
            .store(sample.cache_entries, Ordering::Relaxed);
        self.cache_bytes_estimate
            .store(sample.cache_bytes_estimate, Ordering::Relaxed);
        // Histogram::record can fail only if the value is out of
        // bounds; we clamp instead of dropping data so the
        // observability path never silently loses samples.
        let clamped = sample.latency_us.clamp(HIST_MIN, HIST_MAX);
        if let Ok(mut h) = self.parse_latency_us.lock() {
            // record_correct(latency, expected_interval) is for
            // sampling latency; here latency is observed directly.
            _ = h.record(clamped);
        }
    }

    /// Record one `did_change` event (independently of parse).
    pub(super) fn record_edit(&self) {
        self.edit_count.fetch_add(1, Ordering::Relaxed);
        // `as_millis()` returns `u128`; clamp to `u64::MAX` for the
        // (pathologically distant) future where the epoch overflow
        // would otherwise wrap. `try_from` keeps the conversion
        // honest without an `as u64` cast that would silently
        // truncate.
        let now_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        self.last_edit_at_unix_ms.store(now_ms, Ordering::Relaxed);
    }

    /// Materialise the current counter values + histogram percentiles
    /// into a `Serialize`-able snapshot. Cheap (bounded number of
    /// atomic loads + one histogram lock).
    pub(super) fn snapshot(&self) -> MetricsSnapshot {
        let (p50, p90, p99, max, count, mean) =
            self.parse_latency_us
                .lock()
                .map_or((0, 0, 0, 0, 0, 0.0), |h| {
                    (
                        h.value_at_quantile(0.50),
                        h.value_at_quantile(0.90),
                        h.value_at_quantile(0.99),
                        h.max(),
                        h.len(),
                        h.mean(),
                    )
                });
        let total_lookups = self.cache_hit_total.load(Ordering::Relaxed)
            + self.cache_miss_total.load(Ordering::Relaxed);
        // Cache hit ratio. The counts are u64 in storage but never
        // realistically exceed `u32::MAX` (≈ 4×10⁹) within a single
        // editor session — clamp through `u32` so the f64 conversion
        // is lossless (`f64::from(u32)` is exact; an `as f64` from
        // u64 trips `clippy::cast_precision_loss`). At the saturation
        // boundary the ratio still rounds correctly.
        let hit_rate = if total_lookups == 0 {
            0.0
        } else {
            let hits =
                u32::try_from(self.cache_hit_total.load(Ordering::Relaxed)).unwrap_or(u32::MAX);
            let total = u32::try_from(total_lookups).unwrap_or(u32::MAX);
            f64::from(hits) / f64::from(total)
        };
        MetricsSnapshot {
            edit_count: self.edit_count.load(Ordering::Relaxed),
            parse_count: self.parse_count.load(Ordering::Relaxed),
            cache_hit_total: self.cache_hit_total.load(Ordering::Relaxed),
            cache_miss_total: self.cache_miss_total.load(Ordering::Relaxed),
            cache_hit_rate: hit_rate,
            cache_entries: self.cache_entries.load(Ordering::Relaxed),
            cache_bytes_estimate: self.cache_bytes_estimate.load(Ordering::Relaxed),
            last_edit_at_unix_ms: self.last_edit_at_unix_ms.load(Ordering::Relaxed),
            parse_latency_us: LatencyPercentiles {
                samples: count,
                mean,
                p50,
                p90,
                p99,
                max,
            },
        }
    }
}

/// Materialised view of [`Metrics`].
///
/// Designed to be `Serialize` so the LSP can dump it into a
/// `tracing::info!` event at `did_close` and a third party parsing
/// the log can reconstruct the document's session-long behaviour.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct MetricsSnapshot {
    pub edit_count: u64,
    pub parse_count: u64,
    pub cache_hit_total: u64,
    pub cache_miss_total: u64,
    pub cache_hit_rate: f64,
    pub cache_entries: u64,
    pub cache_bytes_estimate: u64,
    pub last_edit_at_unix_ms: u64,
    pub parse_latency_us: LatencyPercentiles,
}

/// Latency histogram percentiles in microseconds.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct LatencyPercentiles {
    pub samples: u64,
    pub mean: f64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub max: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: a fresh `Metrics` has zero counters and an empty histogram.
    /// Reproduces: preventive (sanity check on `Default`).
    #[test]
    fn default_metrics_has_zero_counters() {
        let m = Metrics::default();
        let s = m.snapshot();
        assert_eq!(s.edit_count, 0);
        assert_eq!(s.parse_count, 0);
        assert_eq!(s.cache_hit_total, 0);
        assert_eq!(s.cache_miss_total, 0);
        // hit_rate is derived from the integer counters above; a
        // float comparison here would only restate the same fact
        // and would force a `clippy::float_cmp` allow. The integer
        // assertions are the source of truth.
        assert_eq!(s.parse_latency_us.samples, 0);
    }

    /// Invariant: `record_parse` is exact under sequential calls.
    /// Reproduces: preventive.
    #[test]
    fn record_parse_accumulates_exactly() {
        let m = Metrics::default();
        m.record_parse(ParseSample {
            latency_us: 100,
            cache_hits: 5,
            cache_misses: 2,
            cache_entries: 3,
            cache_bytes_estimate: 1024,
        });
        m.record_parse(ParseSample {
            latency_us: 200,
            cache_hits: 10,
            cache_misses: 1,
            cache_entries: 4,
            cache_bytes_estimate: 2048,
        });
        let s = m.snapshot();
        assert_eq!(s.parse_count, 2);
        assert_eq!(s.cache_hit_total, 15);
        assert_eq!(s.cache_miss_total, 3);
        assert_eq!(s.cache_entries, 4);
        assert_eq!(s.cache_bytes_estimate, 2048);
        assert_eq!(s.parse_latency_us.samples, 2);
        assert!(s.parse_latency_us.max >= 200);
    }

    /// Invariant: `record_edit` increments the edit counter and stamps the
    /// `last_edit_at_unix_ms` field non-zero.
    /// Reproduces: preventive.
    #[test]
    fn record_edit_stamps_clock() {
        let m = Metrics::default();
        m.record_edit();
        m.record_edit();
        let s = m.snapshot();
        assert_eq!(s.edit_count, 2);
        assert!(s.last_edit_at_unix_ms > 0);
    }

    /// Invariant: snapshot is `Serialize` and produces valid JSON.
    /// Reproduces: preventive — guards future telemetry export.
    #[test]
    fn snapshot_serialises_to_json() {
        let m = Metrics::default();
        m.record_parse(ParseSample {
            latency_us: 50,
            cache_hits: 1,
            cache_misses: 0,
            cache_entries: 1,
            cache_bytes_estimate: 100,
        });
        let s = m.snapshot();
        let json = serde_json::to_string(&s).expect("snapshot must serialise");
        assert!(json.contains("parse_latency_us"));
        assert!(json.contains("cache_hit_rate"));
    }

    /// Invariant: latency above the histogram's max is clamped, not dropped.
    /// Reproduces: preventive — ensures observability never silently
    /// loses samples on a slow path. `HDRHistogram`'s max bucket may
    /// round up above `HIST_MAX` (bucket-boundary rounding), so we
    /// just assert the sample was recorded; the clamp prevents the
    /// `record` call itself from returning an out-of-range error.
    #[test]
    fn record_parse_clamps_huge_latency() {
        let m = Metrics::default();
        m.record_parse(ParseSample {
            latency_us: u64::MAX,
            cache_hits: 0,
            cache_misses: 0,
            cache_entries: 0,
            cache_bytes_estimate: 0,
        });
        let s = m.snapshot();
        assert_eq!(s.parse_latency_us.samples, 1);
        // `HDRHistogram` bucket rounds up; `max` may be slightly >
        // `HIST_MAX`. We just confirm the sample landed in a finite
        // bucket (i.e. wasn't dropped silently).
        assert!(s.parse_latency_us.max > 0);
    }

    /// Invariant: hit rate computation handles the zero-lookup edge.
    /// Reproduces: preventive.
    #[test]
    fn snapshot_hit_rate_is_zero_when_no_lookups() {
        let m = Metrics::default();
        let s = m.snapshot();
        // The zero-lookup branch sets `hit_rate = 0.0` via a literal,
        // not arithmetic. Compare on the bit pattern: that's the
        // standard Rust idiom for "exactly this f64 value" and it
        // doesn't trip `clippy::float_cmp`, which fires on every
        // direct `==` of f64 values (even literal-against-literal).
        assert_eq!(s.cache_hit_rate.to_bits(), 0.0_f64.to_bits());
    }

    /// Invariant: 1 hit / 4 misses → 20% hit rate.
    /// Reproduces: preventive — exact arithmetic.
    #[test]
    fn snapshot_hit_rate_arithmetic() {
        let m = Metrics::default();
        m.record_parse(ParseSample {
            latency_us: 10,
            cache_hits: 1,
            cache_misses: 4,
            cache_entries: 0,
            cache_bytes_estimate: 0,
        });
        let s = m.snapshot();
        assert!((s.cache_hit_rate - 0.20).abs() < 1e-9);
    }

    /// Invariant: counters survive concurrent updates from multiple threads
    /// (no lost updates under contention).
    /// Reproduces: preventive — guards `regression_metrics_counters_lossless`.
    #[test]
    fn record_parse_is_lossless_under_concurrent_updates() {
        use std::sync::Arc;
        use std::thread;
        let m = Arc::new(Metrics::default());
        let mut handles = Vec::new();
        let n_threads: u32 = 8;
        let per_thread: u32 = 250;
        for _ in 0..n_threads {
            let m = Arc::clone(&m);
            handles.push(thread::spawn(move || {
                for _ in 0..per_thread {
                    m.record_parse(ParseSample {
                        latency_us: 42,
                        cache_hits: 1,
                        cache_misses: 0,
                        cache_entries: 0,
                        cache_bytes_estimate: 0,
                    });
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread must not panic");
        }
        let s = m.snapshot();
        let expected = u64::from(n_threads) * u64::from(per_thread);
        assert_eq!(s.parse_count, expected);
        assert_eq!(s.cache_hit_total, expected);
        assert_eq!(s.parse_latency_us.samples, expected);
    }
}
