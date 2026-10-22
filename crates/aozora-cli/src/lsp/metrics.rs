#![expect(
    clippy::expect_used,
    reason = "the static Prometheus metric definitions are valid"
)]

//! Records document edit counts, cache activity, and parse latency through [`Metrics`].

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;
use serde::Serialize;

const HIST_MIN: u64 = 1;
const HIST_MAX: u64 = 1_000_000_000;
const HIST_SIGFIG: u8 = 3;

const MEDIAN_QUANTILE: f64 = 0.50;
const P90_QUANTILE: f64 = 0.90;
const P99_QUANTILE: f64 = 0.99;

#[derive(Debug)]
pub(super) struct Metrics {
    pub edit_count: AtomicU64,
    pub parse_count: AtomicU64,
    pub cache_hit_total: AtomicU64,
    pub cache_miss_total: AtomicU64,
    pub cache_entries: AtomicU64,
    /// Total cached segment text bytes.
    pub cache_bytes_estimate: AtomicU64,
    /// Unix epoch milliseconds of the last edit, or zero before the first edit.
    pub last_edit_at_unix_ms: AtomicU64,
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

#[derive(Debug, Copy, Clone)]
pub(super) struct ParseSample {
    pub latency_us: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_entries: u64,
    pub cache_bytes_estimate: u64,
}

impl Metrics {
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
        let clamped = sample.latency_us.clamp(HIST_MIN, HIST_MAX);
        if let Ok(mut h) = self.parse_latency_us.lock() {
            _ = h.record(clamped);
        }
    }

    pub(super) fn record_edit(&self) {
        self.edit_count.fetch_add(1, Ordering::Relaxed);
        let now_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        self.last_edit_at_unix_ms.store(now_ms, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> MetricsSnapshot {
        let (p50, p90, p99, max, count, mean) =
            self.parse_latency_us
                .lock()
                .map_or((0, 0, 0, 0, 0, 0.0), |h| {
                    (
                        h.value_at_quantile(MEDIAN_QUANTILE),
                        h.value_at_quantile(P90_QUANTILE),
                        h.value_at_quantile(P99_QUANTILE),
                        h.max(),
                        h.len(),
                        h.mean(),
                    )
                });
        let total_lookups = self.cache_hit_total.load(Ordering::Relaxed)
            + self.cache_miss_total.load(Ordering::Relaxed);
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

    #[test]
    fn default_metrics_has_zero_counters() {
        let m = Metrics::default();
        let s = m.snapshot();
        assert_eq!(s.edit_count, 0);
        assert_eq!(s.parse_count, 0);
        assert_eq!(s.cache_hit_total, 0);
        assert_eq!(s.cache_miss_total, 0);
        assert_eq!(s.parse_latency_us.samples, 0);
    }

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

    #[test]
    fn record_edit_stamps_clock() {
        let m = Metrics::default();
        m.record_edit();
        m.record_edit();
        let s = m.snapshot();
        assert_eq!(s.edit_count, 2);
        assert!(s.last_edit_at_unix_ms > 0);
    }

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
        assert!(s.parse_latency_us.max > 0);
    }

    #[test]
    fn snapshot_hit_rate_is_zero_when_no_lookups() {
        let m = Metrics::default();
        let s = m.snapshot();
        assert_eq!(s.cache_hit_rate.to_bits(), 0.0_f64.to_bits());
    }

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
