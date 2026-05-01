//! Per-phase + total-parse log-bucketed latency histogram.
//!
//! Replaces the per-phase scalar quantiles in `phase_breakdown` with a
//! 10-bucket logarithmic histogram covering 1µs to 1s. For each phase
//! and the full pipeline this prints a horizontal bar chart so the
//! tail shape (multi-modal? long tail? clustered?) is visible at a
//! glance.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example latency_histogram -p aozora-bench
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
    clippy::needless_collect,
    reason = "profiling-example tool, not library code; the per-phase .collect() calls are intentional materialisation so each phase can be timed independently"
)]

use std::env;
use std::process;
use std::time::Instant;

use aozora_bench::{log_histogram_ns, render_bar_row};
use aozora_corpus::CorpusItem;
use aozora_encoding::decode_sjis;
use aozora_pipeline::lex_into_arena;
use aozora_pipeline::lexer::{
    ClassifiedSpan, PairEvent, Token, classify, pair, sanitize, tokenize,
};
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;

const BUCKETS: usize = 10;
const MIN_NS: u64 = 1_000;
const MAX_NS: u64 = 1_000_000_000;

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        process::exit(2);
    };
    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("latency_histogram: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!("latency_histogram: loaded {} items", items.len());

    let mut sanitize_ns: Vec<u64> = Vec::with_capacity(items.len());
    let mut tokenize_ns: Vec<u64> = Vec::with_capacity(items.len());
    let mut pair_ns: Vec<u64> = Vec::with_capacity(items.len());
    let mut classify_ns: Vec<u64> = Vec::with_capacity(items.len());
    let mut full_ns: Vec<u64> = Vec::with_capacity(items.len());
    let mut decode_errors = 0usize;

    for item in &items {
        let Ok(text) = decode_sjis(&item.bytes) else {
            decode_errors += 1;
            continue;
        };

        let t = Instant::now();
        let sanitized = sanitize(&text);
        sanitize_ns.push(t.elapsed().as_nanos() as u64);

        let t = Instant::now();
        let tokens: Vec<Token> = tokenize(&sanitized.text).collect();
        tokenize_ns.push(t.elapsed().as_nanos() as u64);

        let t = Instant::now();
        let mut ps = pair(tokens.into_iter());
        let pe: Vec<PairEvent> = (&mut ps).collect();
        drop(ps.take_diagnostics());
        pair_ns.push(t.elapsed().as_nanos() as u64);

        let arena_p3 = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena_p3);
        let t = Instant::now();
        let mut cs = classify(pe, &sanitized.text, &mut alloc);
        let _spans: Vec<ClassifiedSpan<'_>> = (&mut cs).collect();
        drop(cs.take_diagnostics());
        classify_ns.push(t.elapsed().as_nanos() as u64);

        let arena_full = Arena::new();
        let t = Instant::now();
        let _full = lex_into_arena(&text, &arena_full);
        full_ns.push(t.elapsed().as_nanos() as u64);
    }

    eprintln!("latency_histogram: done, {decode_errors} decode errors");
    println!("=== latency_histogram ===");
    println!();
    println!(
        "buckets: {BUCKETS} log-spaced from {} ns to {} ns ({:.1}× per step)",
        MIN_NS,
        MAX_NS,
        ((MAX_NS as f64 / MIN_NS as f64).powf(1.0 / BUCKETS as f64))
    );
    println!();

    print_one("phase 0 sanitize", &sanitize_ns);
    print_one("phase 1 tokenize", &tokenize_ns);
    print_one("phase 2 pair", &pair_ns);
    print_one("phase 3 classify", &classify_ns);
    print_one("lex_into_arena (total)", &full_ns);
}

fn print_one(label: &str, samples: &[u64]) {
    println!("{label} ({} docs)", samples.len());
    if samples.is_empty() {
        println!("  (no samples)");
        println!();
        return;
    }
    let h = log_histogram_ns(samples, BUCKETS, MIN_NS, MAX_NS);
    let max_count = h.iter().map(|(_, _, c)| *c).max().unwrap_or(1);
    for (lo, hi, count) in h {
        let lab = format!("{:>9} - {:<9}", fmt_ns(lo), fmt_ns(hi));
        println!("  {}", render_bar_row(&lab, count, max_count, 60));
    }
    println!();
}

fn fmt_ns(ns: u64) -> String {
    if ns < 1_000 {
        format!("{ns}ns")
    } else if ns < 1_000_000 {
        format!("{:.0}µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.1}ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2}s", ns as f64 / 1_000_000_000.0)
    }
}
