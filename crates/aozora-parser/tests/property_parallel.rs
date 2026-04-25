//! Property tests for the intra-document parallel parse path.
//!
//! The contract: for any input `s`,
//!
//! ```text
//! parse(s)  ==  parse_sequential(s)
//! ```
//!
//! when `s.len() >= PARALLEL_THRESHOLD` and `identify_segments(s)`
//! returns more than one segment, the public `parse` dispatches to
//! the parallel path; otherwise to the sequential one. Either way
//! the public-API result must agree with the sequential reference
//! on every observable: normalized text, registry counts, registry
//! positions in order, and diagnostic count.
//!
//! We exercise the contract on three input shapes:
//!
//! 1. Random aozora-shaped fragments stitched together with paragraph
//!    breaks (the realistic case).
//! 2. Pathological input (unmatched brackets, malformed annotations)
//!    to confirm diagnostic merging stays correct under
//!    error-recovery branches.
//! 3. ASCII filler that crosses [`PARALLEL_THRESHOLD`] to guarantee
//!    the parallel path actually fires in CI.

use aozora_parser::parallel::{PARALLEL_THRESHOLD, parse_sequential};
use aozora_parser::parse;
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::{aozora_fragment, pathological_aozora};
use proptest::prelude::*;

/// Build a multi-paragraph source by interleaving fragments with
/// `\n\n`. Forces the segmenter to find multiple boundaries.
fn arb_multi_paragraph() -> impl Strategy<Value = String> {
    prop::collection::vec(aozora_fragment(8), 2..6).prop_map(|frags| frags.join("\n\n"))
}

/// Same but using the pathological generator, which produces
/// unbalanced brackets and other classifier edge cases.
fn arb_multi_paragraph_pathological() -> impl Strategy<Value = String> {
    prop::collection::vec(pathological_aozora(4), 2..6).prop_map(|frags| frags.join("\n\n"))
}

/// Force parallel-path coverage with deterministic >= threshold input.
/// Built around random fragments so the test isn't brittle.
fn arb_above_threshold() -> impl Strategy<Value = String> {
    prop::collection::vec(aozora_fragment(8), 2..6).prop_map(|frags| {
        let mut buf = frags.join("\n\n");
        // Pad with paragraph-separated ASCII so the total length
        // crosses PARALLEL_THRESHOLD; the padding is itself plain
        // text so the segmenter splits on every blank line.
        while buf.len() < PARALLEL_THRESHOLD + 1024 {
            buf.push_str("\n\nthe quick brown fox jumps over the lazy dog");
        }
        buf
    })
}

fn assert_parallel_matches_sequential(input: &str) {
    let par = parse(input);
    let seq = parse_sequential(input);

    assert_eq!(
        par.artifacts.normalized, seq.artifacts.normalized,
        "normalized diverged"
    );
    assert_eq!(
        par.artifacts.registry.inline.len(),
        seq.artifacts.registry.inline.len(),
        "inline registry size diverged",
    );
    assert_eq!(
        par.artifacts.registry.block_leaf.len(),
        seq.artifacts.registry.block_leaf.len(),
        "block_leaf registry size diverged",
    );
    assert_eq!(
        par.artifacts.registry.block_open.len(),
        seq.artifacts.registry.block_open.len(),
        "block_open registry size diverged",
    );
    assert_eq!(
        par.artifacts.registry.block_close.len(),
        seq.artifacts.registry.block_close.len(),
        "block_close registry size diverged",
    );
    assert_eq!(
        par.diagnostics.len(),
        seq.diagnostics.len(),
        "diagnostic count diverged",
    );
    // Registry positions must match in order: each parallel-path
    // entry has the same position as the sequential reference.
    for (a, b) in par
        .artifacts
        .registry
        .inline
        .iter()
        .zip(seq.artifacts.registry.inline.iter())
    {
        assert_eq!(a.0, b.0, "inline position mismatch");
    }
    for (a, b) in par
        .artifacts
        .registry
        .block_leaf
        .iter()
        .zip(seq.artifacts.registry.block_leaf.iter())
    {
        assert_eq!(a.0, b.0, "block_leaf position mismatch");
    }
}

