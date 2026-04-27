//! Bench harness for the aozora parser.
//!
//! This crate hosts criterion benchmarks shared across the workspace
//! and is also the **canonical PGO profile source**: when the
//! release pipeline is configured to do PGO + BOLT
//! (`docs/adr/0009-clean-layered-architecture.md` Move 4 verification
//! plan), the profile collection step runs
//! `cargo run --release --bin aozora_pgo_train` against the full
//! corpus to gather an even sample of real-world parse work.
//!
//! ## Why a separate crate?
//!
//! - The benches need both `aozora` (the public API) and the
//!   internal `aozora-corpus` walker, neither of which fit in any
//!   one of the existing crates without inverting deps.
//! - Keeping bench harnesses together means we can run
//!   `cargo bench -p aozora-bench` and have one place to point CI's
//!   regression-detection workflow at.

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::path::Path;

use aozora::Document;
use aozora_corpus::{CorpusItem, CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_sjis;

/// Iterate the corpus rooted at `root`, decode SJIS bytes to UTF-8,
/// and parse each document.
///
/// Returns the `(decode_error_count, io_error_count, parsed_doc_count)`
/// triple. Used by the PGO training binary AND by the
/// synthetic-corpus criterion harness.
///
/// # Errors
///
/// Returns `Err` on a corpus-construction failure (typically: the
/// supplied root is not a directory). Per-file errors are counted
/// internally rather than raised — the goal is "exercise as much of
/// the parse path as possible" for profiling.
pub fn parse_corpus<P: AsRef<Path>>(
    root: P,
) -> Result<(usize, usize, usize), aozora_corpus::CorpusError> {
    let corpus = FilesystemCorpus::new(root.as_ref())?;
    let mut decode_errors = 0;
    let mut io_errors = 0;
    let mut parsed = 0;
    for item in corpus.iter() {
        match item {
            Ok(CorpusItem { bytes, .. }) => match decode_sjis(&bytes) {
                Ok(text) => {
                    let doc = Document::new(text.clone());
                    let tree = doc.parse();
                    // Touch the tree so the optimizer can't hoist
                    // the parse out — a real consumer reads
                    // diagnostics and the registry, both of which we
                    // surface here to match prod-shaped pressure.
                    let diag_count = tree.diagnostics().len();
                    black_box(diag_count);
                    parsed += 1;
                }
                Err(_) => decode_errors += 1,
            },
            Err(_) => io_errors += 1,
        }
    }
    Ok((decode_errors, io_errors, parsed))
}

/// Size band a corpus document falls into, by post-decode UTF-8 byte count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeBand {
    /// `< 50 KiB`
    Small,
    /// `50 KiB ..= 500 KiB`
    Medium,
    /// `500 KiB ..= 2 MiB`
    Large,
    /// `> 2 MiB`
    Pathological,
}

impl SizeBand {
    /// Stable display label used in probe output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "<50KB",
            Self::Medium => "50KB-500KB",
            Self::Large => "500KB-2MB",
            Self::Pathological => ">2MB",
        }
    }

    /// Bucket a UTF-8 byte length into a band. Boundaries are inclusive
    /// at the lower edge: 50 KiB → Medium, 500 KiB → Large, 2 MiB →
    /// Pathological. The expected `bytes` is the post-decode UTF-8 length.
    #[must_use]
    pub fn from_bytes(bytes: usize) -> Self {
        const KIB: usize = 1024;
        const MIB: usize = 1024 * 1024;
        if bytes < 50 * KIB {
            Self::Small
        } else if bytes < 500 * KIB {
            Self::Medium
        } else if bytes < 2 * MIB {
            Self::Large
        } else {
            Self::Pathological
        }
    }

    /// Iteration order matching the human-friendly small → pathological
    /// progression. Used by every probe that prints per-band rows so
    /// the report layout stays stable across runs.
    #[must_use]
    pub fn ordered() -> [Self; 4] {
        [Self::Small, Self::Medium, Self::Large, Self::Pathological]
    }
}

/// Corpus items pre-bucketed into size bands. Each band's `Vec` holds
/// the (label, decoded UTF-8 text) pairs that fell into that band.
///
/// Built by [`corpus_size_bands`] from a freshly drained corpus. Probes
/// iterate in [`SizeBand::ordered`] order so reports are deterministic.
#[derive(Debug, Default)]
pub struct SizeBandedCorpus {
    /// `< 50 KiB` bucket.
    pub small: Vec<(String, String)>,
    /// `50 KiB ..= 500 KiB` bucket.
    pub medium: Vec<(String, String)>,
    /// `500 KiB ..= 2 MiB` bucket.
    pub large: Vec<(String, String)>,
    /// `> 2 MiB` bucket.
    pub pathological: Vec<(String, String)>,
    /// SJIS decode failures encountered while bucketing. Probes report
    /// this so the user can sanity-check the corpus root.
    pub decode_errors: usize,
}

