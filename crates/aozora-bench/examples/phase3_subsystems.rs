//! Phase 3 sub-system per-recogniser breakdown.
//!
//! Requires the `aozora-pipeline/phase3-instrument` feature (enforced via
//! `required-features` in `aozora-bench/Cargo.toml`). When the feature
//! is on, each recogniser entry inside [`aozora_pipeline::lexer::phase3_classify`]
//! emits an [`instrumentation::SubsystemGuard`] that records elapsed
//! nanoseconds into a thread-local table. This probe drains the table
//! per document, accumulates totals across the corpus, and reports
//! per-subsystem call count + total ns + percentage of phase 3 + average
//! call duration.
//!
//! Instrumented sub-systems (9 total):
//!
//! Recogniser leaves (do not nest):
//!   - `recognize_ruby`
//!   - `recognize_annotation`
//!   - `recognize_gaiji`
//!   - `build_content_from_body`
//!   - `body_dispatcher` (Aho-Corasick lookup inside annotation body)
//!
//! Framework / dispatch (their elapsed time INCLUDES nested leaf
//! time; subtract leaf sum to get pure framework cost):
//!   - `iter_dispatch` (outer)            — `ClassifyStream::next()` body
//!   - `forward_target_is_preceded`       — per-call source scan / AC lookup
//!   - `install_forward_target_index`     — one-time per-doc AC pre-pass
//!   - `append_to_frame`                  — per-event frame buffer push
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example phase3_subsystems \
//!   -p aozora-bench --features 'aozora-pipeline/phase3-instrument'
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

use std::collections::HashMap;
use std::env;
use std::process;
use std::time::Instant;

use aozora_corpus::CorpusItem;
use aozora_encoding::decode_sjis;
use aozora_pipeline::lex_into_arena;
use aozora_pipeline::lexer::instrumentation::{Subsystem, TimingTable};
use aozora_syntax::borrowed::Arena;

const NS_PER_MS: f64 = 1_000_000.0;

#[derive(Debug, Default, Clone)]
struct Aggregate {
    counts: HashMap<Subsystem, u64>,
    total_ns: HashMap<Subsystem, u64>,
}

impl Aggregate {
    fn merge(&mut self, snap: &TimingTable) {
        for (k, v) in &snap.counts {
            *self.counts.entry(*k).or_insert(0) += v;
        }
        for (k, v) in &snap.total_ns {
            *self.total_ns.entry(*k).or_insert(0) += v;
        }
    }
    fn total_ns_all(&self) -> u64 {
        self.total_ns.values().sum()
    }
}

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("phase3_subsystems: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!(
        "phase3_subsystems: loaded {} items, measuring…",
        items.len()
    );

    let mut agg = Aggregate::default();
    let mut decode_errors: u64 = 0;
    let mut docs_processed: u64 = 0;
    let wall_start = Instant::now();

    for (i, item) in items.iter().enumerate() {
        let Ok(text) = decode_sjis(&item.bytes) else {
            decode_errors += 1;
            continue;
        };
        // Reset before each doc so the snapshot reflects only this
        // doc's recogniser activity. Run lex with a fresh arena.
        TimingTable::reset();
        let arena = Arena::new();
        let _out = lex_into_arena(&text, &arena);
        let snap = TimingTable::snapshot();
        agg.merge(&snap);
        docs_processed += 1;

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
        "phase3_subsystems: done in {:.2}s, {decode_errors} decode errors",
        wall.as_secs_f64()
    );

    print_report(&agg, docs_processed, decode_errors);
}

fn print_report(agg: &Aggregate, docs: u64, decode_errors: u64) {
    println!("=== phase3 subsystems breakdown ===");
    println!();
    println!("documents processed : {docs}");
    println!("decode errors       : {decode_errors}");
    println!(
        "instrumented variants: {} (5 recogniser leaves + 4 framework)",
        Subsystem::ordered().len()
    );
    println!();
    if agg.total_ns_all() == 0 {
        println!("no recogniser activity observed — corpus may be empty or all-plain.");
        return;
    }

    // Compute leaf total (ground truth recogniser work) and the
    // outer dispatch total (== iterator wall) separately so the
    // % column is meaningful.
    let leaf_ns: u64 = Subsystem::ordered()
        .iter()
        .filter(|s| s.is_leaf())
        .map(|s| agg.total_ns.get(s).copied().unwrap_or(0))
        .sum();
    let iter_ns = agg
        .total_ns
        .get(&Subsystem::IterDispatch)
        .copied()
        .unwrap_or(0);
    // Pure dispatch overhead = outer wall - leaves - non-leaf
    // framework subsystems that DON'T nest inside leaves
    // (forward_target_check / forward_index_install run inside
    // recogniser leaves so they ARE double-counted; append_to_frame
    // runs outside leaves so it's NOT double-counted).
    let frame_append_ns = agg
        .total_ns
        .get(&Subsystem::FrameAppend)
        .copied()
        .unwrap_or(0);
    let pure_dispatch_ns = iter_ns
        .saturating_sub(leaf_ns)
        .saturating_sub(frame_append_ns);

    println!(
        "{:<32} {:>12} {:>14} {:>14} {:>14}",
        "subsystem", "calls", "total ms", "% of iter", "avg µs/call"
    );
    println!("{}", "-".repeat(90));
    for sub in Subsystem::ordered() {
        let calls = agg.counts.get(&sub).copied().unwrap_or(0);
        let ns = agg.total_ns.get(&sub).copied().unwrap_or(0);
        let pct = if iter_ns > 0 {
            (ns as f64) * 100.0 / (iter_ns as f64)
        } else {
            0.0
        };
        let avg_us = if calls == 0 {
            0.0
        } else {
            (ns as f64) / (calls as f64) / 1000.0
        };
        println!(
            "{:<32} {:>12} {:>14.2} {:>13.2}% {:>14.3}",
            sub.label(),
            calls,
            (ns as f64) / NS_PER_MS,
            pct,
            avg_us,
        );
    }
    println!();
    println!(
        "iter_dispatch (outer wall)        : {:.2} ms",
        (iter_ns as f64) / NS_PER_MS
    );
    println!(
        "  recogniser-leaf total            : {:.2} ms ({:.1}%)",
        (leaf_ns as f64) / NS_PER_MS,
        if iter_ns > 0 {
            (leaf_ns as f64) * 100.0 / (iter_ns as f64)
        } else {
            0.0
        }
    );
    println!(
        "  append_to_frame (per-event push) : {:.2} ms ({:.1}%)",
        (frame_append_ns as f64) / NS_PER_MS,
        if iter_ns > 0 {
            (frame_append_ns as f64) * 100.0 / (iter_ns as f64)
        } else {
            0.0
        }
    );
    println!("  pure dispatch overhead (== outer - leaves - frame_append):");
    println!(
        "    {:.2} ms ({:.1}%) — iterator next() match-arm + Driver state machine",
        (pure_dispatch_ns as f64) / NS_PER_MS,
        if iter_ns > 0 {
            (pure_dispatch_ns as f64) * 100.0 / (iter_ns as f64)
        } else {
            0.0
        }
    );
}