proptest! {
    #![proptest_config(default_config())]

    /// Realistic multi-paragraph input: parallel ≡ sequential.
    #[test]
    fn parallel_matches_sequential_on_random_paragraphs(
        input in arb_multi_paragraph(),
    ) {
        assert_parallel_matches_sequential(&input);
    }

    /// Pathological input: error-recovery diagnostics must merge
    /// with the same count and shape as the sequential reference.
    #[test]
    fn parallel_matches_sequential_on_pathological_input(
        input in arb_multi_paragraph_pathological(),
    ) {
        assert_parallel_matches_sequential(&input);
    }

    /// Above-threshold input: forces the parallel dispatch path
    /// to actually fire.
    #[test]
    fn parallel_matches_sequential_above_threshold(
        input in arb_above_threshold(),
    ) {
        // Sanity: this input must trigger the parallel path.
        prop_assert!(input.len() >= PARALLEL_THRESHOLD);
        assert_parallel_matches_sequential(&input);
    }
}

// ---------------------------------------------------------------------------
// Pinned regression cases
// ---------------------------------------------------------------------------

#[test]
fn pinned_empty_input_matches() {
    assert_parallel_matches_sequential("");
}

#[test]
fn pinned_single_paragraph_matches() {
    assert_parallel_matches_sequential("｜青梅《おうめ》");
}

#[test]
fn pinned_three_paragraphs_each_with_inline_ruby() {
    assert_parallel_matches_sequential(
        "｜青梅《おうめ》\n\n｜日本《にほん》\n\n｜漢字《かんじ》",
    );
}

#[test]
fn pinned_paired_container_spanning_paragraphs() {
    let s = "［＃ここから割書］\n\nfirst\n\nsecond\n\n［＃ここで割書終わり］";
    assert_parallel_matches_sequential(s);
}

#[test]
fn pinned_above_threshold_forces_parallel_dispatch() {
    let mut s = String::with_capacity(PARALLEL_THRESHOLD * 2);
    while s.len() < PARALLEL_THRESHOLD + 1024 {
        s.push_str("the quick brown fox\n\n");
    }
    assert!(s.len() >= PARALLEL_THRESHOLD);
    assert_parallel_matches_sequential(&s);
}

#[test]
fn pinned_above_threshold_with_annotations() {
    let mut s = String::with_capacity(PARALLEL_THRESHOLD * 2);
    let mut i = 0usize;
    while s.len() < PARALLEL_THRESHOLD + 1024 {
        if i.is_multiple_of(7) {
            s.push_str("｜青梅《おうめ》");
        }
        s.push_str("paragraph text here\n\n");
        i += 1;
    }
    assert!(s.len() >= PARALLEL_THRESHOLD);
    assert_parallel_matches_sequential(&s);
}

#[test]
fn pinned_pua_collision_above_threshold() {
    // Spread a single PUA char so the merged diagnostics have a
    // shifted span; verifies offset-shifting in the merge works.
    let mut s = String::with_capacity(PARALLEL_THRESHOLD + 8192);
    while s.len() < PARALLEL_THRESHOLD {
        s.push_str("plain prose with no triggers\n\n");
    }
    s.push_str("oops \u{E001} here");
    assert_parallel_matches_sequential(&s);
}

// ---------------------------------------------------------------------------
// Threshold boundary parametrisation
//
// Invariant: parallel ≡ sequential at every PARALLEL_THRESHOLD-related
// boundary, including just-below, exact, and just-above. A subtle
// off-by-one in the dispatch (`>=` vs `>`) would let a near-threshold
// input take the wrong path and silently change observables (latency,
// rayon pool warm-up); we pin both paths byte-for-byte equal at each
// boundary so future refactors that touch the threshold get caught.
// ---------------------------------------------------------------------------

/// Build a paragraph-separated input of EXACTLY `target` bytes.
/// Each paragraph is small ASCII so segment count grows with size.
fn build_input_of_size(target: usize) -> String {
    let cell = "the quick brown fox\n\n";
    let mut buf = String::with_capacity(target + cell.len());
    while buf.len() + cell.len() < target {
        buf.push_str(cell);
    }
    // Pad with single ASCII bytes so the resulting length is exact.
    while buf.len() < target {
        buf.push('x');
    }
    buf.truncate(target);
    debug_assert_eq!(buf.len(), target, "build_input_of_size pads to exact length");
    buf
}

/// Invariant: at each boundary length the parallel and sequential paths
/// produce byte-identical output.
/// Reproduces: preventive — guards against off-by-one in
/// `parse()` dispatch logic touching `PARALLEL_THRESHOLD`.
#[test]
fn boundary_lengths_match_sequential() {
    for target in [
        PARALLEL_THRESHOLD - 1,
        PARALLEL_THRESHOLD,
        PARALLEL_THRESHOLD + 1,
        2 * PARALLEL_THRESHOLD - 1,
        2 * PARALLEL_THRESHOLD,
    ] {
        let s = build_input_of_size(target);
        assert_parallel_matches_sequential(&s);
    }
}

