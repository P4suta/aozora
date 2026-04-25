// Profiling example: precision-loss casts are intentional (computing
// quantiles), and CLI exit-on-misconfig is appropriate for a binary.
// Workspace lints assume "library code"; this is a tool.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::match_same_arms,
    clippy::redundant_clone,
    clippy::absolute_paths,
    clippy::single_match_else,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    clippy::allow_attributes_without_reason,
    clippy::option_if_let_else,
    reason = "profiling-example tool, not library code"
)]

//! Profile aozora-parser against a full filesystem corpus.
//!
//! Reads `AOZORA_CORPUS_ROOT`, walks every `.txt` file underneath,
//! decodes Shift_JIS, and runs the parse / serialize / incremental
//! fast-path pipeline. Reports a multi-angle latency + throughput
//! breakdown to stdout.
//!
//! ```text
//! cargo run --release --example profile_corpus -p aozora-parser
//! ```
//!
//! Optional env vars:
//! - `AOZORA_PROFILE_LIMIT=N` — cap to first N corpus items (default: all)
//! - `AOZORA_PROFILE_INCREMENTAL=1` — also run a 1-keystroke incremental
//!   fast-path probe per doc and tally fast-path hit rate (slower)
//! - `AOZORA_PROFILE_DOC_PARALLEL=1` — process documents in parallel
//!   via rayon (Layer 1: doc-level parallelism). Reports total
//!   wall-clock throughput; per-doc latency stats become noisier
//!   under thread contention but are still emitted.

use std::env;
use std::time::{Duration, Instant};

use aozora_corpus::{CorpusItem, from_env};
use aozora_encoding::decode_sjis;
use aozora_parser::{TextEdit, parse, parse_incremental, serialize};
use rayon::prelude::*;

const NS_PER_MS: f64 = 1_000_000.0;
const NS_PER_S: f64 = 1_000_000_000.0;

/// All measurements for a single corpus item.
#[derive(Debug, Clone, Copy)]
struct DocSample {
    bytes_in: usize,
    chars_in: usize,
    /// Wall-clock parse time.
    parse_ns: u64,
    /// Wall-clock serialize time (parse → serialize round-trip cost).
    serialize_ns: u64,
    /// Number of diagnostics produced.
    diagnostics: usize,
    /// Number of registry entries (≈ recognised aozora annotations).
    registry: usize,
    /// `Some(ns)` if incremental probe ran. None when probe is disabled.
    incremental_ns: Option<u64>,
    /// True iff the incremental fast path fired for this doc.
    incremental_fast_path: Option<bool>,
}

