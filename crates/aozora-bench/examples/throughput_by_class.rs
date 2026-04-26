//! Per-size-class throughput report.
//!
//! Splits the corpus into four bands by post-decode UTF-8 byte length:
//!
//! - **Small**       `< 50 KiB`
//! - **Medium**      `50 KiB ..= 500 KiB`
//! - **Large**       `500 KiB ..= 2 MiB`
//! - **Pathological** `> 2 MiB`
//!
//! For each band, reports doc count, total bytes, single-thread parse
//! throughput (MB/s), p50 / p90 / p99 / max latency (ns), and per-byte
//! cost (ns/byte). The phase_breakdown probe lumps every doc together
//! and so over-weights the very few pathological documents that
//! dominate aggregate wall-clock; this probe makes the per-class
//! distribution visible.
//!
//! Reads `AOZORA_CORPUS_ROOT`. Optional `AOZORA_PROFILE_LIMIT=N` caps
//! the sweep.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example throughput_by_class -p aozora-bench
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "profiling-example tool, not library code"
)]

use std::env;
use std::time::Instant;

use aozora_bench::{SizeBand, SizeBandedCorpus, corpus_size_bands};
use aozora_corpus::CorpusItem;
use aozora_lex::lex_into_arena;
use aozora_syntax::borrowed::Arena;

const NS_PER_S: f64 = 1_000_000_000.0;

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile."
        );
        std::process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("throughput_by_class: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!("throughput_by_class: loaded {} items, bucketing…", items.len());
    let banded = corpus_size_bands(items);
    eprintln!(
        "throughput_by_class: bucketed (small={}, medium={}, large={}, path={}, decode_err={})",
        banded.small.len(),
        banded.medium.len(),
        banded.large.len(),
        banded.pathological.len(),
        banded.decode_errors,
    );

    let wall_start = Instant::now();
    let report = measure_all(&banded);
    let wall_elapsed = wall_start.elapsed();

    print_report(&report, &banded, wall_elapsed.as_secs_f64());
}

#[derive(Debug, Default)]
struct BandReport {
    /// Per-doc total parse latency (`lex_into_arena`) in ns.
    latencies_ns: Vec<u64>,
    /// Per-doc input size (post-decode UTF-8 bytes).
    sizes_bytes: Vec<u64>,
}

impl BandReport {
    fn total_bytes(&self) -> u64 {
        self.sizes_bytes.iter().sum()
    }
    fn total_ns(&self) -> u64 {
        self.latencies_ns.iter().sum()
    }
    fn quantile(sorted: &[u64], p: f64) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
}

#[derive(Debug, Default)]
struct AllReport {
    bands: [BandReport; 4],
}

fn measure_all(banded: &SizeBandedCorpus) -> AllReport {
    let mut report = AllReport::default();
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let docs = banded.band(band);
        let mut latencies = Vec::with_capacity(docs.len());
        let mut sizes = Vec::with_capacity(docs.len());
        for (_, text) in docs {
            // Fresh arena per doc — matches `lex_into_arena`'s own
            // contract and avoids amortising allocator state across
            // multiple parses.
            let arena = Arena::new();
            let t = Instant::now();
            let _out = lex_into_arena(text, &arena);
            let ns = t.elapsed().as_nanos() as u64;
            latencies.push(ns);
            sizes.push(text.len() as u64);
        }
        report.bands[slot] = BandReport {
            latencies_ns: latencies,
            sizes_bytes: sizes,
        };
    }
    report
}

fn print_report(report: &AllReport, banded: &SizeBandedCorpus, wall_secs: f64) {
    println!("=== throughput_by_class ===");
    println!();
    println!(
        "Corpus: {} docs across 4 bands; {} decode errors; {:.2}s wall",
        banded.total_docs(),
        banded.decode_errors,
        wall_secs
    );
    println!();
    println!(
        "{:<13} {:>6} {:>13} {:>10} {:>10} {:>10} {:>10} {:>11} {:>10}",
        "band", "docs", "bytes", "MB/s", "p50 µs", "p90 µs", "p99 µs", "max ms", "ns/byte"
    );
    println!("{}", "-".repeat(106));
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let r = &report.bands[slot];
        let docs = r.latencies_ns.len();
        if docs == 0 {
            println!("{:<13} {:>6} {:>13} {:>10} {:>10} {:>10} {:>10} {:>11} {:>10}",
                band.label(), 0, 0, "-", "-", "-", "-", "-", "-");
            continue;
        }
        let mut sorted = r.latencies_ns.clone();
        sorted.sort_unstable();
        let p50 = BandReport::quantile(&sorted, 0.5);
        let p90 = BandReport::quantile(&sorted, 0.9);
        let p99 = BandReport::quantile(&sorted, 0.99);
        let max_ns = *sorted.last().expect("non-empty");
        let total_bytes = r.total_bytes();
        let total_ns = r.total_ns();
        let mb_per_s = if total_ns == 0 {
            0.0
        } else {
            (total_bytes as f64) * NS_PER_S / (total_ns as f64) / (1024.0 * 1024.0)
        };
        let ns_per_byte = if total_bytes == 0 {
            0.0
        } else {
            (total_ns as f64) / (total_bytes as f64)
        };
        println!(
            "{:<13} {:>6} {:>13} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>11.2} {:>10.2}",
            band.label(),
            docs,
            total_bytes,
            mb_per_s,
            (p50 as f64) / 1000.0,
            (p90 as f64) / 1000.0,
            (p99 as f64) / 1000.0,
            (max_ns as f64) / 1_000_000.0,
            ns_per_byte,
        );
    }
}
