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
use aozora_lexer::{classify, pair, sanitize, tokenize};
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;

const RELATIVE_PATH: &str = "001161/files/43624_ruby_28995/43624_ruby_28995.txt";
const ITERS: u32 = 100;

fn main() {
    let Some(root) = env::var_os("AOZORA_CORPUS_ROOT") else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set; this probe needs the corpus to load doc #5667."
        );
        std::process::exit(2);
    };
    let path = PathBuf::from(&root).join(RELATIVE_PATH);
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

    for _ in 0..ITERS {
        let t = Instant::now();
        let sanitized = sanitize(&text);
        sanitize_total += t.elapsed().as_nanos() as u64;

        let t = Instant::now();
        let tokens = tokenize(&sanitized.text);
        tokenize_total += t.elapsed().as_nanos() as u64;

        let t = Instant::now();
        let pair_out = pair(&tokens);
        pair_total += t.elapsed().as_nanos() as u64;

        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let t = Instant::now();
        let _classify_out = classify(&pair_out, &sanitized.text, &mut alloc);
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

    // Single high-precision parse to dump classify shape (annotation
    // count, gaiji count) for sizing the AC DFA work.
    let sanitized = sanitize(&text);
    let tokens = tokenize(&sanitized.text);
    let pair_out = pair(&tokens);
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::new(&arena);
    let classify_out = classify(&pair_out, &sanitized.text, &mut alloc);
    let mut aozora_count = 0;
    let mut counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    for span in &classify_out.spans {
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
    let mut bracket_open_count = 0;
    let mut bracket_pair_count = 0;
    let mut quote_open_count = 0;
    use aozora_lexer::PairEvent;
    for ev in &pair_out.events {
        match ev {
            PairEvent::PairOpen { kind, .. } => match kind {
                aozora_lexer::PairKind::Bracket => {
                    bracket_open_count += 1;
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
    let _ = bracket_open_count;
}
