//! Per-class deforestation gap.
//!
//! Times each corpus document twice:
//!
//! - **fused** — `lex_into_arena`, the production I-2 fused chain.
//! - **materialized** — sanitize → `tokenize().collect()` →
//!   `pair().collect()` → `classify().collect()`. Same call sequence
//!   as `phase_breakdown`, but invoked here so the materialised cost
//!   is paid alongside the fused cost on the same input.
//!
//! For each size band reports the relative gap
//! `(mat - fused) / fused`, quantifying the L1/L2-cache benefit of
//! the iterator-fusion deforestation across input sizes.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example fused_vs_materialized -p aozora-bench
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
    reason = "profiling-example tool, not library code; .collect() in time_materialized is the very thing being measured"
)]

use std::env;
use std::process;
use std::time::Instant;

use aozora_bench::{SizeBand, corpus_size_bands};
use aozora_corpus::CorpusItem;
use aozora_pipeline::lex_into_arena;
use aozora_pipeline::lexer::{
    ClassifiedSpan, PairEvent, Token, classify, pair, sanitize, tokenize,
};
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;

#[derive(Debug, Default)]
struct BandStats {
    fused_ns_total: u128,
    mat_ns_total: u128,
    docs: usize,
    /// Per-doc gap ratios; we keep them so the report can emit p50.
    gaps: Vec<f64>,
}

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        process::exit(2);
    };
    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("fused_vs_materialized: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    let banded = corpus_size_bands(items);
    eprintln!(
        "fused_vs_materialized: bucketed (small={}, medium={}, large={}, path={}, decode_err={})",
        banded.small.len(),
        banded.medium.len(),
        banded.large.len(),
        banded.pathological.len(),
        banded.decode_errors,
    );

    let mut stats: [BandStats; 4] = Default::default();

    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        for (_, text) in banded.band(band) {
            let fused = time_fused(text);
            let mat = time_materialized(text);
            stats[slot].fused_ns_total += u128::from(fused);
            stats[slot].mat_ns_total += u128::from(mat);
            stats[slot].docs += 1;
            if fused > 0 {
                stats[slot]
                    .gaps
                    .push((mat as f64 - fused as f64) / fused as f64);
            }
        }
    }

    print_report(&stats);
}

fn time_fused(text: &str) -> u64 {
    let arena = Arena::new();
    let t = Instant::now();
    drop(lex_into_arena(text, &arena));
    t.elapsed().as_nanos() as u64
}

fn time_materialized(text: &str) -> u64 {
    let t = Instant::now();
    let sanitized = sanitize(text);
    let tokens: Vec<Token> = tokenize(&sanitized.text).collect();
    let mut ps = pair(tokens.into_iter());
    let pe: Vec<PairEvent> = (&mut ps).collect();
    drop(ps.take_diagnostics());
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::new(&arena);
    let mut cs = classify(pe, &sanitized.text, &mut alloc);
    let _spans: Vec<ClassifiedSpan<'_>> = (&mut cs).collect();
    drop(cs.take_diagnostics());
    t.elapsed().as_nanos() as u64
}

fn print_report(stats: &[BandStats; 4]) {
    println!("=== fused_vs_materialized ===");
    println!();
    println!(
        "{:<13} {:>6} {:>14} {:>14} {:>11} {:>11}",
        "band", "docs", "fused total ms", "mat total ms", "agg gap %", "p50 gap %"
    );
    println!("{}", "-".repeat(76));
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let s = &stats[slot];
        if s.docs == 0 {
            println!(
                "{:<13} {:>6} {:>14} {:>14} {:>11} {:>11}",
                band.label(),
                0,
                "-",
                "-",
                "-",
                "-",
            );
            continue;
        }
        let fused_ms = s.fused_ns_total as f64 / 1_000_000.0;
        let mat_ms = s.mat_ns_total as f64 / 1_000_000.0;
        let agg_gap_pct = if s.fused_ns_total == 0 {
            0.0
        } else {
            (s.mat_ns_total as f64 - s.fused_ns_total as f64) / s.fused_ns_total as f64 * 100.0
        };
        let p50_gap_pct = {
            let mut g = s.gaps.clone();
            g.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
            let idx = g.len() / 2;
            g.get(idx).copied().unwrap_or(0.0) * 100.0
        };
        println!(
            "{:<13} {:>6} {:>14.2} {:>14.2} {:>10.1}% {:>10.1}%",
            band.label(),
            s.docs,
            fused_ms,
            mat_ms,
            agg_gap_pct,
            p50_gap_pct,
        );
    }
    println!();
    println!("Positive gap → materialised pipeline is slower than fused (deforestation win).");
}