impl SizeBandedCorpus {
    /// Borrow the docs of a given band as `(label, text)` pairs.
    #[must_use]
    pub fn band(&self, band: SizeBand) -> &[(String, String)] {
        match band {
            SizeBand::Small => &self.small,
            SizeBand::Medium => &self.medium,
            SizeBand::Large => &self.large,
            SizeBand::Pathological => &self.pathological,
        }
    }

    /// Total document count across all bands.
    #[must_use]
    pub fn total_docs(&self) -> usize {
        self.small.len() + self.medium.len() + self.large.len() + self.pathological.len()
    }
}

/// Decode every corpus item to UTF-8 (Shift_JIS) and bucket the
/// successful decodes by post-decode byte length.
///
/// SJIS decode failures are counted in
/// [`SizeBandedCorpus::decode_errors`] but never raised; this matches
/// the philosophy of the other probes ("exercise as much as possible,
/// surface the error counts at the end").
#[must_use]
pub fn corpus_size_bands(items: Vec<CorpusItem>) -> SizeBandedCorpus {
    let mut out = SizeBandedCorpus::default();
    for item in items {
        // CorpusItem is `#[non_exhaustive]`; bind the two fields by
        // name without the destructuring sugar.
        let label = item.label;
        let bytes = item.bytes;
        match decode_sjis(&bytes) {
            Ok(text) => bucket_one(&mut out, (label, text)),
            Err(_) => out.decode_errors += 1,
        }
    }
    out
}

/// Bucket pre-decoded `(label, text)` pairs by post-decode byte length.
///
/// Used by per-phase load-split benchmarks (L-1) to time the decode
/// step in isolation from the bucketing step. The two-step shape lets
/// callers measure `decode_secs` as just-the-decode work and
/// `bucket_secs` as just-the-bucketing work, without one polluting the
/// other.
#[must_use]
pub fn corpus_size_bands_from_decoded(items: Vec<(String, String)>) -> SizeBandedCorpus {
    let mut out = SizeBandedCorpus::default();
    for entry in items {
        bucket_one(&mut out, entry);
    }
    out
}

fn bucket_one(out: &mut SizeBandedCorpus, entry: (String, String)) {
    let band = SizeBand::from_bytes(entry.1.len());
    match band {
        SizeBand::Small => out.small.push(entry),
        SizeBand::Medium => out.medium.push(entry),
        SizeBand::Large => out.large.push(entry),
        SizeBand::Pathological => out.pathological.push(entry),
    }
}

/// Logarithmic-bucket histogram over an `&[u64]` of nanosecond durations.
///
/// `min_ns` and `max_ns` set the inclusive lower / exclusive upper
/// edges of the histogram range; samples outside fall in the first /
/// last bucket respectively. Returns `(low, high, count)` triples in
/// ascending bucket order. Bucket boundaries are computed in `ln`-space
/// so each bucket spans an equal multiplicative ratio. With 10 buckets
/// across `1µs..1s` each bucket covers a ratio of
/// `(1e9/1e3)^(1/10) ≈ 4×`.
///
/// # Panics
///
/// Panics if `buckets == 0` or if `min_ns == 0` or `min_ns >= max_ns`
/// — the bucket-bound math requires a strictly positive range.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "histogram bucket math operates on small ns counts; \
              precision loss is intentional for human-readable output"
)]
pub fn log_histogram_ns(
    samples: &[u64],
    buckets: usize,
    min_ns: u64,
    max_ns: u64,
) -> Vec<(u64, u64, usize)> {
    assert!(buckets >= 1, "log_histogram_ns: buckets must be >= 1");
    assert!(
        min_ns > 0 && max_ns > min_ns,
        "log_histogram_ns: require 0 < min_ns < max_ns",
    );

    let log_lo = (min_ns as f64).ln();
    let log_hi = (max_ns as f64).ln();
    let step = (log_hi - log_lo) / (buckets as f64);

    let mut edges: Vec<u64> = Vec::with_capacity(buckets + 1);
    for i in 0..=buckets {
        let edge = step.mul_add(i as f64, log_lo).exp();
        edges.push(edge as u64);
    }

    let mut counts = vec![0usize; buckets];
    for &s in samples {
        let idx = if s <= edges[0] {
            0
        } else if s >= edges[buckets] {
            buckets - 1
        } else {
            let pos = ((s as f64).ln() - log_lo) / step;
            (pos as usize).min(buckets - 1)
        };
        counts[idx] += 1;
    }

    (0..buckets)
        .map(|i| (edges[i], edges[i + 1], counts[i]))
        .collect()
}

/// Render a single histogram bar row of the form `<label>  <bar>  <count>`.
///
/// The bar is `█` repeated with width proportional to `count / max_count`,
/// capped at `max_width` cells. Empty buckets render as a single space
/// so columns stay aligned.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bar widths are small integers"
)]
pub fn render_bar_row(label: &str, count: usize, max_count: usize, max_width: usize) -> String {
    let width = if max_count == 0 {
        0
    } else {
        let scaled = (count as f64 / max_count as f64) * max_width as f64;
        scaled.round() as usize
    };
    let bar: String = if width == 0 {
        String::new()
    } else {
        "█".repeat(width)
    };
    format!("{label}  {bar:max_width$}  {count}")
}

