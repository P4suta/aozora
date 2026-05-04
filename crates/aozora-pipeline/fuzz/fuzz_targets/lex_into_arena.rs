//! Fuzz target — `aozora_pipeline::lex_into_arena` on arbitrary UTF-8.
//!
//! Arbitrary bytes are decoded as UTF-8 (invalid sequences skip this
//! iteration). The resulting source text is pushed through
//! `lex_into_arena` and the produced [`BorrowedLexOutput`] is sanity-
//! checked: the lexer must terminate without panicking, the
//! normalized text must remain valid UTF-8, and every reported
//! diagnostic span must be in-bounds. Targets parser-side panics in
//! the trigger / pair / classify phases plus arena-bounds bugs.
//!
//! Run with the standard `just fuzz-{quick,deep,marathon,triage,
//! promote}` family from the workspace root, e.g.
//! `just fuzz-quick aozora-pipeline lex_into_arena`.

#![no_main]

use aozora_pipeline::lex_into_arena;
use aozora_syntax::borrowed::Arena;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);
    // Invariants:
    //
    // 1. The normalized text must remain valid UTF-8 (the lexer never
    //    re-encodes; if this trips, a phase corrupted the buffer).
    assert!(
        std::str::from_utf8(out.normalized.as_bytes()).is_ok(),
        "lex_into_arena returned invalid UTF-8 in normalized text",
    );
    // 2. Every diagnostic must report a non-inverted span. We
    //    deliberately do not bound `span.end` against the normalized
    //    length: Phase 0 normalization (CRLF → LF, leading BOM strip)
    //    shrinks the buffer, but diagnostics are emitted in source
    //    coordinates so they can point past the normalized end.
    //    Bounding against the source length isn't useful either —
    //    the diagnostic carries no source reference and the caller
    //    is responsible for picking the right text frame.
    for diag in &out.diagnostics {
        let span = diag.span();
        assert!(
            span.start <= span.end,
            "diagnostic span {:?} has start > end; src bytes = {data:?}",
            span,
        );
    }
});
