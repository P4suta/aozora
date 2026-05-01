//! Per-phase timing breakdown of the lex pipeline.
//!
//! The aggregate `profile_corpus` example measures parse + serialize
//! at the public API boundary; this example reaches inside the lex
//! pipeline and times each phase function individually so we can see
//! which phase actually dominates wall-clock — replacing speculation
//! ("phase 3 is probably 30-40% of parse") with measurement.
//!
//! NOTE (post I-2 deforestation): the production pipeline fuses
//! tokenize → pair → classify into a single iterator chain with no
//! intermediate `Vec` materialisation. This profiling tool deliberately
//! materialises each phase via `.collect()` so the per-phase numbers
//! are individually meaningful — the per-phase costs reported here
//! include the materialisation overhead and are NOT a faithful
//! reflection of the fused pipeline's instruction-cache / cache-line
//! behaviour. Use `lex_into_arena` totals for end-to-end numbers.
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
//! - `AOZORA_PROFILE_PARALLEL=1` — fan per-doc measurements across
//!   rayon's pool. Per-doc latencies remain
//!   meaningful (each closure timing is local), but the wall-clock
//!   collapses to `serial-work / N`. The progress log is suppressed
//!   under parallel mode to avoid interleaved output; one summary
//!   line replaces the per-2k-doc trickle.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    clippy::needless_collect,
    reason = "profiling-example tool, not library code; per-phase .collect() calls are intentional materialisation so each phase can be timed in isolation"
)]

use std::cmp::Reverse;
use std::env;
use std::process;
use std::time::Instant;

use std::cell::RefCell;

use aozora_corpus::{CorpusItem, CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_sjis;
use aozora_pipeline::lex_into_arena;
use aozora_pipeline::lexer::{
    ClassifiedSpan, PairEvent, Token, classify, pair, sanitize, tokenize,
};
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;
use rayon::prelude::*;

// One arena per worker thread per measurement role. Reused across
// docs by resetting between parses. The two arenas
// are kept separate because the Phase 3 measurement and the full
// pipeline measurement run back-to-back inside a single
// `measure_one` call — sharing one arena would force a reset
// mid-call, after which the prior measurement's borrowed output
// would be invalidated.
//
// A: pre-size with 256 KB initial capacity to
// skip the first-few-docs chunk-grow churn. See the matching
// constant in `throughput_by_class.rs` for the heuristic rationale.
//
// `RefCell` matches `Arena`'s `!Sync` contract (each rayon worker
// owns its own thread-local cell, never shared across threads).
const WORKER_ARENA_INITIAL_CAPACITY: usize = 256 * 1024;

thread_local! {
    static WORKER_ARENA_PHASE3: RefCell<Arena> = RefCell::new(Arena::with_capacity(WORKER_ARENA_INITIAL_CAPACITY));
    static WORKER_ARENA_FULL: RefCell<Arena> = RefCell::new(Arena::with_capacity(WORKER_ARENA_INITIAL_CAPACITY));
}

const NS_PER_MS: f64 = 1_000_000.0;
const NS_PER_S: f64 = 1_000_000_000.0;

#[derive(Debug, Clone, Copy, Default)]
struct PhaseSample {
    bytes_in: u64,
    sanitize_ns: u64,
    tokenize_ns: u64,
    pair_ns: u64,
    classify_ns: u64,
    /// `lex_into_arena` total — everything from sanitize through the
    /// fused `ArenaNormalizer` walk that builds the borrowed registry.
    full_ns: u64,
    /// Derived: `full_ns - (sanitize + tokenize + pair + classify)`.
    /// Estimate of the post-classify normalize+registry-build cost
    /// that was the legacy phase 4-6.
    post_classify_ns: u64,
    /// Sum of the four standalone phases.
    total_ns: u64,
}

fn main() {
    let Some(corpus) = corpus_from_env() else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set; nothing to profile.\n\
             usage: AOZORA_CORPUS_ROOT=/path/to/corpus \
             cargo run --release --example phase_breakdown -p aozora-bench"
        );
        process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let parallel = parallel_mode();

    eprintln!("phase_breakdown: starting (limit = {limit:?}, parallel = {parallel})");

    // Drain the corpus so I/O isn't mixed into per-phase numbers.
    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!("phase_breakdown: loaded {} items, measuring…", items.len());

    let wall_start = Instant::now();
    let (samples, labels, decode_errors) = measure_corpus(&items, parallel);
    let wall_elapsed = wall_start.elapsed();
    eprintln!(
        "phase_breakdown: done in {:.2}s, {} decode errors",
        wall_elapsed.as_secs_f64(),
        decode_errors
    );

    print_report(&samples, &labels, wall_elapsed.as_nanos() as u64, parallel);
}

