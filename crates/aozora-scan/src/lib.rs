//! Trigger-byte scanner for the Aozora notation lexer — multi-backend.
//!
//! ## What it does
//!
//! Given a source buffer, finds the byte offsets of every Aozora
//! trigger character (`｜《》［］＃※〔〕「」`). Each is a 3-byte BMP
//! UTF-8 codepoint; the scanner returns a sorted `Vec<u32>` of trigram
//! start offsets, validated by the const-PHF
//! [`aozora_spec::classify_trigger_bytes`].
//!
//! ## Design
//!
//! [`TriggerScanner`] is a `dyn`-compatible trait so multiple backends
//! coexist behind a single runtime dispatcher [`best_scanner`]. Three
//! backends ship in production:
//!
//! - [`TeddyScanner`] — Hyperscan multi-pattern fingerprint matcher
//!   via `aho_corasick::packed::Searcher` (Langdale 2015, BurntSushi
//!   port 2019). Production winner; ~10-20 GiB/s on Japanese.
//! - [`StructuralBitmapScanner`] (`x86_64`+AVX2) — simdjson-style
//!   two-byte (lead × middle) AVX2 candidate filter
//!   (Langdale & Lemire 2019). Production fallback when Teddy can't
//!   build (no SSSE3).
//! - [`DfaScanner`] — Hoehrmann-style multi-pattern byte DFA via
//!   `regex_automata::dfa::dense`. Universal SIMD-free fallback.
//! - [`NaiveScanner`] (`#[doc(hidden)]`) — brute-force PHF reference
//!   used by the proptest cross-check in each backend module.
//!
//! ## Output shape
//!
//! Scanning produces a sorted `Vec<u32>` of trigger start offsets.
//! The lex driver weaves them with surrounding plain text and
//! merges adjacent `《《` / `》》` into the double variants at its
//! layer (the scanner emits them as two adjacent single-trigger
//! offsets).

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::vec::Vec;

use bumpalo::Bump;
use bumpalo::collections::Vec as BumpVec;

mod backends;
mod naive;

#[cfg(feature = "std")]
pub use backends::TeddyScanner;

#[cfg(feature = "std")]
pub use backends::DfaScanner;

#[cfg(target_arch = "x86_64")]
pub use backends::StructuralBitmapScanner;

#[doc(hidden)]
pub use naive::NaiveScanner;

/// A backend that finds trigger-byte candidate positions in a UTF-8
/// source buffer.
///
/// Implementations are stateless; instantiate one and reuse it across
/// scans. The trait is `dyn`-compatible (no generic methods) so the
/// lex layer can hold a `&'static dyn TriggerScanner` selected at
/// runtime via CPU feature detection.
///
/// ## Streaming variant (deferred)
///
/// The current shape returns `Vec<u32>` eagerly, which on a 2 MB
/// source allocates ~80 KB of `u32` offsets (assuming ~2 % trigger
/// density). That is < 1 % of the downstream `Vec<Token>` size and
/// has not surfaced in any profile to date. If a future workload
/// pushes either density (annotation-dense docs) or source size
/// (multi-MB single docs) hard enough that this allocation matters,
/// add an `Iterator<Item = u32>` variant — note that the AVX2
/// 32-byte chunked movemask + Kernighan extraction is significantly
/// more awkward as a coroutine than as a one-shot, so the eager
/// shape is preferred until measurement disagrees.
pub trait TriggerScanner {
    /// Scan `source` and return all byte offsets at which a trigger
    /// character begins, in ascending order.
    ///
    /// The returned offsets are guaranteed to:
    /// 1. Lie on UTF-8 character boundaries (each is the start of a
    ///    3-byte trigger sequence).
    /// 2. Point at one of the 11 single-character triggers
    ///    (`｜《》［］＃※〔〕「」`). The double-character variants
    ///    `《《` / `》》` produce two adjacent offsets here; the lex
    ///    layer fuses them as needed.
    /// 3. Lie within `source.len()`.
    ///
    /// `source` must be valid UTF-8 — the same precondition as
    /// [`str::as_bytes`]. The scanner does not decode it; we operate
    /// on the byte view because every trigger is 3 bytes long.
    fn scan_offsets(&self, source: &str) -> Vec<u32>;
}

