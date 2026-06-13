//! Fuzz target — Phase 3 classify body-recognition on arbitrary UTF-8.
//!
//! Drives the full Phase 1 → Phase 2 → Phase 3 streaming chain
//! (`tokenize` → `pair` → `classify`) directly, bypassing the
//! arena-normalize fold so the fuzzer hammers the classifier's
//! recogniser leaves (ruby / bouten / TCY / gaiji / kaeriten /
//! annotation catch-all) in isolation. Targets the slice-indexing,
//! char-boundary, and `as`-cast paths in
//! `aozora_pipeline::lexer::phase3_classify` that consume
//! source-derived `Span` offsets — the sites that, under
//! `panic = "abort"`, would turn a malformed UTF-8 document into a
//! hard crash.
//!
//! Invariants asserted (beyond "no panic / no abort"):
//!
//! 1. **Span coverage tiling.** When `source` is non-empty the yielded
//!    [`ClassifiedSpan`]s tile every byte end-to-end:
//!    `spans[0].start == 0`, `spans[i].end == spans[i+1].start`, and
//!    `spans[last].end == source.len()`. When `source` is empty the
//!    span list is empty. This is the module-level coverage invariant
//!    Phase 4 relies on; a classify bug that drops or overlaps bytes
//!    breaks it here before it can corrupt the normalized buffer.
//! 2. **Char-boundary spans.** Every span edge lands on a UTF-8 char
//!    boundary of `source`, so the byte ranges are always sliceable.
//! 3. **In-bounds diagnostics.** Every diagnostic span is non-inverted.
//!
//! The classifier borrows from a `BorrowedAllocator`; we materialise a
//! fresh arena per iteration so allocations are reclaimed in one
//! `Bump::reset` on drop.
//!
//! Run via `just fuzz-quick aozora-pipeline classify_phase3` (or
//! `fuzz-deep` / `fuzz-marathon`).

#![no_main]

use aozora_pipeline::lexer::{ClassifiedSpan, classify, pair, tokenize};
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::Arena;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    // Phase 3 consumes Phase 0 *sanitized* text in production, but it
    // is a pure function of whatever `&str` it is handed — every
    // recogniser reads `Span` offsets back out of the same `source`.
    // Feeding it raw (un-sanitized) UTF-8 is therefore a strictly
    // wider adversarial surface than the sanitized path: it exercises
    // the same slice / cast / char-boundary code with positions that
    // need not respect any Phase 0 post-condition.
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::new(&arena);
    let mut pair_stream = pair(tokenize(src));

    let mut spans: Vec<ClassifiedSpan<'_>> = Vec::new();
    let classify_diags = {
        let mut stream = classify(&mut pair_stream, src, &mut alloc);
        for span in &mut stream {
            spans.push(span);
        }
        stream.take_diagnostics()
    };

    // Invariant 1 + 2: contiguous tiling on char boundaries.
    if src.is_empty() {
        assert!(
            spans.is_empty(),
            "empty source must yield no spans; got {spans:?} for src bytes = {data:?}",
        );
    } else {
        assert_eq!(
            spans[0].source_span.start, 0,
            "first span must start at byte 0; src bytes = {data:?}",
        );
        let src_len = u32::try_from(src.len()).expect("fuzz input < 4 GiB");
        let mut prev_end = 0u32;
        for (i, span) in spans.iter().enumerate() {
            let s = span.source_span;
            assert!(
                s.start <= s.end,
                "span {i} inverted: {s:?}; src bytes = {data:?}",
            );
            assert_eq!(
                s.start, prev_end,
                "span {i} does not abut previous span end ({prev_end}); \
                 spans = {spans:?}; src bytes = {data:?}",
            );
            // Char-boundary check makes the slice well-formed; an
            // off-boundary edge would panic on the slice below in
            // production (block-leaf / plain emission).
            assert!(
                src.is_char_boundary(s.start as usize),
                "span {i} start {} is not a char boundary; src bytes = {data:?}",
                s.start,
            );
            assert!(
                src.is_char_boundary(s.end as usize),
                "span {i} end {} is not a char boundary; src bytes = {data:?}",
                s.end,
            );
            // The slice itself must not panic (exercises the exact
            // indexing the arena normalizer performs for Plain spans).
            let _ = &src[s.start as usize..s.end as usize];
            prev_end = s.end;
        }
        assert_eq!(
            prev_end, src_len,
            "spans must tile through source end ({src_len}); \
             spans = {spans:?}; src bytes = {data:?}",
        );
    }

    // Invariant 3: diagnostic spans are non-inverted. We do not bound
    // `end` against `src.len()` — classify diagnostics are emitted in
    // source coordinates and may legitimately point at the EOF cursor.
    let mut diagnostics = pair_stream.take_diagnostics();
    diagnostics.extend(classify_diags);
    for diag in &diagnostics {
        let span = diag.span();
        assert!(
            span.start <= span.end,
            "diagnostic span {span:?} has start > end; src bytes = {data:?}",
        );
    }
});
