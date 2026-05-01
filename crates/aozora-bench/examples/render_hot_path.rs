//! Per-size-class **render** throughput report.
//!
//! Mirror of `throughput_by_class.rs` but for `aozora_render::html`.
//! For each corpus document the probe:
//!
//! 1. Pre-parses the doc once via `lex_into_arena` (untimed — the
//!    parse perf story lives in the other probes; we want a clean
//!    render-bound number here)
//! 2. Renders the parsed `BorrowedLexOutput` to HTML via
//!    `render_to_string`, repeating `AOZORA_RENDER_REPEAT` times so
//!    the per-doc latency is stable and so a `samply` trace gets
//!    enough render-bound wall time to attach to (without the repeat,
//!    the smallest docs spend more time in parse than render and the
//!    sample stack is dominated by lex internals)
//!
//! Reports per-band MB/s (input bytes per render second), p50/p90/p99
//! / max latency, ns/byte, and an aggregate render-vs-parse ratio so
//! the reader can see at a glance whether render time dominates parse
//! time on a given doc class.
//!
//! Reads `AOZORA_CORPUS_ROOT`. Optional `AOZORA_PROFILE_LIMIT=N` caps
//! the sweep. Optional `AOZORA_RENDER_REPEAT=K` controls the per-doc
//! render loop count (default 1; raise to 5+ when running under
//! samply so the parse warmup doesn't dominate the trace).
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example render_hot_path -p aozora-bench
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::disallowed_methods,
    reason = "profiling-example tool, not library code"
)]

use std::env;
use std::process;
use std::time::Instant;

use aozora::html;
use aozora_bench::{SizeBand, SizeBandedCorpus, corpus_size_bands};
use aozora_corpus::CorpusItem;
use aozora_pipeline::lex_into_arena;
use aozora_syntax::borrowed::Arena;

const NS_PER_S: f64 = 1_000_000_000.0;

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let repeat: usize = env::var("AOZORA_RENDER_REPEAT")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1)
        .max(1);

    eprintln!("render_hot_path: starting (limit = {limit:?}, repeat = {repeat})");

    let load_start = Instant::now();
    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!("render_hot_path: loaded {} items, bucketing…", items.len());
    let banded = corpus_size_bands(items);
    let load_secs = load_start.elapsed().as_secs_f64();
    eprintln!(
        "render_hot_path: bucketed (small={}, medium={}, large={}, path={}, decode_err={})",
        banded.small.len(),
        banded.medium.len(),
        banded.large.len(),
        banded.pathological.len(),
        banded.decode_errors,
    );
    eprintln!(
        "render_hot_path: load wall {load_secs:.2}s (Shift-JIS decode + bucketing — \
         excluded from render measurements)"
    );

    let measure_start = Instant::now();
    let report = measure_all(&banded, repeat);
    let measure_secs = measure_start.elapsed().as_secs_f64();

    print_report(&report, &banded, measure_secs, load_secs, repeat);
}

#[derive(Debug, Default)]
struct BandReport {
    /// Per-doc render latency in ns (median of `repeat` runs).
    render_ns: Vec<u64>,
    /// Per-doc parse latency in ns (single run, untimed in the loop).
    parse_ns: Vec<u64>,
    /// Per-doc input size (post-decode UTF-8 bytes).
    sizes_bytes: Vec<u64>,
    /// Per-doc output HTML size in bytes (one of the K renders;
    /// they all produce the same string).
    html_bytes: Vec<u64>,
}

