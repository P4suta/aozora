//! Trigger-byte scanner for the Aozora notation lexer.
//!
//! ## What it does
//!
//! Given a source buffer, finds the byte offsets of every Aozora
//! trigger character (`｜《》［］＃※〔〕「」`). Each is a 3-byte BMP
//! UTF-8 codepoint; the scanner streams trigram start offsets into a
//! caller-provided [`OffsetSink`], or returns a `Vec<u32>` via the
//! convenience entry [`scan_offsets`].
//!
//! ## Why `aho-corasick`, not a hand-rolled SIMD kernel
//!
//! This crate used to carry the tree's only `unsafe`: a bespoke Teddy
//! multi-pattern matcher with one SIMD inner kernel per ISA
//! (`pshufb` / `vqtbl1q_u8` / `i8x16_swizzle`) plus a scalar fallback.
//! It worked, but the candidate filter keyed only on the lead byte's
//! nibbles — and `0xE3` is the lead byte of *every hiragana and
//! katakana codepoint*, so on real Japanese prose the filter fired on
//! a large fraction of the text and paid a scalar trigram-verify to
//! reject each kana byte.
//!
//! [`aho_corasick`] solves the same problem with a safe, portable,
//! expertly-maintained packed matcher whose fingerprint spans more
//! than the lead byte, so it is *both* algorithmically more selective
//! (fewer false-positive verifies) and free of `unsafe`. On 8 MiB of
//! real prose it scanned ~24% faster than the hand-rolled SIMD while
//! producing byte-identical offsets, and it carries every platform —
//! the win is portable, not pinned to the dev machine's AVX2. The
//! crate is now `#![forbid(unsafe_code)]`.
//!
//! ## Output channel
//!
//! [`OffsetSink`] decouples the scanner from "where the offsets land".
//! `Vec<u32>` and `bumpalo::collections::Vec<'_, u32>` both implement
//! it, so callers with an arena (the lex pipeline) write offsets
//! directly into the arena. [`CountSink`] counts pushes without
//! storing, useful for capacity probes.
//!
//! ## Naive reference
//!
//! [`NaiveScanner`] is the brute-force `O(n × classify)` walker — the
//! independent oracle the production scanner is differentially tested
//! against (`tests/property_backend_equiv.rs`), and the safe scanner
//! used directly on `no_std` builds (where `aho-corasick`'s packed
//! path — which needs runtime CPU detection — is unavailable).

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
use bumpalo::Bump;
#[cfg(feature = "std")]
use bumpalo::collections::Vec as BumpVec;

mod naive;
mod trait_def;

pub use trait_def::{CountSink, OffsetSink};

#[doc(hidden)]
pub use naive::NaiveScanner;

/// The process-wide trigger automaton, built once.
///
/// `aho-corasick`'s automaton construction (and its one-time runtime
/// CPU-feature detection for the packed backend) is amortised across
/// every parse via a `OnceLock`, mirroring the old per-process backend
/// detection. The patterns come straight from the canonical
/// [`aozora_spec::trigger::ALL_TRIGGER_TRIGRAMS`] so the scanner and the
/// classifier can never disagree on the trigger set.
#[cfg(feature = "std")]
fn automaton() -> &'static aho_corasick::AhoCorasick {
    use aho_corasick::{AhoCorasick, MatchKind};
    use aozora_spec::trigger::ALL_TRIGGER_TRIGRAMS;
    use std::sync::OnceLock;

    static AUTOMATON: OnceLock<AhoCorasick> = OnceLock::new();
    AUTOMATON.get_or_init(|| {
        AhoCorasick::builder()
            .match_kind(MatchKind::Standard)
            .build(ALL_TRIGGER_TRIGRAMS)
            .expect("the 11 fixed trigger trigrams always compile")
    })
}

/// Force the one-time automaton build (and packed-backend CPU
/// detection) now, off the hot path. The first [`scan_offsets_in`] then
/// reuses the cached automaton. Idempotent and sub-microsecond.
///
/// `no_std` builds scan with [`NaiveScanner`] and have nothing to warm,
/// so this is a documented no-op there.
#[cfg(feature = "std")]
pub fn prewarm() {
    let _ = automaton();
}

/// `no_std` no-op counterpart of [`prewarm`].
#[cfg(not(feature = "std"))]
pub fn prewarm() {}

/// Push every trigger offset in `source` into `sink`, in ascending
/// order. Generic over the sink so the lex pipeline writes straight
/// into its arena with no heap round-trip.
#[cfg(feature = "std")]
fn scan_into<S: OffsetSink>(source: &str, sink: &mut S) {
    for m in automaton().find_iter(source) {
        sink.push(u32::try_from(m.start()).unwrap_or(u32::MAX));
    }
}

/// `no_std` scan: the safe naive walker (no packed backend without
/// runtime CPU detection).
#[cfg(not(feature = "std"))]
fn scan_into<S: OffsetSink>(source: &str, sink: &mut S) {
    NaiveScanner.scan(source, sink);
}

/// Scan `source` and return every trigger byte offset.
///
/// Convenience entry that allocates a fresh `Vec<u32>`. Callers with a
/// `bumpalo` arena should reach for [`scan_offsets_in`] instead — it
/// writes directly into the arena without the heap round-trip.
#[must_use]
pub fn scan_offsets(source: &str) -> alloc::vec::Vec<u32> {
    let mut sink = alloc::vec::Vec::new();
    scan_into(source, &mut sink);
    sink
}

/// Arena-backed variant: scan trigger byte offsets directly into a
/// caller-provided [`BumpVec<u32>`] living in the lex pipeline's
/// per-parse [`Bump`] arena. No heap allocation, no memcpy.
#[cfg(feature = "std")]
#[must_use]
pub fn scan_offsets_in<'a>(source: &str, arena: &'a Bump) -> BumpVec<'a, u32> {
    let mut out = BumpVec::new_in(arena);
    scan_into(source, &mut out);
    out
}

/// `no_std` variant of [`scan_offsets_in`].
#[cfg(not(feature = "std"))]
#[must_use]
pub fn scan_offsets_in<'a>(source: &str, arena: &'a Bump) -> BumpVec<'a, u32> {
    let mut out = BumpVec::new_in(arena);
    scan_into(source, &mut out);
    out
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn scan_matches_naive_on_mixed_sample() {
        // Whatever the production scanner does, it must agree with the
        // brute-force naive reference on a representative mixed sample
        // (ruby, refmark, square brackets, hash, corner brackets).
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let scanned = scan_offsets(s);
        let naive = NaiveScanner.scan_offsets(s);
        assert_eq!(scanned, naive);
        assert_eq!(scanned.len(), 8, "sample has 8 triggers");
    }

    #[test]
    fn scan_offsets_in_produces_same_offsets_as_scan_offsets() {
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let arena = Bump::new();
        let arena_offsets: alloc::vec::Vec<u32> =
            scan_offsets_in(s, &arena).iter().copied().collect();
        let heap_offsets = scan_offsets(s);
        assert_eq!(arena_offsets, heap_offsets);
    }

    #[test]
    fn skips_kana_sharing_the_e3_lead_byte() {
        // The whole motivation: あいうえお all start with 0xE3 but are
        // not triggers. The scanner must emit nothing for pure kana.
        assert!(scan_offsets("あいうえおこんにちは").is_empty());
    }
}
