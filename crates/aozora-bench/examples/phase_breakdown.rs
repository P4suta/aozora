//! Per-phase timing breakdown of the lex pipeline.
//!
//! The aggregate `profile_corpus` example measures parse + serialize
//! at the public API boundary; this example reaches inside the lex
//! pipeline and times each phase function individually so we can see
//! which phase actually dominates wall-clock — replacing speculation
//! ("phase 3 is probably 30-40% of parse") with measurement.
//!
//! Reads `AOZORA_CORPUS_ROOT` (same convention as `profile_corpus`),
//! walks every `.txt` under it, decodes Shift_JIS, and runs the
//! six lex phases manually with [`Instant::now`] around each call.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example phase_breakdown -p aozora-bench
//! ```
//!
//! Optional env vars:
//! - `AOZORA_PROFILE_LIMIT=N` — cap the sweep to the first N docs
//!   (useful for spot checks during development)

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

use aozora_corpus::{CorpusItem, CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_sjis;
use aozora_lexer::{classify, normalize, pair, sanitize, tokenize, validate};

const NS_PER_MS: f64 = 1_000_000.0;
const NS_PER_S: f64 = 1_000_000_000.0;

#[derive(Debug, Clone, Copy, Default)]
struct PhaseSample {
    bytes_in: u64,
    sanitize_ns: u64,
    tokenize_ns: u64,
    pair_ns: u64,
    classify_ns: u64,
    normalize_ns: u64,
    validate_ns: u64,
    /// All-phases sum.
    total_ns: u64,
}

fn main() {
    let Some(corpus) = corpus_from_env() else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set; nothing to profile.\n\
             usage: AOZORA_CORPUS_ROOT=/path/to/corpus \
             cargo run --release --example phase_breakdown -p aozora-bench"
        );
        std::process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!(
        "phase_breakdown: starting (limit = {limit:?})"
    );

    // Drain the corpus so I/O isn't mixed into per-phase numbers.
    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!("phase_breakdown: loaded {} items, measuring…", items.len());

    let wall_start = Instant::now();
    let mut samples: Vec<PhaseSample> = Vec::with_capacity(items.len());
    let mut labels: Vec<String> = Vec::with_capacity(items.len());
    let mut decode_errors = 0usize;
    for (i, item) in items.iter().enumerate() {
        let Ok(text) = decode_sjis(&item.bytes) else {
            decode_errors += 1;
            continue;
        };
        samples.push(measure_one(&text));
        labels.push(item.label.clone());
        if (i + 1).is_multiple_of(2_000) {
            eprintln!(
                "  …processed {} docs ({:.1}s elapsed)",
                i + 1,
                wall_start.elapsed().as_secs_f64()
            );
        }
    }
    let wall_elapsed = wall_start.elapsed();
    eprintln!(
        "phase_breakdown: done in {:.2}s, {} decode errors",
        wall_elapsed.as_secs_f64(),
        decode_errors
    );

    print_report(&samples, &labels, wall_elapsed.as_nanos() as u64);
}

fn measure_one(text: &str) -> PhaseSample {
    let bytes_in = text.len() as u64;

    // Phase 0
    let t = Instant::now();
    let sanitized = sanitize(text);
    let sanitize_ns = t.elapsed().as_nanos() as u64;

    // Phase 1
    let t = Instant::now();
    let tokens = tokenize(&sanitized.text);
    let tokenize_ns = t.elapsed().as_nanos() as u64;

    // Phase 2
    let t = Instant::now();
    let pair_out = pair(&tokens);
    let pair_ns = t.elapsed().as_nanos() as u64;

    // Phase 3
    let t = Instant::now();
    let classify_out = classify(&pair_out, &sanitized.text);
    let classify_ns = t.elapsed().as_nanos() as u64;

    // Phase 4
    let t = Instant::now();
    let normalize_out = normalize(&classify_out, &sanitized.text);
    let normalize_ns = t.elapsed().as_nanos() as u64;

    // Phase 6
    let t = Instant::now();
    let _validated = validate(normalize_out);
    let validate_ns = t.elapsed().as_nanos() as u64;

    let total_ns =
        sanitize_ns + tokenize_ns + pair_ns + classify_ns + normalize_ns + validate_ns;

    PhaseSample {
        bytes_in,
        sanitize_ns,
        tokenize_ns,
        pair_ns,
        classify_ns,
        normalize_ns,
        validate_ns,
        total_ns,
    }
}