/// The runtime-best [`TriggerScanner`] for the current target.
///
/// Dispatch order (best to worst):
///
/// 1. **[`TeddyScanner`]** — built once via `OnceLock` (Hyperscan
///    Teddy via `aho_corasick::packed`). Returns `None` on hosts
///    without SSSE3, in which case we fall through.
/// 2. **[`StructuralBitmapScanner`]** — `x86_64` + AVX2 only;
///    simdjson-style two-byte (lead × middle) bitmap. Used when
///    Teddy isn't available but we still have AVX2.
/// 3. **[`DfaScanner`]** — universal SIMD-free fallback
///    (`regex_automata` dense byte DFA over the 11 trigger
///    trigrams). Used on minimal-ISA hosts.
/// 4. **[`NaiveScanner`]** — `no_std` last resort.
///
/// All four are byte-identical to each other (proptest
/// cross-checked against `NaiveScanner` in each backend module);
/// callers can blindly trust the dispatcher's choice.
#[cfg(feature = "std")]
#[must_use]
pub fn best_scanner() -> &'static dyn TriggerScanner {
    use std::sync::OnceLock;

    static TEDDY: OnceLock<Option<TeddyScanner>> = OnceLock::new();
    static DFA: OnceLock<DfaScanner> = OnceLock::new();

    if let Some(t) = TEDDY.get_or_init(TeddyScanner::new) {
        return t;
    }

    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        return &StructuralBitmapScanner;
    }

    DFA.get_or_init(DfaScanner::new)
}

/// `no_std` variant: the only backend that doesn't pull in `alloc`-
/// hungry searcher infrastructure is the brute-force PHF walker.
#[cfg(not(feature = "std"))]
#[must_use]
pub fn best_scanner() -> &'static dyn TriggerScanner {
    &NaiveScanner
}

/// Name of the backend [`best_scanner`] would select on this host,
/// for diagnostic / logging purposes.
///
/// Pure inspection — no SIMD work. Callers that want to confirm the
/// chosen backend is firing in production can `eprintln!` or log this
/// once at startup without needing to add `log` as a dependency to
/// the lex layer (this crate stays `no_std`-clean).
#[must_use]
pub fn best_scanner_name() -> &'static str {
    #[cfg(feature = "std")]
    {
        // Mirror the dispatch order of `best_scanner` so the two
        // stay in sync. Cheap because TeddyScanner::new is the same
        // call best_scanner caches via OnceLock — but we don't share
        // the cache here to keep this fn doc-comment trivially
        // inspectable as "no allocation, no SIMD".
        if TeddyScanner::new().is_some() {
            return "teddy";
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            return "structural_bitmap";
        }
        "dfa"
    }
    #[cfg(not(feature = "std"))]
    {
        "naive"
    }
}

/// Arena-backed variant: scan into a [`BumpVec<u32>`] instead of the
/// heap [`Vec<u32>`] [`TriggerScanner::scan_offsets`] returns. Lifts
/// the scratch buffer into the per-parse arena the lex pipeline
/// already owns, avoiding the heap allocation + `Vec::extend_desugared`
/// chain on the trigger-offset path.
///
/// `dyn`-compatibility constraint: `TriggerScanner` is held as a
/// `&'static dyn` (see [`best_scanner`]) so its trait method cannot
/// have a lifetime-generic. Instead this is a free function that
/// (1) calls the scanner's heap entry point, (2) re-homes the
/// resulting offsets into the caller's arena, (3) drops the heap
/// `Vec`. Net: one bounded heap alloc per parse (cap-tuned in each
/// backend at 1.8 % density) + one short memcpy into the arena,
/// vs the old shape's repeated heap doublings + a heap result that
/// outlived the parse to the inter-phase boundary.
#[cfg(feature = "std")]
#[must_use]
pub fn scan_offsets_in<'a>(source: &str, arena: &'a Bump) -> BumpVec<'a, u32> {
    let scratch = best_scanner().scan_offsets(source);
    let mut out = BumpVec::with_capacity_in(scratch.len(), arena);
    out.extend_from_slice(&scratch);
    out
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    /// On any modern x86_64 dev/CI host (~2026) the dispatcher
    /// should land on Teddy. Asserting it directly catches the
    /// next-most-likely refactor mistake: silently downgrading the
    /// dispatcher to a slower backend by reordering the cfg branches.
    #[test]
    #[cfg(all(feature = "std", target_arch = "x86_64"))]
    fn dispatcher_picks_teddy_when_supported() {
        // SSSE3 is universal on `x86_64-v2` and above — every
        // 2026-era CI runner has it. If this assertion ever fires,
        // it means TeddyScanner::new is returning None even though
        // SSSE3 *should* be available, which is a real regression
        // worth investigating, not a flaky test.
        if std::is_x86_feature_detected!("ssse3") {
            assert_eq!(best_scanner_name(), "teddy");
        }
    }

    #[test]
    fn dispatcher_returns_byte_identical_results() {
        // Whatever backend the dispatcher picks, it must agree
        // with the brute-force reference on a representative
        // mixed-Japanese sample (ruby, refmark, square brackets,
        // hash, corner brackets — 8 triggers).
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let dispatched = best_scanner().scan_offsets(s);
        let naive = NaiveScanner.scan_offsets(s);
        assert_eq!(dispatched, naive);
        assert_eq!(dispatched.len(), 8, "sample has 8 triggers");
    }
}