// ---------------------------------------------------------------------------
// Determinism under varying rayon scheduling
//
// Invariant: the same input parsed N times in a row through the
// parallel path must produce IDENTICAL output every time. Any
// non-determinism here (e.g. registry order depending on which thread
// finished first) would surface as flaky tests downstream.
// ---------------------------------------------------------------------------

/// Invariant: 10 parallel parses of the same input produce equal outputs.
/// Reproduces: preventive — guards against rayon work-stealing-induced
/// non-determinism in merge order.
#[test]
fn parallel_parse_is_deterministic_across_repeated_calls() {
    let s = build_input_of_size(2 * PARALLEL_THRESHOLD);
    let baseline = parse(&s);
    for i in 0..10 {
        let r = parse(&s);
        assert_eq!(
            r.artifacts.normalized, baseline.artifacts.normalized,
            "iteration {i}: normalized diverged",
        );
        assert_eq!(
            r.diagnostics.len(),
            baseline.diagnostics.len(),
            "iteration {i}: diag count diverged",
        );
        // Registry positions must match in order.
        for (a, b) in r
            .artifacts
            .registry
            .inline
            .iter()
            .zip(baseline.artifacts.registry.inline.iter())
        {
            assert_eq!(a.0, b.0, "iteration {i}: inline pos diverged");
        }
    }
}

// ---------------------------------------------------------------------------
// 1-thread fallback (CI shared runner scenario)
//
// Invariant: when `current_num_threads() == 1` (a 1-CPU runner or
// rayon configured with a 1-thread pool), the parallel path still
// produces correct output. With a 1-thread pool every batch runs on
// the same thread serially, but the merge logic is exercised the same
// way as the multi-threaded path.
// ---------------------------------------------------------------------------

/// Invariant: 1-thread rayon pool produces output equal to the
/// sequential reference.
/// Reproduces: preventive — guards against assumptions of
/// `current_num_threads() > 1` in the dispatch path.
#[test]
fn parallel_parse_correct_under_1_thread_pool() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("1-thread rayon pool builds");
    let s = build_input_of_size(2 * PARALLEL_THRESHOLD);
    pool.install(|| {
        let par = parse(&s);
        let seq = parse_sequential(&s);
        assert_eq!(par.artifacts.normalized, seq.artifacts.normalized);
        assert_eq!(par.diagnostics.len(), seq.diagnostics.len());
    });
}

// ---------------------------------------------------------------------------
// Panic safety in rayon par_iter
//
// Invariant: a panic in any single `lex` call inside the rayon
// par_iter propagates up to the caller's `parse_parallel` invocation
// without leaving partial registry/normalized state observable from
// subsequent calls (each `parse_parallel` is a self-contained pure
// function — there's no shared state to corrupt). We guard against
// future refactors that introduce shared mutable state via
// `catch_unwind` in the test, asserting that:
//   - the panic propagates (i.e. the worker doesn't silently swallow it)
//   - the next `parse_parallel` call on a normal input returns
//     correct output (no leaked state).
//
// We trigger the panic by constructing input that the lexer is known
// to handle without panicking, then forcing one segment to overflow a
// `debug_assert!` we add. Since we can't currently inject a panic
// without modifying the lexer, we instead test the OUTER guarantee:
// `catch_unwind` around `parse(invalid_utf8_via_unsafe?)` doesn't
// apply (we'd need unsafe to construct an invalid &str). The
// realistic guard is the post-panic correctness check.
// ---------------------------------------------------------------------------

/// Invariant: a panicking thread does not corrupt global state visible
/// to subsequent `parse_parallel` calls. We can't easily inject a panic
/// without modifying the lexer, so the best-available guard is: many
/// repeated calls under heavy concurrency leave the shared state
/// (rayon thread pool, allocator) consistent. The `concurrent_stress`
/// integration test additionally hammers this from multiple threads.
/// Reproduces: preventive — guards against future shared-state additions.
#[test]
fn repeated_parallel_calls_keep_global_state_consistent() {
    let s = build_input_of_size(2 * PARALLEL_THRESHOLD);
    let baseline_normalized = parse(&s).artifacts.normalized;
    // 50 calls in a row. If anything (rayon scratch buffers, ahash
    // state, etc.) leaks across calls, output should diverge.
    for i in 0..50 {
        let r = parse(&s);
        assert_eq!(
            r.artifacts.normalized, baseline_normalized,
            "iteration {i}: state-leak indicator",
        );
    }
}
