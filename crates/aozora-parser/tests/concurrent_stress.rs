//! Concurrent stress test for `parse()` and the parallel parse path.
//!
//! # Purpose
//!
//! Property tests cover input shape; this file covers input *load
//! pattern*. Specifically, we hammer `parse()` from multiple OS
//! threads simultaneously to surface:
//!
//! - Panic / crash under concurrent rayon `par_iter` dispatches
//!   (e.g. global state leakage between calls).
//! - Diverging output between threads sharing the same input
//!   (would indicate a non-deterministic merge or cache bug).
//! - Allocator / rayon thread-pool contention regressions.
//!
//! Each thread independently picks an input from a deterministic
//! fixture pool (seeded by thread id) so different threads see
//! different inputs but each thread's sequence is reproducible. The
//! invariant checked is `parse(s) ≡ parse_sequential(s)` byte-for-
//! byte; failure prints the offending thread id + iteration.
//!
//! # Tunables
//!
//! - `AOZORA_STRESS_K` (default 200) — iterations per thread. Set to
//!   10000+ for nightly cron, 200 for PR loop. Each iteration is a
//!   single `parse()` call so K=10000 × 8 threads = 80 000 parses.
//! - `AOZORA_STRESS_THREADS` (default 8) — number of OS threads.

use std::env;
use std::panic;
use std::sync::Arc;
use std::thread;

use aozora_parser::parallel::{PARALLEL_THRESHOLD, parse_sequential};
use aozora_parser::parse;

/// Build a fixture pool of corpus-shaped inputs spanning the
/// dispatch threshold. Six entries:
/// - small ASCII (well below threshold)
/// - small Japanese (below)
/// - mid-size with annotations (around threshold)
/// - just-above threshold pure ASCII
/// - just-above threshold with paragraph-spanning paired container
/// - well above threshold (~2x)
fn fixture_pool() -> Vec<String> {
    let small_ascii = "the quick brown fox\n\nshort doc".to_owned();
    let small_jp = "春は曙\n\n夏は夜\n\n秋は夕暮れ".to_owned();

    let mid_ann = {
        let mut s = String::new();
        for _ in 0..200 {
            s.push_str("｜青梅《おうめ》paragraph text\n\n");
        }
        s
    };

    let just_above_ascii = {
        let mut s = String::with_capacity(PARALLEL_THRESHOLD * 2);
        while s.len() < PARALLEL_THRESHOLD + 1024 {
            s.push_str("the quick brown fox jumps over\n\n");
        }
        s
    };

    let just_above_container = {
        let mut s = String::from("［＃ここから割書］\n\n");
        while s.len() < PARALLEL_THRESHOLD + 1024 {
            s.push_str("body content paragraph\n\n");
        }
        s.push_str("［＃ここで割書終わり］\n\n");
        // Plus a few non-container paragraphs after.
        for _ in 0..50 {
            s.push_str("trailing plain prose\n\n");
        }
        s
    };

    let large_mixed = {
        let mut s = String::with_capacity(2 * PARALLEL_THRESHOLD);
        let mut i = 0;
        while s.len() < 2 * PARALLEL_THRESHOLD {
            if i % 5 == 0 {
                s.push_str("｜漢字《かんじ》");
            }
            s.push_str("paragraph text here\n\n");
            i += 1;
        }
        s
    };

    vec![
        small_ascii,
        small_jp,
        mid_ann,
        just_above_ascii,
        just_above_container,
        large_mixed,
    ]
}

fn k_iterations() -> usize {
    env::var("AOZORA_STRESS_K")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(200)
}

fn n_threads() -> usize {
    env::var("AOZORA_STRESS_THREADS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(8)
}

/// Invariant: `parse(s) == parse_sequential(s)` for every input from
/// the fixture pool, called K times in parallel from N threads with
/// no shared state. A divergence indicates either:
/// - rayon scheduling causing non-deterministic merge order
/// - thread-local state in the parser leaking across calls
/// - allocator behaviour producing observably-different output
///
/// Reproduces: preventive — primary stress shake-out for the
/// 3-layer parallel parser.
#[test]
fn parse_under_concurrent_load_matches_sequential() {
    let pool = Arc::new(fixture_pool());
    let k = k_iterations();
    let n = n_threads();

    eprintln!(
        "concurrent_stress: {n} threads × {k} iters × {} fixtures",
        pool.len()
    );

    let mut handles = Vec::with_capacity(n);
    for thread_id in 0..n {
        let pool = Arc::clone(&pool);
        handles.push(thread::spawn(move || {
            // Deterministic per-thread sequence: pick fixture by
            // (thread_id + i) % pool.len(). This guarantees every
            // thread visits every fixture, but with shifted phase
            // so threads don't all parse the same input at the
            // same instant.
            for i in 0..k {
                let fixture_idx = (thread_id + i) % pool.len();
                let input = &pool[fixture_idx];
                let par = parse(input);
                let seq = parse_sequential(input);
                assert_eq!(
                    par.artifacts.normalized, seq.artifacts.normalized,
                    "thread {thread_id} iter {i} fixture {fixture_idx}: normalized diverged",
                );
                assert_eq!(
                    par.diagnostics.len(),
                    seq.diagnostics.len(),
                    "thread {thread_id} iter {i} fixture {fixture_idx}: diag count diverged",
                );
                assert_eq!(
                    par.artifacts.registry.inline.len(),
                    seq.artifacts.registry.inline.len(),
                    "thread {thread_id} iter {i} fixture {fixture_idx}: inline registry diverged",
                );
            }
        }));
    }

    // Collect results: any panic in a worker propagates here as
    // `Err` from `join`. We re-panic with the thread id so the
    // failure message is actionable.
    for (idx, h) in handles.into_iter().enumerate() {
        if let Err(payload) = h.join() {
            panic::resume_unwind(Box::new(format!(
                "stress thread {idx} panicked: {payload:?}"
            )));
        }
    }
}

/// Invariant: every fixture's parsed output is byte-stable across
/// repeated single-threaded calls. Establishes a cheap baseline so
/// the multi-threaded test above can attribute divergence to
/// concurrency (not to fixture-level non-determinism).
/// Reproduces: preventive — sanity check on the fixture pool.
#[test]
fn fixture_pool_is_byte_stable_under_repeated_serial_parse() {
    for (idx, input) in fixture_pool().iter().enumerate() {
        let baseline = parse(input);
        for i in 0..5 {
            let r = parse(input);
            assert_eq!(
                r.artifacts.normalized, baseline.artifacts.normalized,
                "fixture {idx} iter {i}: serial-parse divergence",
            );
        }
    }
}