/// Whether to fan per-doc measurements across rayon's pool. Opt-in
/// via `AOZORA_PROFILE_PARALLEL=1`.
fn parallel_mode() -> bool {
    matches!(
        env::var("AOZORA_PROFILE_PARALLEL").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

/// Per-doc result. `None` flags a Shift-JIS decode error so the
/// post-collect aggregator can count them instead of using a shared
/// `AtomicU64`. `par_iter().map(...).collect()` preserves input
/// order, so the `samples` / `labels` vectors stay aligned with the
/// original corpus iteration order — `Top-5 by classify` rankings
/// match between sequential and parallel runs.
struct DocResult {
    sample: PhaseSample,
    label: String,
}

fn measure_corpus(items: &[CorpusItem], parallel: bool) -> (Vec<PhaseSample>, Vec<String>, usize) {
    let process = |item: &CorpusItem| -> Option<DocResult> {
        let text = decode_sjis(&item.bytes).ok()?;
        Some(DocResult {
            sample: measure_one(&text),
            label: item.label.clone(),
        })
    };

    let results: Vec<Option<DocResult>> = if parallel {
        items.par_iter().map(process).collect()
    } else {
        items.iter().map(process).collect()
    };

    let mut samples: Vec<PhaseSample> = Vec::with_capacity(results.len());
    let mut labels: Vec<String> = Vec::with_capacity(results.len());
    let mut decode_errors = 0usize;
    for r in results {
        match r {
            Some(d) => {
                samples.push(d.sample);
                labels.push(d.label);
            }
            None => decode_errors += 1,
        }
    }
    (samples, labels, decode_errors)
}

fn measure_one(text: &str) -> PhaseSample {
    let bytes_in = text.len() as u64;

    // Phase 0
    let t = Instant::now();
    let sanitized = sanitize(text);
    let sanitize_ns = t.elapsed().as_nanos() as u64;

    // Phase 1 — collect into a Vec for per-phase isolation. Production
    // pipeline (`lex_into_arena`) does NOT materialise; see the file
    // header.
    let t = Instant::now();
    let tokens: Vec<Token> = tokenize(&sanitized.text).collect();
    let tokenize_ns = t.elapsed().as_nanos() as u64;

    // Phase 2 — same caveat as Phase 1.
    let t = Instant::now();
    let mut pair_stream = pair(tokens.into_iter());
    let pair_events: Vec<PairEvent> = (&mut pair_stream).collect();
    drop(pair_stream.take_diagnostics());
    let pair_ns = t.elapsed().as_nanos() as u64;

    // Phase 3 — needs an arena + allocator. Borrows the per-worker
    // reusable arena and resets it before parsing so the prior doc's
    // allocations don't bloat this measurement. The arena is
    // pre-sized to `text.len() * 4` so the chunk-grow `mmap` fires
    // before the per-phase timer rather than inside it.
    let classify_ns = WORKER_ARENA_PHASE3.with(|cell| {
        let mut arena = cell.borrow_mut();
        arena.reset_with_hint(text.len().saturating_mul(4));
        let mut alloc = BorrowedAllocator::new(&arena);
        let t = Instant::now();
        let mut classify_stream = classify(pair_events, &sanitized.text, &mut alloc);
        let _classify_spans: Vec<ClassifiedSpan<'_>> = (&mut classify_stream).collect();
        drop(classify_stream.take_diagnostics());
        t.elapsed().as_nanos() as u64
    });

    // Full pipeline (sanitize → arena registry build). Includes the
    // post-classify ArenaNormalizer walk (the work that the legacy
    // phases 4–6 used to perform). Subtracting the four standalone
    // phases from `full_ns` gives an estimate of the post-classify
    // cost without us having to reach into `aozora-pipeline`'s private
    // builder. Same per-worker arena reuse as the Phase 3 block —
    // separate cell because the two measurements would otherwise
    // share one arena and reset mid-call.
    let full_ns = WORKER_ARENA_FULL.with(|cell| {
        let mut arena = cell.borrow_mut();
        arena.reset_with_hint(text.len().saturating_mul(4));
        let t = Instant::now();
        let _full = lex_into_arena(text, &arena);
        t.elapsed().as_nanos() as u64
    });

    let standalone_sum = sanitize_ns + tokenize_ns + pair_ns + classify_ns;
    let post_classify_ns = full_ns.saturating_sub(standalone_sum);
    let total_ns = standalone_sum;

    PhaseSample {
        bytes_in,
        sanitize_ns,
        tokenize_ns,
        pair_ns,
        classify_ns,
        full_ns,
        post_classify_ns,
        total_ns,
    }
}

fn print_report(samples: &[PhaseSample], labels: &[String], wall_ns: u64, parallel: bool) {
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
        samples.iter().map(|s| s.post_classify_ns).sum::<u64>(),
        samples.iter().map(|s| s.full_ns).sum::<u64>(),
        samples.iter().map(|s| s.total_ns).sum::<u64>(),
    );
    let (sanitize, tokenize, pair_, classify_, post_classify_, full_, total) = sums;

    println!("=== aozora-pipeline phase breakdown ===");
    println!();
    println!("Corpus");
    println!("  docs              : {n}");
    println!(
        "  bytes (sanitised) : {} ({:.2} MB)",
        total_bytes,
        total_bytes as f64 / (1024.0 * 1024.0)
    );
    println!("  wall-clock        : {:.2} s", wall_ns as f64 / NS_PER_S);
    if parallel {
        // sum of per-doc full_ns is what serial execution would have
        // taken; wall_ns is the achieved concurrent wall-clock.
        let serial_full_ns: u64 = samples.iter().map(|s| s.full_ns).sum();
        let scaling = if wall_ns > 0 {
            serial_full_ns as f64 / wall_ns as f64
        } else {
            0.0
        };
        let threads = rayon::current_num_threads();
        println!(
            "  parallel          : {threads} threads, scaling {scaling:.2}× \
             (serial work {:.2}s)",
            serial_full_ns as f64 / NS_PER_S
        );
    }
    println!();

    println!("Per-phase totals (sum across all docs)");
    print_phase_row("phase 0 sanitize ", sanitize, total, total_bytes);
    print_phase_row("phase 1 tokenize ", tokenize, total, total_bytes);
    print_phase_row("phase 2 pair     ", pair_, total, total_bytes);
    print_phase_row("phase 3 classify ", classify_, total, total_bytes);
    println!("  ─────────────────────────────────────────────────");
    print_phase_row("4 standalone sum ", total, total, total_bytes);
    print_phase_row("post-classify (∼) ", post_classify_, full_, total_bytes);
    print_phase_row("lex_into_arena   ", full_, full_, total_bytes);
    println!();

    println!("Per-doc latency (per phase, microseconds)");
    print_phase_quantiles(
        "sanitize     ",
        samples.iter().map(|s| s.sanitize_ns).collect(),
    );
    print_phase_quantiles(
        "tokenize     ",
        samples.iter().map(|s| s.tokenize_ns).collect(),
    );
    print_phase_quantiles("pair         ", samples.iter().map(|s| s.pair_ns).collect());
    print_phase_quantiles(
        "classify     ",
        samples.iter().map(|s| s.classify_ns).collect(),
    );
    print_phase_quantiles(
        "post-classify",
        samples.iter().map(|s| s.post_classify_ns).collect(),
    );
    print_phase_quantiles(
        "lex_into_arena",
        samples.iter().map(|s| s.full_ns).collect(),
    );
    print_phase_quantiles(
        "4-PHASE TOTAL",
        samples.iter().map(|s| s.total_ns).collect(),
    );
    println!();

    // Identify the top-3 docs by classify_ns — likely the
    // pathological annotation-density outliers.
    let mut by_classify: Vec<(usize, &PhaseSample)> = samples.iter().enumerate().collect();
    by_classify.sort_by_key(|(_, s)| Reverse(s.classify_ns));
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
    let mut by_sanitize: Vec<(usize, &PhaseSample)> = samples.iter().enumerate().collect();
    by_sanitize.sort_by_key(|(_, s)| Reverse(s.sanitize_ns));
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
    let throughput_mbps = total_bytes as f64 * NS_PER_S / phase_ns as f64 / (1024.0 * 1024.0);
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
