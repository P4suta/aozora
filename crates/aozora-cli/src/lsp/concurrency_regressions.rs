//! Bug-pattern regression suite for the LSP concurrency surface.
//!
//! `aozora::Document` is `!Sync` (bumpalo arena interior), so the
//! shared concurrent state lives in the in-process server's
//! [`ParseCache`](super::parse_cache::ParseCache). These tests pin the
//! invariant set the cache must hold under concurrent reparses from
//! multiple threads.

use std::sync::{Arc, Mutex};
use std::thread;

use aozora::unstable::Sentinel;

use super::parse_cache::ParseCache;

/// Invariant: a `ParseCache` shared via `Arc<Mutex<_>>` between
/// threads never deadlocks across rapid concurrent reparses.
#[test]
fn shared_cache_under_lock_handles_burst_reparse_load() {
    let cache = Arc::new(Mutex::new(ParseCache::default()));
    let handles: Vec<_> = (0u32..8)
        .map(|i| {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                let payload = if i.is_multiple_of(2) {
                    "｜青梅《おうめ》".to_owned()
                } else {
                    format!("paragraph #{i}")
                };
                for _ in 0..16 {
                    let mut guard = cache.lock().expect("lock cache");
                    drop(guard.reparse(&payload));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("worker panicked");
    }
}

/// Invariant: after the final reparse the cache reflects the LAST
/// parsed text (no race-condition lost-update).
#[test]
fn last_reparse_wins_under_serialised_access() {
    let cache = Arc::new(Mutex::new(ParseCache::default()));
    let mut guard = cache.lock().expect("lock");
    drop(guard.reparse("first"));
    drop(guard.reparse("｜青梅《おうめ》"));
    drop(guard);
    let inline = {
        let guard = cache.lock().expect("lock");
        guard
            .with_tree(|tree| tree.lex_output().registry.count_kind(Sentinel::Inline))
            .expect("populated")
    };
    assert_eq!(inline, 1);
}