/// Build a synthetic Aozora-shaped buffer of approximately
/// `target_bytes` size.
///
/// Mixes plain hiragana, kanji, ruby spans, and annotations so the
/// parser exercises every classifier path. Used by
/// `benches/synthetic_corpus.rs` to give a portable benchmark that
/// doesn't depend on a local corpus checkout.
#[must_use]
pub fn build_synthetic_aozora(target_bytes: usize) -> String {
    // One repeating unit covers each major construct exactly once.
    let unit = "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\
                なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\
                ［＃ここで字下げ終わり］\n\n";
    let unit_bytes = unit.len();
    let cycles = target_bytes.div_ceil(unit_bytes);
    let mut s = String::with_capacity(cycles * unit_bytes);
    for _ in 0..cycles {
        s.push_str(unit);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_synthetic_returns_at_least_target_bytes() {
        let s = build_synthetic_aozora(1024);
        assert!(s.len() >= 1024);
    }

    #[test]
    fn build_synthetic_parses_without_panic() {
        let s = build_synthetic_aozora(4096);
        let doc = Document::new(s);
        let tree = doc.parse();
        black_box(tree);
    }

    #[test]
    fn size_band_boundaries_are_inclusive_at_lower_edge() {
        assert_eq!(SizeBand::from_bytes(0), SizeBand::Small);
        assert_eq!(SizeBand::from_bytes(50 * 1024 - 1), SizeBand::Small);
        assert_eq!(SizeBand::from_bytes(50 * 1024), SizeBand::Medium);
        assert_eq!(SizeBand::from_bytes(500 * 1024 - 1), SizeBand::Medium);
        assert_eq!(SizeBand::from_bytes(500 * 1024), SizeBand::Large);
        assert_eq!(SizeBand::from_bytes(2 * 1024 * 1024 - 1), SizeBand::Large);
        assert_eq!(
            SizeBand::from_bytes(2 * 1024 * 1024),
            SizeBand::Pathological
        );
        assert_eq!(SizeBand::from_bytes(usize::MAX), SizeBand::Pathological);
    }

    #[test]
    fn log_histogram_ns_distributes_samples_across_buckets() {
        // 1µs..1s, 6 buckets — ratio per bucket ≈ √(1e6) ≈ 31.6×.
        let samples: Vec<u64> = vec![
            1_500,         // bucket 0 (≈ 1µs..32µs)
            10_000,        // bucket 0 or 1
            1_000_000,     // bucket 2 or 3
            500_000_000,   // bucket 5
            2_000_000_000, // overflow → bucket 5
        ];
        let h = log_histogram_ns(&samples, 6, 1_000, 1_000_000_000);
        assert_eq!(h.len(), 6);
        let total: usize = h.iter().map(|(_, _, c)| c).sum();
        assert_eq!(total, samples.len());
        // Edges must be ascending and the very last edge >= 1s.
        for window in h.windows(2) {
            assert!(window[0].1 <= window[1].0 + 1, "edges must be ascending");
        }
        assert!(h.last().expect("non-empty").1 >= 1_000_000_000 - 1);
    }

    #[test]
    fn render_bar_row_scales_proportionally_and_pads() {
        let row = render_bar_row("[a..b]", 50, 100, 20);
        // Width should be ~10 chars (50/100 * 20).
        let bar_chars = row.matches('█').count();
        assert!(
            (8..=12).contains(&bar_chars),
            "expected ~10 bars, got {bar_chars}: {row}"
        );
        assert!(row.starts_with("[a..b]  "));
        assert!(row.trim_end().ends_with("50"));
    }

    #[test]
    fn render_bar_row_handles_zero_max() {
        let row = render_bar_row("zero", 0, 0, 10);
        assert!(!row.contains('█'));
        assert!(row.trim_end().ends_with('0'));
    }

    #[test]
    fn corpus_size_bands_buckets_decoded_items() {
        // Build synthetic SJIS-decodable items by encoding ASCII.
        let small = CorpusItem::new("small", b"hello".to_vec());
        let medium = CorpusItem::new("medium", vec![b'a'; 100 * 1024]);
        let banded = corpus_size_bands(vec![small, medium]);
        assert_eq!(banded.small.len(), 1);
        assert_eq!(banded.medium.len(), 1);
        assert_eq!(banded.large.len(), 0);
        assert_eq!(banded.pathological.len(), 0);
        assert_eq!(banded.decode_errors, 0);
        assert_eq!(banded.total_docs(), 2);
    }

    #[test]
    fn build_synthetic_emits_no_diagnostics_for_well_formed_input() {
        let s = build_synthetic_aozora(2048);
        let doc = Document::new(s);
        let tree = doc.parse();
        assert!(
            tree.diagnostics().is_empty(),
            "synthetic corpus should be diagnostic-free, got {:?}",
            tree.diagnostics()
        );
    }
}
