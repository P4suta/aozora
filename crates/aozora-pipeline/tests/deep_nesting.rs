//! Deep-nesting stress for the streaming pair + classify pipeline.
//!
//! Plan G.4 deliverable. Two things to pin:
//!
//! 1. The Frame body buffer (a `SmallVec<[PairEvent; 16]>`) handles
//!    nesting that exceeds the inline capacity by spilling to the heap
//!    transparently — the registry shape at 16, 64 and 256 levels of
//!    nesting must stay structurally consistent.
//! 2. The streaming classifier's recognition is iterative, not
//!    recursive (see `ClassifyStream::iter` + the `inner_stack` in
//!    `Frame`). 256 nested annotations must therefore not blow the
//!    test-thread stack.
//!
//! The shape we use throughout: nested annotation bodies of the form
//!
//!   `[#[#[#...]]]`
//!
//! Each `[#...]` is a `PairOpen(Bracket) + Solo(Hash) + body +
//! PairClose(Bracket)`; the whole thing is unrecognised by Phase 3
//! (no keyword matches), so it folds into a single `Annotation { kind:
//! Unknown }` for the OUTERMOST bracket. The innermost-out annotation
//! is what we care about for the iterative-classify smoke; the
//! registry shape of `1 inline entry + 0 leaves + 0 containers`
//! stays the same regardless of nesting depth, so structure
//! consistency is the load-bearing assertion.

use aozora_pipeline::lex_into_arena;
use aozora_spec::Sentinel;
use aozora_syntax::borrowed::Arena;

/// Build `［＃` × depth followed by `］` × depth.
fn nested_annotation(depth: usize) -> String {
    let mut s = String::with_capacity(depth * 9); // 6 bytes for `［＃`, 3 for `］`
    for _ in 0..depth {
        s.push_str("［＃");
    }
    for _ in 0..depth {
        s.push('］');
    }
    s
}

/// 16 levels — exactly fills the `SmallVec` inline capacity.
#[test]
fn nested_annotations_16_levels_lex_without_panic() {
    let src = nested_annotation(16);
    let arena = Arena::new();
    let out = lex_into_arena(&src, &arena);
    // Outermost bracket pair is the only top-level frame so it
    // produces exactly one inline `Annotation` registry entry. No
    // diagnostics (every `［＃` has a matching `］`).
    assert_eq!(
        out.registry.count_kind(Sentinel::Inline),
        1,
        "16-deep nested annotation must yield one outer inline entry, \
         got {}",
        out.registry.count_kind(Sentinel::Inline)
    );
    assert_eq!(out.registry.count_kind(Sentinel::BlockLeaf), 0);
    assert_eq!(out.registry.count_kind(Sentinel::BlockOpen), 0);
    assert_eq!(out.registry.count_kind(Sentinel::BlockClose), 0);
    assert!(
        out.diagnostics.is_empty(),
        "balanced nesting must not emit diagnostics, got {:?}",
        out.diagnostics
    );
}

/// 64 levels — forces the `SmallVec` body buffer to spill to the heap.
/// The result must still be structurally identical to the 16-level
/// case (same registry shape, no diagnostics).
#[test]
fn nested_annotations_64_levels_spill_to_heap_unchanged() {
    let src = nested_annotation(64);
    let arena = Arena::new();
    let out = lex_into_arena(&src, &arena);
    assert_eq!(out.registry.count_kind(Sentinel::Inline), 1);
    assert_eq!(out.registry.count_kind(Sentinel::BlockLeaf), 0);
    assert_eq!(out.registry.count_kind(Sentinel::BlockOpen), 0);
    assert_eq!(out.registry.count_kind(Sentinel::BlockClose), 0);
    assert!(out.diagnostics.is_empty());
}

/// 256 levels — well past any plausible real-corpus nesting, included
/// to verify the recognition loop is iterative (the classifier walks
/// `frame.inner_stack` rather than recursing into nested helpers).
/// A recursive helper at this depth would blow Rust's default 2 MiB
/// thread stack; this test therefore doubles as an iterative-classify
/// smoke.
#[test]
fn nested_annotations_256_levels_do_not_overflow_stack() {
    let src = nested_annotation(256);
    let arena = Arena::new();
    let out = lex_into_arena(&src, &arena);
    assert_eq!(out.registry.count_kind(Sentinel::Inline), 1);
    assert_eq!(out.registry.count_kind(Sentinel::BlockLeaf), 0);
    assert_eq!(out.registry.count_kind(Sentinel::BlockOpen), 0);
    assert_eq!(out.registry.count_kind(Sentinel::BlockClose), 0);
    assert!(out.diagnostics.is_empty());
}

/// Asymmetric content inside each nesting level. Builds
/// `［＃A［＃B［＃C［＃D］］］］` and verifies the result is byte-equal
/// to a re-run (determinism) and produces the expected single outer
/// inline entry.
#[test]
fn nested_annotations_asymmetric_bodies_classify_consistently() {
    // Each layer carries its own ASCII letter so the body bytes change
    // at every depth; a regression in body-buffer indexing would shift
    // the inline registry's content.
    let src = "［＃A［＃B［＃C［＃D］］］］";
    let arena_a = Arena::new();
    let a = lex_into_arena(src, &arena_a);
    let arena_b = Arena::new();
    let b = lex_into_arena(src, &arena_b);

    // One outer inline entry, identical across runs.
    assert_eq!(a.registry.count_kind(Sentinel::Inline), 1);
    assert_eq!(b.registry.count_kind(Sentinel::Inline), 1);
    assert_eq!(
        a.normalized, b.normalized,
        "asymmetric nesting must be deterministic"
    );
    // No diagnostics for a balanced shape.
    assert!(a.diagnostics.is_empty(), "got {:?}", a.diagnostics);
}

/// One more asymmetric shape: alternate `［＃X］` solo annotations
/// followed by a deeply-nested chain. Two top-level brackets ⇒ two
/// inline entries.
#[test]
fn mixed_solo_and_nested_annotations_yield_expected_registry_entries() {
    // First `［＃X］` is a top-level solo; second is the nested chain.
    let src = "［＃X］［＃A［＃B［＃C］］］";
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);
    assert_eq!(
        out.registry.count_kind(Sentinel::Inline),
        2,
        "expected 2 outer inline entries, got {}",
        out.registry.count_kind(Sentinel::Inline)
    );
    assert!(
        out.diagnostics.is_empty(),
        "balanced shape must not emit diagnostics, got {:?}",
        out.diagnostics
    );
}