fn main() {
    let Some(corpus) = from_env() else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set; nothing to profile.\n\
             usage: AOZORA_CORPUS_ROOT=/path/to/corpus \
             cargo run --release --example profile_corpus -p aozora-parser"
        );
        std::process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let with_incremental = env::var_os("AOZORA_PROFILE_INCREMENTAL").is_some();
    let doc_parallel = env::var_os("AOZORA_PROFILE_DOC_PARALLEL").is_some();

    eprintln!(
        "profile_corpus: provenance = {} | limit = {:?} | incremental = {} | doc_parallel = {}",
        corpus.provenance(),
        limit,
        with_incremental,
        doc_parallel,
    );

    // 1) Drain the corpus into a Vec first (sequentially — corpus
    //    iteration is I/O bound and not parallelism-friendly).
    let mut items: Vec<CorpusItem> = Vec::with_capacity(20_000);
    let mut decode_errors = 0usize;
    let mut io_errors = 0usize;
    for (i, item) in corpus.iter().enumerate() {
        if let Some(cap) = limit
            && i >= cap
        {
            break;
        }
        match item {
            Ok(it) => items.push(it),
            Err(_) => io_errors += 1,
        }
    }
    eprintln!(
        "profile_corpus: loaded {} items ({} I/O errors); starting measurement…",
        items.len(),
        io_errors,
    );

    // 2) Measurement phase. Sequential timing keeps per-doc latency
    //    stats clean; parallel timing measures peak throughput.
    let wall_start = Instant::now();
    let samples: Vec<DocSample> = if doc_parallel {
        let (samples, errs) = items
            .par_iter()
            .map(|item| match decode_sjis(&item.bytes) {
                Ok(text) => Ok(measure_one(&text, with_incremental)),
                Err(_) => Err(()),
            })
            .fold(
                || (Vec::<DocSample>::new(), 0usize),
                |(mut acc, errs), r| match r {
                    Ok(s) => {
                        acc.push(s);
                        (acc, errs)
                    }
                    Err(()) => (acc, errs + 1),
                },
            )
            .reduce(
                || (Vec::<DocSample>::new(), 0usize),
                |(mut a, ea), (b, eb)| {
                    a.extend(b);
                    (a, ea + eb)
                },
            );
        decode_errors += errs;
        samples
    } else {
        let mut samples = Vec::with_capacity(items.len());
        for item in &items {
            let Ok(text) = decode_sjis(&item.bytes) else {
                decode_errors += 1;
                continue;
            };
            samples.push(measure_one(&text, with_incremental));
            if samples.len().is_multiple_of(2_000) {
                eprintln!(
                    "  …processed {} docs ({:.1}s elapsed)",
                    samples.len(),
                    wall_start.elapsed().as_secs_f64(),
                );
            }
        }
        samples
    };
    let wall_elapsed = wall_start.elapsed();

    print_report(
        &samples,
        wall_elapsed,
        decode_errors,
        io_errors,
        with_incremental,
    );
}

