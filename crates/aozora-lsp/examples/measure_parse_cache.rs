//! One-shot wall-time measurement of the **segment-cache diagnostics**
//! reparse (#237 A3) — distinct from `measure_incremental`, which times
//! the tree-sitter snapshot rebuild (a different subsystem).
//!
//! Builds a synthetic document of `N` blank-line-separated plain-prose
//! paragraphs. That is the shape that actually exercises segment reuse:
//! each paragraph is its own segment and there are no
//! whole-document-scoped diagnostics, so a plain interior edit takes the
//! incremental fast path. (A single-`\n`-joined or gaiji-heavy document
//! collapses to one segment / forces a full re-parse, masking the win.)
//!
//! It then compares two reparses of the same `ParseCache`:
//!
//! - cold — [`ParseCache::reparse`]: a full parse, every segment re-lexed.
//! - warm — [`ParseCache::reparse_incremental`] after a single plain-char
//!   edit inside one middle paragraph: only that segment re-lexes; the
//!   other `N - 1` are reused.
//!
//! Run with:
//! ```text
//! cargo run -p aozora-lsp --release --example measure_parse_cache --features internals
//! ```

use std::time::Instant;

use aozora_lsp::internals::{ByteEdit, ParseCache};

/// `n` blank-line-separated plain-prose paragraphs. Each is unique (the
/// index is woven in) so an edit in the middle one is unambiguous.
fn plain_paragraphs(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        if i > 0 {
            s.push_str("\n\n");
        }
        s.push('第');
        s.push_str(&i.to_string());
        s.push_str("段落、ここに本文がそれなりの長さで続いていきます。");
    }
    s
}

fn main() {
    let n = 20_000usize;
    let doc = plain_paragraphs(n);
    println!("synthetic doc: {n} paragraphs, {} bytes", doc.len());

    // Cold: a full parse from scratch.
    let mut cache = ParseCache::default();
    let t = Instant::now();
    let (_d0, cold) = cache.reparse(&doc);
    let cold_us = t.elapsed().as_micros();
    println!(
        "cold  full reparse : {cold_us:>9} µs  (segments={}, hits={}, misses={})",
        cold.cache_entries_after, cold.cache_hits, cold.cache_misses,
    );

    // Warm: a single plain-char edit inside the middle paragraph.
    let marker = format!("第{}段落", n / 2);
    let at = doc.find(&marker).expect("middle paragraph present") + marker.len();
    let mut edited = doc.clone();
    edited.insert(at, 'ぞ');
    let edit = ByteEdit::new(at..at, "ぞ".to_owned());

    let t = Instant::now();
    let (diags, warm) = cache.reparse_incremental(&edited, &[edit]);
    let warm_us = t.elapsed().as_micros();
    println!(
        "warm  incremental  : {warm_us:>9} µs  (segments={}, hits={}, misses={})",
        warm.cache_entries_after, warm.cache_hits, warm.cache_misses,
    );

    // The incremental result must equal a from-scratch parse.
    let mut fresh = ParseCache::default();
    let (want, _) = fresh.reparse(&edited);
    let as_debug =
        |ds: &[aozora::Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
    assert_eq!(
        as_debug(&diags),
        as_debug(&want),
        "incremental diagnostics must equal a full parse",
    );
    assert_eq!(
        warm.cache_hits,
        u64::try_from(n - 1).expect("paragraph count fits u64"),
        "every untouched segment reused",
    );

    println!("speedup (cold / warm): ~{}x", cold_us / warm_us.max(1));
}
