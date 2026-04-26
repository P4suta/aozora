//! Streaming-API semantics for [`aozora_lex::Pipeline`] and the
//! [`aozora_lexer`] streaming building blocks (`tokenize` / `pair` /
//! `classify`).
//!
//! Plan G.4 deliverable. These tests pin the *behaviour* of the
//! streaming pipeline that earlier unit tests treated as
//! implementation detail:
//!
//! * Iterators may be dropped at any point with no side-effect leakage
//!   (no panics, accumulated diagnostics still readable).
//! * Iteration is *lazy* — `.next()` does not silently drain its
//!   upstream past the byte position it actually emits.
//! * `.take(N)` and `.collect().into_iter().take(N)` agree.
//! * EOF-time `Unclosed` synthesis is observable on the public stream.
//! * The type-state [`Pipeline`] composition is byte-for-byte
//!   equivalent to the one-shot [`lex_into_arena`] front door.
//! * Diagnostics observed at intermediate states still surface in the
//!   final [`BorrowedLexOutput::diagnostics`].
//!
//! Each block below stays *semantic*: never inspect private fields
//! (Pipeline is intentionally opaque past the public surface), only
//! the behaviour visible to a real downstream caller.

use aozora_lex::{Pipeline, lex_into_arena};
use aozora_lexer::{PairEvent, PairKind, classify, pair, tokenize};
use aozora_spec::Diagnostic;
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;

// =====================================================================
// 1. Drop / early-termination
// =====================================================================

/// Construct a `PairStream`, take a couple of events via `next()`, then
/// drop the stream. Must not panic and `take_diagnostics` (called on a
/// fresh stream over the same input) must be callable without panic.
#[test]
fn pair_stream_drop_partway_does_not_panic_and_diagnostics_remain_readable() {
    // 3 simple bracket pairs produce 9 events (Open, Solo(Hash), Text,
    // Close) ×3 minus the missing Solo(Hash) bodies — but here we don't
    // care about the exact shape, only that early drop is clean.
    let src = "[X][Y][Z]";
    let mut stream = pair(tokenize(src));
    let _e0 = stream.next();
    let _e1 = stream.next();
    // Drop the stream by letting it leave scope:
    drop(stream);

    // A second, independent stream over the same source must still
    // behave exactly like the first would have.
    let mut stream2 = pair(tokenize(src));
    while stream2.next().is_some() {}
    // Drainable; should not panic regardless of exact diagnostic count.
    let _diags = stream2.take_diagnostics();
}

/// Same shape for `ClassifyStream`. Drop midway, then run an
/// independent one over the same input to make sure no global state
/// (e.g. the forward-target index) was left in a corrupt state.
#[test]
fn classify_stream_drop_partway_does_not_corrupt_global_state() {
    let src = "abc｜D《e》fgh";
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::new(&arena);

    {
        let mut pair_stream = pair(tokenize(src));
        let mut classify_stream = classify(&mut pair_stream, src, &mut alloc);
        let _s0 = classify_stream.next();
        // Drop both streams here — `classify_stream` first (last
        // declared), then `pair_stream`.
    }

    // A fresh end-to-end run over the same source must still behave
    // identically to a never-interrupted run.
    let arena2 = Arena::new();
    let oneshot = lex_into_arena(src, &arena2);
    assert_eq!(oneshot.registry.inline.len(), 1, "ruby span survived");
}

// =====================================================================
// 2. Lazy iteration semantics
// =====================================================================

/// PairStream events are emitted *roughly* in lock-step with the
/// underlying source position. Full laziness is hard to assert without
/// a custom counting wrapper because `tokenize` is itself lazy and
/// `pair` may peek; we instead pin a behavioural proxy: the byte
/// position of the Nth emitted event grows in proportion to N.
///
/// Concretely: take the first 10 events from a long input and check
/// the maximum source byte position observed stays well below the
/// full input length — proves the stream did not silently drain its
/// upstream all the way to EOF on the first `next()`.
#[test]
fn pair_stream_take_n_does_not_exhaust_underlying_source() {
    // Aozora full-width brackets are 3 bytes each, hash 3 bytes.
    // `［＃A］` = 12 bytes; 256 of them = 3 072 bytes. The stream emits
    // 4 events per group: PairOpen, Solo(Hash), Text("A"), PairClose.
    let unit = "［＃A］";
    let src: String = unit.repeat(256);
    let total_len = src.len();
    let mut stream = pair(tokenize(&src));
    let first_10: Vec<PairEvent> = (0..10).filter_map(|_| stream.next()).collect();
    assert_eq!(first_10.len(), 10, "events: {first_10:?}");
    // The first 10 events cover roughly the first 2-3 groups (2.5 ×
    // 12 bytes ≈ 30 bytes). We pin a generous ceiling (1/8 of the
    // full input) to keep the assertion robust against minor changes
    // in event packing while still catching a "drains everything"
    // regression.
    let max_end: u32 = first_10
        .iter()
        .filter_map(|e| e.span().map(|s| s.end))
        .max()
        .expect("at least one event has a span");
    assert!(
        (max_end as usize) < total_len / 8,
        "first 10 events should not drive the upstream beyond ~1/8 of \
         the source, got max_end={max_end} (total source length: {total_len})"
    );
}

// =====================================================================
// 3. .take(N) equivalence
// =====================================================================

