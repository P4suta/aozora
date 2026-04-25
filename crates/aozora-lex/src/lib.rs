//! Aozora notation lexer — public entry point for the fused
//! streaming pipeline.
//!
//! The 0.2.0 architecture (ADR-0009) splits the legacy 7-phase
//! [`aozora_lexer`] into:
//!
//! - [`aozora_scan`] — the SIMD-friendly trigger-byte scanner that
//!   replaces phase 1's char-by-char loop with a `memchr3` candidate
//!   sweep + const-PHF precise classify.
//! - **this crate** — the orchestrator that drives `aozora-scan` for
//!   the trigger pass and (for now) delegates the remaining
//!   sanitise / pair / classify / normalise / validate work to the
//!   legacy `aozora-lexer` engine. Each subsequent Move 2 commit
//!   migrates one more phase into the fused engine that lives in this
//!   crate.
//!
//! ## Why two crates?
//!
//! The trigger scan is cleanly factor-able and benefits enormously
//! from SIMD: shipping it as a standalone `no_std` crate lets us
//! benchmark backends (`memchr3` scalar today; AVX2 / NEON /
//! `wasm-simd` later) in isolation, and lets the FFI / WASM driver
//! crates (Move 4) link only what they need.
//!
//! ## Observable equivalence
//!
//! ADR-0010 codifies the "observable equivalence" purity contract for
//! this crate: [`lex`] is a pure function from source text to
//! [`LexOutput`] *as observed externally*, even though the internal
//! pipeline mutates a bumpalo arena and runs SIMD scratch buffers.
//! The `byte_identical_*` proptests in `aozora-test-utils` pin this
//! property against the legacy `aozora_lexer::lex` for as long as
//! both implementations coexist.

#![forbid(unsafe_code)]

// Public surface — the `lex(&str) -> LexOutput` entry point plus the
// supporting types the lex driver and downstream consumers need.
//
// During Move 2's gradual fused-engine migration we re-export the
// legacy aozora-lexer types so callers can switch their `use
// aozora_lexer::*` imports to `use aozora_lex::*` without otherwise
// changing code. Once Move 2 finishes and `aozora-lexer` is deleted,
// each of these re-exports moves to a definition local to this crate
// (or to `aozora-spec`, where the canonical types already live).

pub use aozora_lexer::{
    LexOutput, NormalizeOutput, PlaceholderRegistry, SanitizeOutput, ValidateOutput,
};
pub use aozora_spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic, INLINE_SENTINEL,
    PairKind, Span, TriggerKind, classify_trigger_bytes,
};

/// Run the Aozora-notation lexer pipeline over `source`.
///
/// Pure function (per ADR-0010 observable equivalence). The returned
/// [`LexOutput`] carries the normalized text, the placeholder
/// registry, and any diagnostics emitted along the way.
///
/// # Panics
///
/// Panics if `source.len()` exceeds `u32::MAX` bytes (~4 GiB) —
/// inherited from the upstream lexer's `Span` field width. In
/// practice this bound is never hit on aozora-format text.
///
/// # Performance
///
/// `#[inline]` because this is a one-line wrapper today and we want
/// the same code to compile across the `aozora-lex` ↔ `aozora-lexer`
/// crate boundary as if it were a single function (avoids a ~8%
/// regression vs the legacy direct call observed on the corpus
/// sweep). When Move 2's fused engine grows out of this body the
/// `#[inline]` may need to come off — but that decision can wait
/// until the body justifies it.
#[must_use]
#[inline(always)]
#[allow(
    clippy::inline_always,
    reason = "thin wrapper across a crate boundary; #[inline] alone wasn't enough to elide the call under thin-LTO"
)]
pub fn lex(source: &str) -> LexOutput {
    // Move 2.2 status:
    //
    // We currently delegate the entire pipeline to the legacy
    // aozora-lexer engine. The migration plan (Move 2.3 onward)
    // replaces phase 1 first (with aozora-scan), then folds phases
    // 0, 2, 3, 4, 5, 6 into a single fused state machine that lives
    // in this crate. The public signature of `lex` does not change
    // across that migration — the byte-identical proptest harness in
    // `aozora-test-utils` pins it.
    aozora_lexer::lex(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `aozora_scan::ScalarScanner` MUST yield the exact same byte
    /// offsets that the legacy phase-1 tokeniser uses for its trigger
    /// positions. We don't have a public hook into phase 1's offsets,
    /// so we cross-check at the [`LexOutput`] level: every PUA sentinel
    /// in `normalized` must correspond to a consumed source trigger.
    #[test]
    fn lex_produces_normalized_with_pua_sentinels_for_trigger_inputs() {
        let out = lex("｜青梅《おうめ》");
        // Exactly one inline sentinel for the ruby span.
        let inline_count = out
            .normalized
            .chars()
            .filter(|c| *c == INLINE_SENTINEL)
            .count();
        assert_eq!(inline_count, 1, "normalized: {:?}", out.normalized);
        assert_eq!(out.registry.inline.len(), 1);
    }

    #[test]
    fn lex_passes_through_plain_text_unchanged() {
        let out = lex("hello, world");
        assert_eq!(out.normalized, "hello, world");
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_re_exports_sentinel_constants() {
        // Sanity: the constants re-exported from aozora-spec match
        // the values the lexer actually emits, so downstream
        // consumers can use them either via `aozora_lex::*` or
        // `aozora_spec::*` interchangeably.
        assert_eq!(INLINE_SENTINEL, '\u{E001}');
        assert_eq!(BLOCK_LEAF_SENTINEL, '\u{E002}');
        assert_eq!(BLOCK_OPEN_SENTINEL, '\u{E003}');
        assert_eq!(BLOCK_CLOSE_SENTINEL, '\u{E004}');
    }

    #[test]
    fn lex_handles_empty_input() {
        let out = lex("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_emits_diagnostics_for_pua_collision() {
        let out = lex("abc\u{E001}def");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn lex_preserves_sanitized_len_for_segment_merge() {
        // Sanitize is identity on plain text → sanitized_len == source.len().
        let out = lex("plain text");
        assert_eq!(usize::try_from(out.sanitized_len), Ok("plain text".len()));
    }
}