fn measure_one(text: &str, with_incremental: bool) -> DocSample {
    let bytes_in = text.len();
    let chars_in = text.chars().count();

    let t0 = Instant::now();
    let parsed = parse(text);
    let parse_ns = t0.elapsed().as_nanos() as u64;

    let t1 = Instant::now();
    drop(serialize(&parsed));
    let serialize_ns = t1.elapsed().as_nanos() as u64;

    let diagnostics = parsed.diagnostics.len();
    let registry = parsed.artifacts.registry.len();

    let (incremental_ns, incremental_fast_path) = if with_incremental {
        // Insert a single ASCII char at the head of the doc and time
        // parse_incremental. Captures the LSP-keystroke-style hot path.
        let edit = vec![TextEdit::new(0..0, "x".to_owned())];
        let t2 = Instant::now();
        match parse_incremental(&parsed, text, &edit) {
            Ok(outcome) => {
                let ns = t2.elapsed().as_nanos() as u64;
                let fast = matches!(
                    outcome.decision,
                    aozora_parser::IncrementalDecision::PlainTextWindow
                );
                (Some(ns), Some(fast))
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    DocSample {
        bytes_in,
        chars_in,
        parse_ns,
        serialize_ns,
        diagnostics,
        registry,
        incremental_ns,
        incremental_fast_path,
    }
}

fn print_report(
    samples: &[DocSample],
    wall: Duration,
    decode_errors: usize,
    io_errors: usize,
    with_incremental: bool,
) {
    let n = samples.len();
    if n == 0 {
        println!("No docs processed.");
        return;
    }
    let total_bytes: u64 = samples.iter().map(|s| s.bytes_in as u64).sum();
    let total_chars: u64 = samples.iter().map(|s| s.chars_in as u64).sum();
    let total_parse_ns: u64 = samples.iter().map(|s| s.parse_ns).sum();
    let total_serialize_ns: u64 = samples.iter().map(|s| s.serialize_ns).sum();
    let total_diag: u64 = samples.iter().map(|s| s.diagnostics as u64).sum();
    let total_registry: u64 = samples.iter().map(|s| s.registry as u64).sum();
    let docs_with_diag = samples.iter().filter(|s| s.diagnostics > 0).count();
    let docs_with_registry = samples.iter().filter(|s| s.registry > 0).count();

    let mut parse_ns: Vec<u64> = samples.iter().map(|s| s.parse_ns).collect();
    parse_ns.sort_unstable();
    let mut serialize_ns: Vec<u64> = samples.iter().map(|s| s.serialize_ns).collect();
    serialize_ns.sort_unstable();
    let mut bytes: Vec<u64> = samples.iter().map(|s| s.bytes_in as u64).collect();
    bytes.sort_unstable();
    let mut registry: Vec<u64> = samples.iter().map(|s| s.registry as u64).collect();
    registry.sort_unstable();
    // Per-byte parse rate (ns/byte) — controls for doc length variance.
    let mut ns_per_byte: Vec<f64> = samples
        .iter()
        .filter(|s| s.bytes_in > 0)
        .map(|s| s.parse_ns as f64 / s.bytes_in as f64)
        .collect();
    ns_per_byte.sort_by(|a, b| a.partial_cmp(b).unwrap());

    println!("=== aozora-parser corpus profile ===");
    println!();
    println!("Corpus");
    println!("  docs processed     : {n}");
    println!("  decode errors      : {decode_errors}");
    println!("  i/o errors         : {io_errors}");
    println!(
        "  total bytes        : {} ({:.2} MB)",
        total_bytes,
        total_bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  total chars        : {} ({:.2} M)",
        total_chars,
        total_chars as f64 / 1_000_000.0
    );
    println!("  wall-clock         : {:.2} s", wall.as_secs_f64());
    println!();

    println!("Parse latency (per doc)");
    print_ns_quantiles("  parse", &parse_ns);
    println!(
        "  total parse time (sum CPU): {:.2} s ({:.1}% of wall)",
        total_parse_ns as f64 / NS_PER_S,
        total_parse_ns as f64 / wall.as_nanos() as f64 * 100.0
    );
    println!(
        "  per-thread throughput     : {:.2} MB/s ({:.2} M chars/s)",
        total_bytes as f64 * NS_PER_S / total_parse_ns as f64 / (1024.0 * 1024.0),
        total_chars as f64 * NS_PER_S / total_parse_ns as f64 / 1_000_000.0,
    );
    // Aggregate wall-clock throughput: under doc-level parallelism this
    // is the meaningful number, since per-thread time sums across all
    // workers in flight at once.
    println!(
        "  aggregate wall-clock      : {:.2} MB/s ({:.2} M chars/s)",
        total_bytes as f64 / wall.as_secs_f64() / (1024.0 * 1024.0),
        total_chars as f64 / wall.as_secs_f64() / 1_000_000.0,
    );
    println!();

    println!("Per-byte parse cost (controls for doc length)");
    print_f64_quantiles("  ns/byte", &ns_per_byte);
    println!();

    println!("Serialize latency (per doc)");
    print_ns_quantiles("  serialize", &serialize_ns);
    println!(
        "  total serialize  : {:.2} s",
        total_serialize_ns as f64 / NS_PER_S
    );
    println!(
        "  serialize throughput : {:.2} MB/s",
        total_bytes as f64 * NS_PER_S / total_serialize_ns as f64 / (1024.0 * 1024.0),
    );
    println!();

    println!("Document size distribution (bytes)");
    print_u64_quantiles("  bytes", &bytes);
    println!();

    println!("Registry size (annotations recognised per doc)");
    print_u64_quantiles("  registry", &registry);
    println!(
        "  total registry entries : {total_registry} \
         ({:.1} per doc on avg)",
        total_registry as f64 / n as f64
    );
    println!(
        "  docs with ≥1 annotation: {docs_with_registry} ({:.1}%)",
        docs_with_registry as f64 / n as f64 * 100.0
    );
    println!();

    println!("Diagnostics");
    println!("  total diagnostics : {total_diag}");
    println!(
        "  docs with ≥1 diag : {docs_with_diag} ({:.2}%)",
        docs_with_diag as f64 / n as f64 * 100.0
    );
    println!();

    if with_incremental {
        let inc_samples: Vec<u64> = samples.iter().filter_map(|s| s.incremental_ns).collect();
        let fast_path_hits = samples
            .iter()
            .filter(|s| s.incremental_fast_path == Some(true))
            .count();
        let probed = samples
            .iter()
            .filter(|s| s.incremental_fast_path.is_some())
            .count();
        let mut sorted_inc = inc_samples.clone();
        sorted_inc.sort_unstable();

        println!("Incremental probe (1-byte insert at head)");
        println!("  probed docs     : {probed}");
        println!(
            "  fast-path hits  : {fast_path_hits} ({:.2}% of probed)",
            fast_path_hits as f64 / probed.max(1) as f64 * 100.0
        );
        if !sorted_inc.is_empty() {
            print_ns_quantiles("  incremental", &sorted_inc);
        }
        println!();
    }

    println!("Annotation density (top-3 by registry size)");
    let mut top_by_reg: Vec<(usize, &DocSample)> = samples.iter().enumerate().collect();
    top_by_reg.sort_by_key(|(_, s)| std::cmp::Reverse(s.registry));
    for (idx, s) in top_by_reg.iter().take(3) {
        println!(
            "  doc #{idx}: {} bytes / {} chars / {} annotations / {} diagnostics",
            s.bytes_in, s.chars_in, s.registry, s.diagnostics
        );
    }
    println!();

    println!("Slowest parse (top-3 by ns)");
    let mut top_by_lat: Vec<(usize, &DocSample)> = samples.iter().enumerate().collect();
    top_by_lat.sort_by_key(|(_, s)| std::cmp::Reverse(s.parse_ns));
    for (idx, s) in top_by_lat.iter().take(3) {
        println!(
            "  doc #{idx}: {:.2} ms parse / {} bytes / {} annotations \
             ({:.2} MB/s)",
            s.parse_ns as f64 / NS_PER_MS,
            s.bytes_in,
            s.registry,
            s.bytes_in as f64 * NS_PER_S / s.parse_ns as f64 / (1024.0 * 1024.0),
        );
    }
}

fn print_ns_quantiles(label: &str, sorted: &[u64]) {
    let mean: u64 = sorted.iter().copied().sum::<u64>() / sorted.len() as u64;
    println!(
        "{label}: min {min:.2}µs / p50 {p50:.2}µs / p90 {p90:.2}µs / \
         p99 {p99:.2}µs / max {max:.2}ms / mean {mean:.2}µs",
        min = sorted[0] as f64 / 1000.0,
        p50 = q(sorted, 0.5) as f64 / 1000.0,
        p90 = q(sorted, 0.9) as f64 / 1000.0,
        p99 = q(sorted, 0.99) as f64 / 1000.0,
        max = q(sorted, 1.0) as f64 / NS_PER_MS,
        mean = mean as f64 / 1000.0,
    );
}

fn print_u64_quantiles(label: &str, sorted: &[u64]) {
    let mean: u64 = sorted.iter().copied().sum::<u64>() / sorted.len() as u64;
    println!(
        "{label}: min {min} / p50 {p50} / p90 {p90} / p99 {p99} / max {max} / mean {mean}",
        min = sorted[0],
        p50 = q(sorted, 0.5),
        p90 = q(sorted, 0.9),
        p99 = q(sorted, 0.99),
        max = q(sorted, 1.0),
    );
}

fn print_f64_quantiles(label: &str, sorted: &[f64]) {
    let sum: f64 = sorted.iter().sum();
    let mean = sum / sorted.len() as f64;
    println!(
        "{label}: min {min:.2} / p50 {p50:.2} / p90 {p90:.2} / p99 {p99:.2} / \
         max {max:.2} / mean {mean:.2}",
        min = sorted[0],
        p50 = qf(sorted, 0.5),
        p90 = qf(sorted, 0.9),
        p99 = qf(sorted, 0.99),
        max = qf(sorted, 1.0),
    );
}

fn q(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn qf(sorted: &[f64], p: f64) -> f64 {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
