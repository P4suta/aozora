//! Single-doc probe for the pathological annotation outlier.
//!
//! Phase breakdown shows doc #5667 (`明治人物月旦`) consumes 170 ms
//! in phase 3 classify alone — 98% of its parse wall-clock. This
//! probe loads that single file and times each phase 1000 times to
//! get a stable per-call cost, plus emits classify call counts for
//! Aho-Corasick design.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::missing_panics_doc,
    clippy::disallowed_methods,
    reason = "profiling tool, not library"
)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use aozora_encoding::decode_sjis;
use aozora_lex::lex_into_arena;
use aozora_lexer::{
    ClassifiedSpan, PairEvent, Token, classify, pair, sanitize, tokenize,
};
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;

/// Default pathological doc — kaeriten / annotation density extreme.
/// Override via the `AOZORA_PROBE_DOC` env var (relative to
/// `AOZORA_CORPUS_ROOT`).
const DEFAULT_RELATIVE_PATH: &str = "001161/files/43624_ruby_28995/43624_ruby_28995.txt";
const ITERS: u32 = 100;

fn main() {
    let Some(root) = env::var_os("AOZORA_CORPUS_ROOT") else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set; this probe needs the corpus to load doc #5667."
        );
        std::process::exit(2);
    };
    let relative = env::var("AOZORA_PROBE_DOC")
        .unwrap_or_else(|_| DEFAULT_RELATIVE_PATH.to_owned());
    let path = PathBuf::from(&root).join(&relative);
    if !path.is_file() {
        eprintln!(
            "expected pathological doc at {}; if your corpus checkout differs, override the path",
            path.display()
        );
        std::process::exit(2);
    }

    let bytes = fs::read(&path).expect("read pathological doc");
    let text = decode_sjis(&bytes).expect("decode SJIS");

    println!(
        "Loaded {}\n  bytes (UTF-8): {}\n  chars: {}\n",
        path.display(),
        text.len(),
        text.chars().count()
    );

    let mut sanitize_total = 0u64;
    let mut tokenize_total = 0u64;
    let mut pair_total = 0u64;
    let mut classify_total = 0u64;
    let mut full_total = 0u64;

    // NOTE: post-I-2 the production pipeline fuses tokenize → pair
    // → classify with no `Vec` materialisation; this probe still
    // collects each phase to a Vec for individual timing.
    for _ in 0..ITERS {
        let t = Instant::now();
        let sanitized = sanitize(&text);
        sanitize_total += t.elapsed().as_nanos() as u64;

        let t = Instant::now();
        let tokens: Vec<Token> = tokenize(&sanitized.text).collect();
        tokenize_total += t.elapsed().as_nanos() as u64;

        let t = Instant::now();
        let mut pair_stream = pair(tokens.into_iter());
        let pair_events: Vec<PairEvent> = (&mut pair_stream).collect();
        drop(pair_stream.take_diagnostics());
        pair_total += t.elapsed().as_nanos() as u64;

        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let t = Instant::now();
        let mut classify_stream = classify(pair_events.into_iter(), &sanitized.text, &mut alloc);
        let _classify_spans: Vec<ClassifiedSpan<'_>> = (&mut classify_stream).collect();
        drop(classify_stream.take_diagnostics());
        classify_total += t.elapsed().as_nanos() as u64;

        // Full pipeline run, separate arena so the per-doc cost
        // includes the post-classify ArenaNormalizer walk.
        let arena_full = Arena::new();
        let t = Instant::now();
        let _full = lex_into_arena(&text, &arena_full);
        full_total += t.elapsed().as_nanos() as u64;
    }

    let avg = |total: u64| -> f64 { total as f64 / f64::from(ITERS) / 1_000_000.0 };
    let pct = |total: u64, all: u64| -> f64 { total as f64 * 100.0 / all as f64 };

    let standalone =
        sanitize_total + tokenize_total + pair_total + classify_total;

    println!("Per-call averages over {ITERS} iterations:");
    println!("  sanitize       : {:>7.2} ms  ({:>5.1}%)", avg(sanitize_total), pct(sanitize_total, standalone));
    println!("  tokenize       : {:>7.2} ms  ({:>5.1}%)", avg(tokenize_total), pct(tokenize_total, standalone));
    println!("  pair           : {:>7.2} ms  ({:>5.1}%)", avg(pair_total), pct(pair_total, standalone));
    println!("  classify       : {:>7.2} ms  ({:>5.1}%)", avg(classify_total), pct(classify_total, standalone));
    println!("  ──────────────────────────────────────");
    println!("  4-PHASE TOTAL  : {:>7.2} ms", avg(standalone));
    println!("  lex_into_arena : {:>7.2} ms", avg(full_total));
    println!("  post-classify ∼: {:>7.2} ms", avg(full_total.saturating_sub(standalone)));

    // When `aozora-lexer/phase3-instrument` is enabled, dump the
    // per-subsystem call counts for the LAST iteration so callers can
    // see which subsystem dominates on this specific document.
    // (Cargo's cross-package feature resolution surfaces it as
    // `aozora_lexer` having the `instrumentation` module visible —
    // we test for module visibility via a `cfg(feature = ...)` on
    // a local stand-in feature defined in this crate's Cargo.toml.)
    #[cfg(feature = "instrument")]
    {
        use aozora_lexer::instrumentation::{
            PendingSizeHistogram, Subsystem, TimingTable, YieldCounters,
        };
        TimingTable::reset();
        YieldCounters::reset();
        PendingSizeHistogram::reset();
        {
            let arena_inst = Arena::new();
            drop(lex_into_arena(&text, &arena_inst));
        }
        let snap = TimingTable::snapshot();
        let yields = YieldCounters::snapshot();
        println!();
        println!("Per-subsystem (single instrumented run):");
        for sub in Subsystem::ordered() {
            let calls = snap.counts.get(&sub).copied().unwrap_or(0);
            let ns = snap.total_ns.get(&sub).copied().unwrap_or(0);
            let avg_ns = if calls == 0 { 0.0 } else { (ns as f64) / (calls as f64) };
            println!(
                "  {:<32}  {:>8} calls  {:>10.3} ms  {:>10.0} ns/call",
                sub.label(),
                calls,
                (ns as f64) / 1_000_000.0,
                avg_ns,
            );
        }
        println!();
        let total = yields.total();
        println!("Yield-kind histogram (total yields = {total}):");
        let bar = |n: u64| -> String {
            let frac = if total == 0 { 0.0 } else { (n as f64) / (total as f64) };
            let width = (frac * 30.0) as usize;
            "█".repeat(width)
        };
        println!("  Plain        {:>8}  {:>5.1}%  {}", yields.plain,      100.0 * yields.plain      as f64 / total.max(1) as f64, bar(yields.plain));
        println!("  Newline      {:>8}  {:>5.1}%  {}", yields.newline,    100.0 * yields.newline    as f64 / total.max(1) as f64, bar(yields.newline));
        println!("  Aozora       {:>8}  {:>5.1}%  {}", yields.aozora,     100.0 * yields.aozora     as f64 / total.max(1) as f64, bar(yields.aozora));
        println!("  BlockOpen    {:>8}  {:>5.1}%  {}", yields.block_open, 100.0 * yields.block_open as f64 / total.max(1) as f64, bar(yields.block_open));
        println!("  BlockClose   {:>8}  {:>5.1}%  {}", yields.block_close,100.0 * yields.block_close as f64 / total.max(1) as f64, bar(yields.block_close));

        let phist = PendingSizeHistogram::snapshot();
        let ptotal = phist.total();
        println!();
        println!("pending_outputs.len() histogram at pop_front (total = {ptotal}, max seen = {}):", phist.max_seen);
        let pbar = |n: u64| -> String {
            let frac = if ptotal == 0 { 0.0 } else { (n as f64) / (ptotal as f64) };
            let width = (frac * 30.0) as usize;
            "█".repeat(width)
        };
        let pp = |label: &str, n: u64| {
            let pct = 100.0 * n as f64 / ptotal.max(1) as f64;
            println!("  size {:>10}  {:>10}  {:>5.1}%  {}", label, n, pct, pbar(n));
        };
        pp("0 (empty)",  phist.size_0);
        pp("1",          phist.size_1);
        pp("2-4",        phist.size_2_4);
        pp("5-15",       phist.size_5_15);
        pp("16-63",      phist.size_16_63);
        pp("64-255",     phist.size_64_255);
        pp("256+",       phist.size_256_plus);

        let replay_sizes = aozora_lexer::instrumentation::snapshot_replay_sizes();
        if !replay_sizes.is_empty() {
            let mut sorted = replay_sizes.clone();
            sorted.sort_unstable();
            let total_events: u64 = sorted.iter().sum();
            println!();
            println!(
                "replay_unrecognised_body invocations: {} (total events replayed = {})",
                sorted.len(),
                total_events,
            );
            println!("  body sizes (sorted): {:?}", sorted);
        }
        aozora_lexer::instrumentation::reset_replay_sizes();
    }

    // Single high-precision parse to dump classify shape (annotation
    // count, gaiji count) for sizing the AC DFA work.
    let sanitized = sanitize(&text);
    let tokens: Vec<Token> = tokenize(&sanitized.text).collect();
    let mut pair_stream = pair(tokens.into_iter());
    let pair_events: Vec<PairEvent> = (&mut pair_stream).collect();
    drop(pair_stream.take_diagnostics());
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::new(&arena);
    let mut classify_stream = classify(pair_events.iter().cloned(), &sanitized.text, &mut alloc);
    let classify_spans: Vec<ClassifiedSpan<'_>> = (&mut classify_stream).collect();
    drop(classify_stream.take_diagnostics());
    let mut aozora_count = 0;
    let mut counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    for span in &classify_spans {
        use aozora_lexer::SpanKind;
        if let SpanKind::Aozora(node) = &span.kind {
            aozora_count += 1;
            use aozora_syntax::borrowed::AozoraNode;
            let name = match node {
                AozoraNode::Ruby(_) => "Ruby",
                AozoraNode::Bouten(_) => "Bouten",
                AozoraNode::TateChuYoko(_) => "TateChuYoko",
                AozoraNode::Gaiji(_) => "Gaiji",
                AozoraNode::Indent(_) => "Indent",
                AozoraNode::AlignEnd(_) => "AlignEnd",
                AozoraNode::Warichu(_) => "Warichu",
                AozoraNode::Keigakomi(_) => "Keigakomi",
                AozoraNode::PageBreak => "PageBreak",
                AozoraNode::SectionBreak(_) => "SectionBreak",
                AozoraNode::AozoraHeading(_) => "AozoraHeading",
                AozoraNode::HeadingHint(_) => "HeadingHint",
                AozoraNode::Sashie(_) => "Sashie",
                AozoraNode::Kaeriten(_) => "Kaeriten",
                AozoraNode::Annotation(_) => "Annotation",
                AozoraNode::DoubleRuby(_) => "DoubleRuby",
                AozoraNode::Container(_) => "Container",
                _ => "_unknown",
            };
            *counts.entry(name).or_insert(0) += 1;
        }
    }
    println!();
    println!("Classify output shape:");
    println!("  Aozora nodes total : {aozora_count}");
    let mut entries: Vec<(&str, usize)> = counts.into_iter().collect();
    entries.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (k, n) in entries {
        println!("  {k:<14} : {n}");
    }
    println!("  classify ms / Aozora node : {:.3} µs",
        avg(classify_total) * 1000.0 / aozora_count as f64);

    // Count event-stream features that would help the AC analysis.
    let mut bracket_pair_count = 0;
    let mut quote_open_count = 0;
    for ev in &pair_events {
        match ev {
            PairEvent::PairOpen { kind, .. } => match kind {
                aozora_lexer::PairKind::Bracket => {
                    bracket_pair_count += 1;
                }
                aozora_lexer::PairKind::Quote => quote_open_count += 1,
                _ => {}
            },
            _ => {}
        }
    }
    println!();
    println!("Pair-event shape:");
    println!("  Bracket pairs (［…］) : {bracket_pair_count}");
    println!("  Quote pairs (「…」)   : {quote_open_count}");
}