fn print_report(samples: &[PhaseSample], labels: &[String], wall_ns: u64) {
    if samples.is_empty() {
        println!("No samples processed.");
        return;
    }
    let n = samples.len();
    let total_bytes: u64 = samples.iter().map(|s| s.bytes_in).sum();

    let sums = (
        samples.iter().map(|s| s.sanitize_ns).sum::<u64>(),
        samples.iter().map(|s| s.tokenize_ns).sum::<u64>(),
        samples.iter().map(|s| s.pair_ns).sum::<u64>(),
        samples.iter().map(|s| s.classify_ns).sum::<u64>(),
        samples.iter().map(|s| s.normalize_ns).sum::<u64>(),
        samples.iter().map(|s| s.validate_ns).sum::<u64>(),
        samples.iter().map(|s| s.total_ns).sum::<u64>(),
    );
    let (sanitize, tokenize, pair_, classify_, normalize_, validate_, total) = sums;

    println!("=== aozora-lex phase breakdown ===");
    println!();
    println!("Corpus");
    println!("  docs              : {n}");
    println!("  bytes (sanitised) : {} ({:.2} MB)", total_bytes, total_bytes as f64 / (1024.0 * 1024.0));
    println!("  wall-clock        : {:.2} s", wall_ns as f64 / NS_PER_S);
    println!();

    println!("Per-phase totals (sum across all docs)");
    print_phase_row("phase 0 sanitize", sanitize, total, total_bytes);
    print_phase_row("phase 1 tokenize", tokenize, total, total_bytes);
    print_phase_row("phase 2 pair    ", pair_, total, total_bytes);
    print_phase_row("phase 3 classify", classify_, total, total_bytes);
    print_phase_row("phase 4 normalize", normalize_, total, total_bytes);
    print_phase_row("phase 6 validate", validate_, total, total_bytes);
    println!("  ─────────────────────────────────────────────────");
    print_phase_row("ALL              ", total, total, total_bytes);
    println!();

    println!("Per-doc latency (per phase, microseconds)");
    print_phase_quantiles("sanitize ", samples.iter().map(|s| s.sanitize_ns).collect());
    print_phase_quantiles("tokenize ", samples.iter().map(|s| s.tokenize_ns).collect());
    print_phase_quantiles("pair     ", samples.iter().map(|s| s.pair_ns).collect());
    print_phase_quantiles("classify ", samples.iter().map(|s| s.classify_ns).collect());
    print_phase_quantiles("normalize", samples.iter().map(|s| s.normalize_ns).collect());
    print_phase_quantiles("validate ", samples.iter().map(|s| s.validate_ns).collect());
    print_phase_quantiles("TOTAL    ", samples.iter().map(|s| s.total_ns).collect());
    println!();

    // Identify the top-3 docs by classify_ns — likely the
    // pathological annotation-density outliers.
    let mut by_classify: Vec<(usize, &PhaseSample)> =
        samples.iter().enumerate().collect();
    by_classify.sort_by_key(|(_, s)| std::cmp::Reverse(s.classify_ns));
    println!("Top-5 docs by phase 3 classify cost");
    for (idx, s) in by_classify.iter().take(5) {
        let label = labels.get(*idx).map_or("?", String::as_str);
        println!(
            "  classify {:>7.2} ms / total {:>7.2} ms / {:>8} bytes — {label}",
            s.classify_ns as f64 / NS_PER_MS,
            s.total_ns as f64 / NS_PER_MS,
            s.bytes_in
        );
    }

    println!();
    println!("Top-5 docs by sanitize cost (phase 0 was unexpectedly hot)");
    let mut by_sanitize: Vec<(usize, &PhaseSample)> =
        samples.iter().enumerate().collect();
    by_sanitize.sort_by_key(|(_, s)| std::cmp::Reverse(s.sanitize_ns));
    for (idx, s) in by_sanitize.iter().take(5) {
        let label = labels.get(*idx).map_or("?", String::as_str);
        println!(
            "  sanitize {:>7.2} ms / total {:>7.2} ms / {:>8} bytes — {label}",
            s.sanitize_ns as f64 / NS_PER_MS,
            s.total_ns as f64 / NS_PER_MS,
            s.bytes_in
        );
    }
}

fn print_phase_row(label: &str, phase_ns: u64, total_ns: u64, total_bytes: u64) {
    let pct = phase_ns as f64 * 100.0 / total_ns as f64;
    let throughput_mbps =
        total_bytes as f64 * NS_PER_S / phase_ns as f64 / (1024.0 * 1024.0);
    println!(
        "  {label} : {:>6.0} ms ({:>5.1}%) — {:>6.1} MB/s",
        phase_ns as f64 / NS_PER_MS,
        pct,
        throughput_mbps
    );
}

fn print_phase_quantiles(label: &str, mut values: Vec<u64>) {
    values.sort_unstable();
    let n = values.len();
    let q = |p: f64| -> f64 {
        let idx = ((n as f64 - 1.0) * p).round() as usize;
        values[idx.min(n - 1)] as f64 / 1000.0
    };
    let mean = values.iter().sum::<u64>() as f64 / n as f64 / 1000.0;
    println!(
        "  {label}: p50 {:>6.2} µs / p90 {:>7.2} µs / p99 {:>8.2} µs / max {:>8.2} ms / mean {:>6.2} µs",
        q(0.5),
        q(0.9),
        q(0.99),
        q(1.0) * 1000.0 / 1_000_000.0, // µs → ms for max
        mean
    );
}

fn corpus_from_env() -> Option<Box<dyn CorpusSource>> {
    let root = env::var_os("AOZORA_CORPUS_ROOT")?;
    Some(Box::new(FilesystemCorpus::new(root).ok()?))
}