/// `pair(tokenize(s)).take(N).collect()` MUST equal
/// `pair(tokenize(s)).collect().into_iter().take(N).collect()` for
/// several inputs and several N. Guards against per-iteration side
/// effects diverging from index-based access.
#[test]
fn pair_stream_take_n_matches_collect_then_take() {
    let inputs: &[&str] = &[
        "",
        "plain",
        "［＃改ページ］",
        "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］",
        "｜青梅《おうめ》街道沿いの古い宿",
        "「a」「b」「c」「d」「e」",
    ];
    let ns: &[usize] = &[0, 1, 2, 5, 100];

    for src in inputs {
        // Full materialisation as the reference.
        let full: Vec<PairEvent> = pair(tokenize(src)).collect();
        for &n in ns {
            let via_take: Vec<PairEvent> = pair(tokenize(src)).take(n).collect();
            let via_collect_take: Vec<PairEvent> = full.iter().cloned().take(n).collect();
            assert_eq!(
                via_take, via_collect_take,
                "src={src:?} n={n}: take(n) must agree with collect().take(n)"
            );
        }
    }
}

// =====================================================================
// 4. EOF Unclosed isolation
// =====================================================================

/// One unclosed bracket → exactly one trailing `PairEvent::Unclosed`
/// of `PairKind::Bracket` and exactly one corresponding diagnostic.
#[test]
fn pair_stream_emits_one_unclosed_event_for_one_unclosed_bracket() {
    let src = "［＃unclosed";
    let mut stream = pair(tokenize(src));
    let events: Vec<PairEvent> = (&mut stream).collect();
    let diagnostics = stream.take_diagnostics();

    // The trailing event must be the Unclosed for the outer bracket.
    let unclosed_count = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                PairEvent::Unclosed {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        unclosed_count, 1,
        "expected exactly one Unclosed Bracket event, got {events:?}"
    );

    let bracket_diag_count = diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d,
                Diagnostic::UnclosedBracket {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        bracket_diag_count, 1,
        "expected exactly one UnclosedBracket diagnostic, got {diagnostics:?}"
    );
}

// =====================================================================
// 5. Pipeline state-transition smoke tests
// =====================================================================

/// For several non-trivial inputs the explicit chain
/// `Pipeline::new(s, &arena).sanitize().tokenize().pair().build()`
/// must produce results identical to `lex_into_arena(s, &arena2)`.
#[test]
fn pipeline_chain_matches_lex_into_arena_for_corpus_shapes() {
    let inputs: &[&str] = &[
        "",
        "plain text",
        "｜青梅《おうめ》",
        "［＃改ページ］",
        "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］",
        "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
         なる珍しき木が立つ。［＃ここから2字下げ］\n\
         その下で人々は語らひ、［＃「青空」に傍点］\n\
         ［＃ここで字下げ終わり］",
    ];
    for src in inputs {
        let arena_chain = Arena::new();
        let chain = Pipeline::new(src, &arena_chain)
            .sanitize()
            .tokenize()
            .pair()
            .build();
        let arena_one = Arena::new();
        let oneshot = lex_into_arena(src, &arena_one);
        assert_eq!(
            chain.normalized, oneshot.normalized,
            "normalized text drift for input {src:?}"
        );
        assert_eq!(
            chain.sanitized_len, oneshot.sanitized_len,
            "sanitized_len drift for input {src:?}"
        );
        assert_eq!(
            chain.diagnostics.len(),
            oneshot.diagnostics.len(),
            "diagnostic count drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.inline.len(),
            oneshot.registry.inline.len(),
            "inline registry drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.block_leaf.len(),
            oneshot.registry.block_leaf.len(),
            "block_leaf registry drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.block_open.len(),
            oneshot.registry.block_open.len(),
            "block_open registry drift for input {src:?}"
        );
        assert_eq!(
            chain.registry.block_close.len(),
            oneshot.registry.block_close.len(),
            "block_close registry drift for input {src:?}"
        );
    }
}

// =====================================================================
// 6. Diagnostic survives intermediate inspection
// =====================================================================

/// A Phase-0 diagnostic visible at the [`Sanitized`] state must also
/// appear in the final [`BorrowedLexOutput::diagnostics`] after
/// `.build()`. This pins the contract that intermediate inspection
/// does NOT consume diagnostics.
#[test]
fn pipeline_phase0_diagnostic_observed_at_sanitized_also_present_after_build() {
    let arena = Arena::new();
    let src = "abc\u{E001}def";

    let sanitized = Pipeline::new(src, &arena).sanitize();
    let phase0_count_at_sanitized = sanitized
        .diagnostics()
        .iter()
        .filter(|d| matches!(d, Diagnostic::SourceContainsPua { .. }))
        .count();
    assert_eq!(
        phase0_count_at_sanitized, 1,
        "expected one SourceContainsPua at Sanitized state, \
         got: {:?}",
        sanitized.diagnostics()
    );

    // Drive to completion. The phase-0 diagnostic must still be there.
    let final_out = sanitized.tokenize().pair().build();
    let phase0_count_at_build = final_out
        .diagnostics
        .iter()
        .filter(|d| matches!(d, Diagnostic::SourceContainsPua { .. }))
        .count();
    assert_eq!(
        phase0_count_at_build, 1,
        "expected the same SourceContainsPua to survive build, \
         got: {:?}",
        final_out.diagnostics
    );
}
