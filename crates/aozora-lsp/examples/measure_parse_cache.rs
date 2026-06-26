//! One-shot wall-time measurement of the [`ParseCache`] reparse and the
//! borrowed-tree access it lends (#237 Stage B'1) — distinct from
//! `measure_incremental`, which times the tree-sitter snapshot rebuild (a
//! different subsystem).
//!
//! Builds a synthetic document of `N` blank-line-separated plain-prose
//! paragraphs and times two things:
//!
//! - a full [`ParseCache::reparse`] (the cost the editor pays per debounced
//!   keystroke under the current foundation — every reparse is a full parse);
//! - a [`ParseCache::with_tree`] call, which is now **cheap**: the owned parse
//!   output is retained and lent as a borrowed `Tree::view`, so a request
//!   handler reaches the tree without re-parsing.
//!
//! Note: under #237 Stage B'1 every reparse is a full parse, so a follow-up
//! edit is *not* faster via segment reuse (`cache_hits == 0`). Incremental
//! reuse — and a faster "warm" reparse — returns in a later #237 PR. The win
//! delivered here is the cheap per-request `with_tree`.
//!
//! Run with:
//! ```text
//! cargo run -p aozora-lsp --release --example measure_parse_cache --features internals
//! ```

use std::time::Instant;

use aozora_lsp::internals::ParseCache;

/// `n` blank-line-separated plain-prose paragraphs. Each is unique (the
/// index is woven in).
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

    // A full parse from scratch — the per-keystroke cost.
    let mut cache = ParseCache::default();
    let t = Instant::now();
    let (_d0, stats) = cache.reparse(&doc);
    let parse_us = t.elapsed().as_micros();
    println!(
        "full reparse  : {parse_us:>9} µs  (entries={}, hits={}, misses={})",
        stats.cache_entries_after, stats.cache_hits, stats.cache_misses,
    );

    // Per-request tree access — now cheap (borrows the retained output, no
    // re-parse). Time a single `with_tree` call.
    let t = Instant::now();
    let node_count = cache
        .with_tree(|tree| tree.source_nodes().len())
        .expect("populated cache lends a tree");
    let with_tree_us = t.elapsed().as_micros();
    println!(
        "with_tree     : {with_tree_us:>9} µs  (source_nodes={node_count}) — borrows retained output, no re-parse",
    );

    println!(
        "with_tree / full reparse: ~{}x cheaper",
        parse_us / with_tree_us.max(1),
    );
}