impl BandReport {
    fn total_input_bytes(&self) -> u64 {
        self.sizes_bytes.iter().sum()
    }
    fn total_html_bytes(&self) -> u64 {
        self.html_bytes.iter().sum()
    }
    fn total_render_ns(&self) -> u64 {
        self.render_ns.iter().sum()
    }
    fn total_parse_ns(&self) -> u64 {
        self.parse_ns.iter().sum()
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

fn measure_all(banded: &SizeBandedCorpus, repeat: usize) -> AllReport {
    let mut report = AllReport::default();
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let docs = banded.band(band);
        let mut render_ns: Vec<u64> = Vec::with_capacity(docs.len());
        let mut parse_ns: Vec<u64> = Vec::with_capacity(docs.len());
        let mut sizes: Vec<u64> = Vec::with_capacity(docs.len());
        let mut html_bytes: Vec<u64> = Vec::with_capacity(docs.len());

        for (_, text) in docs {
            // Parse once (timed for the parse-vs-render ratio summary
            // line below; not on the render hot path itself).
            let arena = Arena::new();
            let t = Instant::now();
            let out = lex_into_arena(text, &arena);
            parse_ns.push(t.elapsed().as_nanos() as u64);

            // Render `repeat` times, keep the median ns.
            let mut samples: Vec<u64> = Vec::with_capacity(repeat);
            let mut last_html_len: u64 = 0;
            for _ in 0..repeat {
                let t = Instant::now();
                let html = html::render_to_string(&out);
                samples.push(t.elapsed().as_nanos() as u64);
                last_html_len = html.len() as u64;
                drop(html);
            }
            samples.sort_unstable();
            let median = samples[samples.len() / 2];
            render_ns.push(median);
            sizes.push(text.len() as u64);
            html_bytes.push(last_html_len);
        }

        report.bands[slot] = BandReport {
            render_ns,
            parse_ns,
            sizes_bytes: sizes,
            html_bytes,
        };
    }
    report
}

fn print_report(
    report: &AllReport,
    banded: &SizeBandedCorpus,
    measure_secs: f64,
    load_secs: f64,
    repeat: usize,
) {
    println!("=== render_hot_path ===");
    println!();
    println!(
        "Corpus: {} docs across 4 bands; {} decode errors",
        banded.total_docs(),
        banded.decode_errors,
    );
    println!(
        "Wall:    load {load_secs:.2}s   measure {measure_secs:.2}s ({repeat} render{plural} per doc)",
        plural = if repeat == 1 { "" } else { "s" }
    );
    println!();
    println!(
        "{:<13} {:>6} {:>13} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "band", "docs", "in bytes", "MB/s", "p50 µs", "p90 µs", "p99 µs", "max ms", "ns/in-byte"
    );
    println!("{}", "-".repeat(106));
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let r = &report.bands[slot];
        let docs = r.render_ns.len();
        if docs == 0 {
            println!(
                "{:<13} {:>6} {:>13} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
                band.label(),
                0,
                0,
                "-",
                "-",
                "-",
                "-",
                "-",
                "-"
            );
            continue;
        }
        let mut sorted = r.render_ns.clone();
        sorted.sort_unstable();
        let p50 = BandReport::quantile(&sorted, 0.5);
        let p90 = BandReport::quantile(&sorted, 0.9);
        let p99 = BandReport::quantile(&sorted, 0.99);
        let max_ns = *sorted.last().expect("non-empty");
        let in_bytes = r.total_input_bytes();
        let render_ns = r.total_render_ns();
        let mb_per_s = if render_ns == 0 {
            0.0
        } else {
            (in_bytes as f64) * NS_PER_S / (render_ns as f64) / (1024.0 * 1024.0)
        };
        let ns_per_byte = if in_bytes == 0 {
            0.0
        } else {
            (render_ns as f64) / (in_bytes as f64)
        };

        println!(
            "{:<13} {:>6} {:>13} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.2} {:>10.2}",
            band.label(),
            docs,
            in_bytes,
            mb_per_s,
            p50 as f64 / 1_000.0,
            p90 as f64 / 1_000.0,
            p99 as f64 / 1_000.0,
            max_ns as f64 / 1_000_000.0,
            ns_per_byte,
        );
    }

    println!();
    println!("Render output / input ratio + render-vs-parse cost share:");
    println!(
        "{:<13} {:>10} {:>14} {:>14} {:>16}",
        "band", "out/in", "parse total ms", "render total ms", "render / parse"
    );
    println!("{}", "-".repeat(70));
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let r = &report.bands[slot];
        if r.render_ns.is_empty() {
            continue;
        }
        let in_bytes = r.total_input_bytes();
        let html_bytes = r.total_html_bytes();
        let out_in = if in_bytes == 0 {
            0.0
        } else {
            (html_bytes as f64) / (in_bytes as f64)
        };
        let parse_ms = (r.total_parse_ns() as f64) / 1_000_000.0;
        let render_ms = (r.total_render_ns() as f64) / 1_000_000.0;
        let render_per_parse = if parse_ms > 0.0 {
            render_ms / parse_ms
        } else {
            0.0
        };
        println!(
            "{:<13} {:>10.2} {:>14.2} {:>14.2} {:>16.2}",
            band.label(),
            out_in,
            parse_ms,
            render_ms,
            render_per_parse,
        );
    }
}
