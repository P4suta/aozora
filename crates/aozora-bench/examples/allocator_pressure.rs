//! Per-document allocator-pressure and intern dedup distribution.
//!
//! For each corpus document this probe measures:
//!
//! - **Arena bytes per source byte** — `Arena::allocated_bytes()`
//!   sampled before and after `lex_into_arena`, divided by the input
//!   UTF-8 byte length. The histogram reveals the allocator amplification
//!   factor for typical vs tail-heavy documents.
//! - **Per-document intern dedup ratio** —
//!   `(cache_hits + table_hits) / calls` from the `BorrowedLexOutput`'s
//!   `intern_stats`. Histogrammed across the corpus.
//!
//! Together these surface allocator regressions that aggregate dedup
//! and aggregate arena byte counts hide.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example allocator_pressure -p aozora-bench
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

use aozora_corpus::CorpusItem;
use aozora_encoding::decode_sjis;
use aozora_lex::lex_into_arena;
use aozora_syntax::borrowed::Arena;

#[derive(Debug, Clone, Copy)]
struct DocSample {
    source_bytes: u64,
    arena_bytes_delta: u64,
    intern_calls: u64,
    intern_reuses: u64,
}

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        std::process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("allocator_pressure: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!(
        "allocator_pressure: loaded {} items, measuring…",
        items.len()
    );

    let mut samples: Vec<DocSample> = Vec::with_capacity(items.len());
    let mut decode_errors: u64 = 0;
    let wall_start = Instant::now();

    for (i, item) in items.iter().enumerate() {
        let Ok(text) = decode_sjis(&item.bytes) else {
            decode_errors += 1;
            continue;
        };
        // Fresh arena: `allocated_bytes()` after lex is the per-doc
        // arena footprint. We do NOT skip docs whose source is empty —
        // they're rare and a zero-length source is a legitimate edge.
        let arena = Arena::new();
        let before = arena.allocated_bytes() as u64;
        let out = lex_into_arena(&text, &arena);
        let after = arena.allocated_bytes() as u64;
        let arena_delta = after.saturating_sub(before);
        let reuses = out.intern_stats.cache_hits + out.intern_stats.table_hits;
        samples.push(DocSample {
            source_bytes: text.len() as u64,
            arena_bytes_delta: arena_delta,
            intern_calls: out.intern_stats.calls,
            intern_reuses: reuses,
        });

        if (i + 1).is_multiple_of(2_000) {
            eprintln!(
                "  …processed {} docs ({:.1}s elapsed)",
                i + 1,
                wall_start.elapsed().as_secs_f64()
            );
        }
    }

    let wall = wall_start.elapsed();
    eprintln!(
        "allocator_pressure: done in {:.2}s, {decode_errors} decode errors",
        wall.as_secs_f64()
    );

    print_report(&samples, decode_errors);
}

fn print_report(samples: &[DocSample], decode_errors: u64) {
    println!("=== allocator_pressure ===");
    println!();
    println!("documents processed : {}", samples.len());
    println!("decode errors       : {decode_errors}");
    println!();
    if samples.is_empty() {
        return;
    }

    // --- Arena bytes / source byte distribution ---
    let mut ratios: Vec<f64> = samples
        .iter()
        .filter(|s| s.source_bytes > 0)
        .map(|s| s.arena_bytes_delta as f64 / s.source_bytes as f64)
        .collect();
    ratios.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let q = |p: f64| -> f64 {
        if ratios.is_empty() {
            0.0
        } else {
            let idx = ((ratios.len() as f64 - 1.0) * p).round() as usize;
            ratios[idx.min(ratios.len() - 1)]
        }
    };
    let mean: f64 = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;

    println!("arena_bytes / source_byte (per doc)");
    println!(
        "  p50 {:>5.2}  p90 {:>5.2}  p99 {:>5.2}  max {:>5.2}  mean {:>5.2}",
        q(0.5),
        q(0.9),
        q(0.99),
        q(1.0),
        mean,
    );
    let buckets: [(&str, f64, f64); 6] = [
        ("0-1×", 0.0, 1.0),
        ("1-2×", 1.0, 2.0),
        ("2-4×", 2.0, 4.0),
        ("4-8×", 4.0, 8.0),
        ("8-16×", 8.0, 16.0),
        (">16×", 16.0, f64::INFINITY),
    ];
    let mut counts = [0usize; 6];
    for r in &ratios {
        for (i, (_, lo, hi)) in buckets.iter().enumerate() {
            if *r >= *lo && *r < *hi {
                counts[i] += 1;
                break;
            }
        }
    }
    let max_count = counts.iter().copied().max().unwrap_or(1);
    println!();
    println!("histogram (bucket -> docs)");
    for (i, (label, _, _)) in buckets.iter().enumerate() {
        println!(
            "  {label:<6} {} {} ({:.1}%)",
            bar(counts[i], max_count, 30),
            counts[i],
            counts[i] as f64 * 100.0 / ratios.len() as f64
        );
    }
    println!();

    // --- Intern dedup ratio distribution ---
    let mut dedup: Vec<f64> = samples
        .iter()
        .filter(|s| s.intern_calls > 0)
        .map(|s| s.intern_reuses as f64 / s.intern_calls as f64)
        .collect();
    dedup.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let dq = |p: f64| -> f64 {
        if dedup.is_empty() {
            0.0
        } else {
            let idx = ((dedup.len() as f64 - 1.0) * p).round() as usize;
            dedup[idx.min(dedup.len() - 1)]
        }
    };
    let dmean: f64 = dedup.iter().sum::<f64>() / dedup.len().max(1) as f64;
    println!("intern dedup ratio (per doc; (cache+table)/calls)");
    println!(
        "  p50 {:>5.3}  p90 {:>5.3}  p99 {:>5.3}  max {:>5.3}  mean {:>5.3}",
        dq(0.5),
        dq(0.9),
        dq(0.99),
        dq(1.0),
        dmean,
    );
    let dbuckets: [(&str, f64, f64); 6] = [
        ("0.00-0.20", 0.00, 0.20),
        ("0.20-0.40", 0.20, 0.40),
        ("0.40-0.60", 0.40, 0.60),
        ("0.60-0.80", 0.60, 0.80),
        ("0.80-0.95", 0.80, 0.95),
        ("0.95-1.00", 0.95, 1.0001),
    ];
    let mut dcounts = [0usize; 6];
    for r in &dedup {
        for (i, (_, lo, hi)) in dbuckets.iter().enumerate() {
            if *r >= *lo && *r < *hi {
                dcounts[i] += 1;
                break;
            }
        }
    }
    let dmax_count = dcounts.iter().copied().max().unwrap_or(1);
    println!();
    println!("histogram (bucket -> docs)");
    for (i, (label, _, _)) in dbuckets.iter().enumerate() {
        println!(
            "  {label:<11} {} {} ({:.1}%)",
            bar(dcounts[i], dmax_count, 30),
            dcounts[i],
            dcounts[i] as f64 * 100.0 / dedup.len().max(1) as f64
        );
    }
}

fn bar(count: usize, max_count: usize, width: usize) -> String {
    if max_count == 0 {
        return " ".repeat(width);
    }
    let n = (count as f64 / max_count as f64 * width as f64).round() as usize;
    let n = n.min(width);
    let mut s = "█".repeat(n);
    s.push_str(&" ".repeat(width.saturating_sub(n)));
    s
}
