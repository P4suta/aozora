//! Provides corpus loading and synthetic inputs for parser benchmarks.

#![forbid(unsafe_code)]

use core::str;
use std::cell::RefCell;
use std::mem;

use aozora::decode_auto_into;
use aozora_corpus::{Archive, ArchivePayload, CorpusItem, FilesystemCorpus, with_load_pool};
use rayon::prelude::*;

const INITIAL_DECODE_BUFFER_BYTES: usize = 128 * 1024;

/// Document size category based on decoded UTF-8 byte length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeBand {
    /// Less than 50 kibibytes.
    Small,
    /// At least 50 and less than 500 kibibytes.
    Medium,
    /// At least 500 kibibytes and less than 2 mebibytes.
    Large,
    /// At least 2 mebibytes.
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

    /// Classifies a decoded UTF-8 byte length using inclusive lower bounds.
    #[must_use]
    pub fn from_bytes(bytes: usize) -> Self {
        const KIB: usize = 1024;
        const MIB: usize = 1024 * 1024;
        const MEDIUM_BAND_START: usize = 50 * KIB;
        const LARGE_BAND_START: usize = 500 * KIB;
        if bytes < MEDIUM_BAND_START {
            Self::Small
        } else if bytes < LARGE_BAND_START {
            Self::Medium
        } else if bytes < 2 * MIB {
            Self::Large
        } else {
            Self::Pathological
        }
    }

    /// Returns size categories in ascending order.
    #[must_use]
    pub fn ordered() -> [Self; 4] {
        [Self::Small, Self::Medium, Self::Large, Self::Pathological]
    }
}

/// Decoded document labels and text grouped by size.
#[derive(Debug, Default)]
pub struct SizeBandedCorpus {
    /// Documents in [`SizeBand::Small`].
    pub small: Vec<(String, String)>,
    /// Documents in [`SizeBand::Medium`].
    pub medium: Vec<(String, String)>,
    /// Documents in [`SizeBand::Large`].
    pub large: Vec<(String, String)>,
    /// Documents in [`SizeBand::Pathological`].
    pub pathological: Vec<(String, String)>,
    /// Number of decode failures.
    pub decode_errors: usize,
    /// Directory-walk or file-read failures encountered while loading.
    pub io_errors: usize,
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

/// Decodes corpus items and groups successful results by UTF-8 byte length.
/// Counts decode failures without stopping the load.
#[must_use]
pub fn corpus_size_bands(items: Vec<CorpusItem>) -> SizeBandedCorpus {
    let mut out = SizeBandedCorpus::default();
    let mut buf = String::with_capacity(128 * 1024);
    for item in items {
        let label = item.label;
        let bytes = item.bytes;
        buf.clear();
        match decode_auto_into(&bytes, &mut buf) {
            Ok(()) => bucket_one(&mut out, (label, mem::take(&mut buf))),
            Err(_) => out.decode_errors += 1,
        }
    }
    out
}

/// Groups decoded document labels and text by UTF-8 byte length.
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

/// Reads, decodes, and groups corpus files in parallel.
/// Counts decode and I/O failures without stopping the load.
/// Ordering within a size category can vary between runs.
#[must_use]
pub fn parallel_size_bands(corpus: &FilesystemCorpus) -> SizeBandedCorpus {
    parallel_size_bands_limited(corpus, None)
}

/// Loads at most `limit` corpus entries in parallel, or all entries for `None`.
/// Traversal errors count toward the limit.
#[must_use]
pub fn parallel_size_bands_limited(
    corpus: &FilesystemCorpus,
    limit: Option<usize>,
) -> SizeBandedCorpus {
    thread_local! {
        static DECODE_BUF: RefCell<String> = RefCell::new(String::with_capacity(INITIAL_DECODE_BUFFER_BYTES));
    }

    let mut paths = Vec::new();
    let mut walk_errors = 0;
    for path in corpus.walk_paths().take(limit.unwrap_or(usize::MAX)) {
        match path {
            Ok(path) => paths.push(path),
            Err(_) => walk_errors += 1,
        }
    }

    let mut banded = with_load_pool(|| {
        paths
            .par_iter()
            .fold(SizeBandedCorpus::default, |mut acc, path| {
                let Ok(item) = corpus.read_path(path) else {
                    acc.io_errors += 1;
                    return acc;
                };
                let outcome = DECODE_BUF.with(|cell| {
                    let mut buf = cell.borrow_mut();
                    buf.clear();
                    match decode_auto_into(&item.bytes, &mut buf) {
                        Ok(()) => Some(mem::take(&mut *buf)),
                        Err(_) => None,
                    }
                });
                match outcome {
                    Some(text) => bucket_one(&mut acc, (item.label, text)),
                    None => acc.decode_errors += 1,
                }
                acc
            })
            .reduce(SizeBandedCorpus::default, merge_banded)
    });
    banded.io_errors += walk_errors;
    banded
}

fn merge_banded(mut a: SizeBandedCorpus, b: SizeBandedCorpus) -> SizeBandedCorpus {
    a.small.extend(b.small);
    a.medium.extend(b.medium);
    a.large.extend(b.large);
    a.pathological.extend(b.pathological);
    a.decode_errors += b.decode_errors;
    a.io_errors += b.io_errors;
    a
}

/// Loads and groups decoded archive entries in parallel.
#[must_use]
pub fn archive_size_bands(archive: &Archive) -> SizeBandedCorpus {
    archive_size_bands_limited(archive, None)
}

/// Loads and groups at most `limit` archive entries, or all entries for `None`.
#[must_use]
pub fn archive_size_bands_limited(archive: &Archive, limit: Option<usize>) -> SizeBandedCorpus {
    use std::cell::RefCell;
    use std::mem;

    let utf8 = archive.is_utf8();
    let entry_count = archive.entries().len().min(limit.unwrap_or(usize::MAX));

    thread_local! {
        static DECODE_BUF: RefCell<String> = RefCell::new(String::with_capacity(INITIAL_DECODE_BUFFER_BYTES));
    }

    with_load_pool(|| {
        (0..entry_count)
            .into_par_iter()
            .fold(SizeBandedCorpus::default, |mut acc, i| {
                let entry_meta = &archive.entries()[i];
                let Ok(payload): Result<ArchivePayload<'_>, _> = archive.payload_at(i) else {
                    acc.decode_errors += 1;
                    return acc;
                };
                let bytes = payload.as_bytes();

                let text = if utf8 {
                    let Ok(s) = str::from_utf8(bytes) else {
                        acc.decode_errors += 1;
                        return acc;
                    };
                    s.to_owned()
                } else {
                    let outcome = DECODE_BUF.with(|cell| {
                        let mut buf = cell.borrow_mut();
                        buf.clear();
                        match decode_auto_into(bytes, &mut buf) {
                            Ok(()) => Some(mem::take(&mut *buf)),
                            Err(_) => None,
                        }
                    });
                    let Some(text) = outcome else {
                        acc.decode_errors += 1;
                        return acc;
                    };
                    text
                };
                bucket_one(&mut acc, (entry_meta.label.clone(), text));
                acc
            })
            .reduce(SizeBandedCorpus::default, merge_banded)
    })
}

