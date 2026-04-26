//! Histogram of corpus documents by phase-1..3 diagnostic count.
//!
//! Buckets are: 0, 1-5, 6-20, 21-100, 100+. For each bucket the report
//! shows doc count, mean parse latency (`lex_into_arena`), and the
//! bucket's share of the corpus. A separate "top-5" list calls out the
//! single noisiest documents — useful when chasing a regression that
//! manifests as a sudden diagnostic spike.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example diagnostic_distribution \
//!   -p aozora-bench
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

const NS_PER_MS: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy)]
struct Sample {
    diag_count: usize,
    parse_ns: u64,
}

fn bucket_of(n: usize) -> usize {
    if n == 0 {
        0
    } else if n <= 5 {
        1
    } else if n <= 20 {
        2
    } else if n <= 100 {
        3
    } else {
        4
    }
}

const BUCKET_LABELS: [&str; 5] = ["0", "1-5", "6-20", "21-100", "100+"];

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        std::process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("diagnostic_distribution: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!(
        "diagnostic_distribution: loaded {} items, measuring…",
        items.len()
    );

    let mut samples: Vec<Sample> = Vec::with_capacity(items.len());
    let mut labels: Vec<String> = Vec::with_capacity(items.len());
    let mut decode_errors = 0usize;
    let wall_start = Instant::now();

    for (i, item) in items.iter().enumerate() {
        let Ok(text) = decode_sjis(&item.bytes) else {
            decode_errors += 1;
            continue;
        };
        let arena = Arena::new();
        let t = Instant::now();
        let out = lex_into_arena(&text, &arena);
        let parse_ns = t.elapsed().as_nanos() as u64;
        samples.push(Sample {
            diag_count: out.diagnostics.len(),
            parse_ns,
        });
        labels.push(item.label.clone());

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
        "diagnostic_distribution: done in {:.2}s, {decode_errors} decode errors",
        wall.as_secs_f64()
    );

    print_report(&samples, &labels, decode_errors);
}

fn print_report(samples: &[Sample], labels: &[String], decode_errors: usize) {
    let n = samples.len();
    println!("=== diagnostic_distribution ===");
    println!();
    println!("documents processed : {n}");
    println!("decode errors       : {decode_errors}");
    println!();

    if n == 0 {
        return;
    }

    let mut counts = [0usize; 5];
    let mut totals_ns = [0u64; 5];
    for s in samples {
        let b = bucket_of(s.diag_count);
        counts[b] += 1;
        totals_ns[b] = totals_ns[b].saturating_add(s.parse_ns);
    }

    println!(
        "{:<8} {:>10} {:>10} {:>16}",
        "diags", "docs", "% corpus", "mean parse µs"
    );
    println!("{}", "-".repeat(48));
    for i in 0..5 {
        let pct = counts[i] as f64 * 100.0 / n as f64;
        let mean_us = if counts[i] == 0 {
            0.0
        } else {
            totals_ns[i] as f64 / counts[i] as f64 / 1000.0
        };
        println!(
            "{:<8} {:>10} {:>9.2}% {:>16.2}",
            BUCKET_LABELS[i], counts[i], pct, mean_us
        );
    }
    println!();

    let mut indexed: Vec<(usize, &Sample)> = samples.iter().enumerate().collect();
    indexed.sort_by_key(|(_, s)| std::cmp::Reverse(s.diag_count));
    println!("Top-5 docs by diagnostic count");
    for (idx, s) in indexed.iter().take(5) {
        let label = labels.get(*idx).map_or("?", String::as_str);
        println!(
            "  diags {:>6}  parse {:>8.2} ms  — {label}",
            s.diag_count,
            s.parse_ns as f64 / NS_PER_MS
        );
    }
}
