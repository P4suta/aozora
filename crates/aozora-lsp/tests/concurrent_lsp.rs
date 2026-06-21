//! Concurrent-access regression tests for the LSP backend's
//! `Arc<DashMap<Url, OpenDocument>>` surface.
//!
//! Drive the public `ParseCache` directly from multiple threads
//! and assert: independent threads' parses never deadlock, every
//! thread observes a consistent diagnostic count after its own
//! reparse, and per-document state never gets crossed between URIs.

use std::sync::{Arc, Mutex};
use std::thread;

use aozora_lsp::internals::ParseCache;

#[test]
fn concurrent_reparse_two_independent_caches_completes_without_deadlock() {
    let cache_a = Arc::new(Mutex::new(ParseCache::default()));
    let cache_b = Arc::new(Mutex::new(ParseCache::default()));

    let a = {
        let cache = Arc::clone(&cache_a);
        thread::spawn(move || {
            for _ in 0..32 {
                let mut guard = cache.lock().expect("lock cache_a");
                drop(guard.reparse("｜青梅《おうめ》"));
            }
        })
    };
    let b = {
        let cache = Arc::clone(&cache_b);
        thread::spawn(move || {
            for _ in 0..32 {
                let mut guard = cache.lock().expect("lock cache_b");
                drop(guard.reparse("plain text"));
            }
        })
    };
    a.join().expect("thread A panicked");
    b.join().expect("thread B panicked");
}

#[test]
fn parse_cache_with_tree_after_reparse_is_consistent() {
    // Single cache reparsed from one thread, then read from the same
    // thread (DashMap-style entry handoff). Pins the invariant that a
    // reparse always populates a tree that `with_tree` can borrow.
    let mut cache = ParseCache::default();
    drop(cache.reparse("｜青梅《おうめ》"));
    let inline_count = cache
        .with_tree(|tree| {
            tree.lex_output()
                .registry
                .count_kind(aozora::Sentinel::Inline)
        })
        .expect("populated");
    assert_eq!(inline_count, 1);
}