/// Returns logarithmic duration buckets as ascending `(low, high, count)` triples.
/// Uses an inclusive `min_ns` and exclusive `max_ns`.
/// Outliers use the first or last bucket.
///
/// # Panics
/// Panics if `buckets` or `min_ns` is zero, or if `min_ns >= max_ns`.
#[must_use]
#[expect(
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

/// Renders a histogram row with a bar proportional to `count / max_count`, capped at `max_width`.
/// An empty bucket uses a space.
#[must_use]
#[expect(
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

/// Generates Aozora source containing at least `target_bytes` bytes.
#[must_use]
pub fn build_synthetic_aozora(target_bytes: usize) -> String {
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

/// Generates notation-dense Aozora source containing at least `target_bytes` bytes.
#[must_use]
pub fn build_pathological_aozora(target_bytes: usize) -> String {
    let unit = "明治の｜人物《じんぶつ》月旦、\
                ※［＃「木＋世」、第3水準1-85-28］なる\
                ｜青梅《おうめ》街道を、［＃「人物」に傍点］\
                ※［＃「にんべん＋弖」、第4水準2-1-52］と\
                ［＃ここから2字下げ］共に訪ね語らふ。［＃ここで字下げ終わり］\n";
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
    use std::fs;
    use std::hint::black_box;

    use aozora_corpus::ArchiveBuilder;
    use aozora_corpus::archive::FLAG_UTF8;

    use super::*;

    #[test]
    fn build_synthetic_returns_at_least_target_bytes() {
        let s = build_synthetic_aozora(1024);
        assert!(s.len() >= 1024);
    }

    #[test]
    fn build_synthetic_parses_without_panic() {
        let s = build_synthetic_aozora(4096);
        let doc = aozora::parse(s).expect("source fits parser span limit");
        let tree = doc.snapshot();
        black_box(tree);
    }

    #[test]
    fn build_pathological_returns_at_least_target_bytes_and_parses() {
        let s = build_pathological_aozora(8192);
        assert!(s.len() >= 8192, "expected >= 8192 bytes, got {}", s.len());
        let doc = aozora::parse(s).expect("source fits parser span limit");
        let tree = doc.snapshot();
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
        let samples: Vec<u64> = vec![1_500, 10_000, 1_000_000, 500_000_000, 2_000_000_000];
        let h = log_histogram_ns(&samples, 6, 1_000, 1_000_000_000);
        assert_eq!(h.len(), 6);
        let total: usize = h.iter().map(|(_, _, c)| c).sum();
        assert_eq!(total, samples.len());
        for window in h.windows(2) {
            assert!(window[0].1 <= window[1].0 + 1, "edges must be ascending");
        }
        assert!(h.last().expect("non-empty").1 >= 1_000_000_000 - 1);
    }

    #[test]
    fn render_bar_row_scales_proportionally_and_pads() {
        let row = render_bar_row("[a..b]", 50, 100, 20);
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
    fn limited_parallel_load_honours_the_document_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(dir.path().join(name), b"text").expect("write fixture");
        }
        let corpus = FilesystemCorpus::new(dir.path()).expect("corpus");

        let banded = parallel_size_bands_limited(&corpus, Some(2));

        assert_eq!(banded.total_docs(), 2);
    }

    #[test]
    fn limited_archive_load_honours_the_document_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corpus.aozc");
        let mut builder = ArchiveBuilder::new(FLAG_UTF8);
        for name in ["a.txt", "b.txt", "c.txt"] {
            builder.push_entry(name, b"text", 0).expect("push entry");
        }
        builder.finish(&path).expect("finish archive");
        let archive = Archive::open(path).expect("open archive");

        let banded = archive_size_bands_limited(&archive, Some(2));

        assert_eq!(banded.total_docs(), 2);
    }

    #[test]
    fn build_synthetic_emits_no_diagnostics_for_well_formed_input() {
        let s = build_synthetic_aozora(2048);
        let doc = aozora::parse(s).expect("source fits parser span limit");
        let tree = doc.snapshot();
        assert!(
            tree.diagnostics().is_empty(),
            "synthetic corpus should be diagnostic-free, got {:?}",
            tree.diagnostics()
        );
    }
}
