//! Trigger-byte scanner for the Aozora notation lexer.
//!
//! ## What it does
//!
//! Given a source buffer, finds the byte offsets of every Aozora
//! trigger character (`｜《》［］＃※〔〕「」`). Each is a 3-byte BMP
//! UTF-8 codepoint; the scanner streams trigram start offsets into
//! a caller-provided [`OffsetSink`], or returns a `Vec<u32>` via
//! the convenience entry [`scan_offsets`].
//!
//! ## Architecture
//!
//! Two orthogonal axes drive the production scanner (both kept
//! crate-private; the public surface is `BackendChoice`,
//! `OffsetSink`, `CountSink`, and the `scan_offsets*` shims):
//!
//! - **Algorithm** (`crate::kernel`) — the hand-rolled Teddy outer
//!   driver runs the candidate-filter / verify pipeline against a
//!   per-platform inner kernel.
//! - **Platform** (`crate::arch`) — each per-ISA kernel
//!   implements the `TeddyInner::lead_mask_chunk` interface using
//!   the appropriate SIMD intrinsics — `pshufb` / `_mm256_shuffle_epi8`
//!   on x86_64, `vqtbl1q_u8` on aarch64, `i8x16_swizzle` on wasm32.
//!
//! [`BackendChoice`] resolves to the fastest kernel the host can
//! run via runtime CPU detection (cached in `OnceLock`); the
//! `match`-based [`BackendChoice::scan`] gives static dispatch into
//! the monomorphised Teddy outer driver, so the SIMD inner kernel
//! inlines through the outer driver into the call site.
//!
//! ## Output channel
//!
//! [`OffsetSink`] decouples the scanner from "where the offsets
//! land". `Vec<u32>` and `bumpalo::collections::Vec<'_, u32>` both
//! implement it, so callers with an arena (the lex pipeline) write
//! offsets directly into the arena — the legacy "heap-allocate then
//! memcpy into the arena" detour is gone. [`CountSink`] counts
//! pushes without storing, useful for capacity probes.
//!
//! ## Naive reference
//!
//! [`NaiveScanner`] is the brute-force `O(n × PHF)` walker — slowest
//! by design but the independent oracle every kernel cross-validates
//! against. Kept `pub` (under `#[doc(hidden)]`) so the integration
//! proptests and benches can reach it.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
use bumpalo::Bump;
#[cfg(feature = "std")]
use bumpalo::collections::Vec as BumpVec;

mod arch;
mod dispatch;
mod kernel;
mod naive;
mod trait_def;

pub use dispatch::BackendChoice;
pub use trait_def::{CountSink, OffsetSink};

#[doc(hidden)]
pub use naive::NaiveScanner;

/// Process-wide cached backend choice (std builds).
///
/// Runtime CPU-feature detection runs at most once per process and the
/// result caches in a `OnceLock`; later calls dispatch through the
/// cached enum without touching `is_x86_feature_detected!` again.
/// Lifting it here — rather than a function-local `static` buried in one
/// entry point — lets the hot path, the convenience entry, and `prewarm`
/// share the *same* lock, so warming is honest and no entry re-detects.
#[cfg(feature = "std")]
fn cached_choice() -> BackendChoice {
    use std::sync::OnceLock;
    static CHOICE: OnceLock<BackendChoice> = OnceLock::new();
    *CHOICE.get_or_init(BackendChoice::detect)
}

/// Force the one-time runtime CPU-feature detection now.
///
/// The first [`scan_offsets_in`] then dispatches through the
/// already-cached backend choice instead of paying
/// `is_x86_feature_detected!` on the hot path. Idempotent and
/// sub-microsecond.
///
/// `no_std` builds dispatch straight to `ScalarTeddy` with no `OnceLock`
/// to populate, so this is a documented no-op there.
#[cfg(feature = "std")]
pub fn prewarm() {
    let _ = cached_choice();
}

/// `no_std` no-op counterpart of [`prewarm`] — there is no `OnceLock` to
/// populate when runtime detection is unavailable.
#[cfg(not(feature = "std"))]
pub fn prewarm() {}

/// Scan `source` and return every trigger byte offset.
///
/// Convenience entry that allocates a fresh `Vec<u32>` and dispatches to
/// the host's fastest backend. Callers with a `bumpalo` arena should
/// reach for [`scan_offsets_in`] instead — it writes directly into the
/// arena without the heap roundtrip this entry pays for.
#[must_use]
pub fn scan_offsets(source: &str) -> alloc::vec::Vec<u32> {
    let mut sink = alloc::vec::Vec::new();
    #[cfg(feature = "std")]
    cached_choice().scan(source, &mut sink);
    #[cfg(not(feature = "std"))]
    BackendChoice::detect().scan(source, &mut sink);
    sink
}

/// Arena-backed variant: scan trigger byte offsets directly into a
/// caller-provided [`BumpVec<u32>`] living in the lex pipeline's
/// per-parse [`Bump`] arena. No heap allocation, no memcpy.
///
/// Backend selection uses the process-wide cached choice; the first
/// call (or an explicit [`prewarm`]) runs detection once, and every
/// later call dispatches through the cached enum.
#[cfg(feature = "std")]
#[must_use]
pub fn scan_offsets_in<'a>(source: &str, arena: &'a Bump) -> BumpVec<'a, u32> {
    let mut out = BumpVec::new_in(arena);
    cached_choice().scan(source, &mut out);
    out
}

/// `no_std` variant of [`scan_offsets_in`]. Without `std` no
/// runtime CPU detection is possible, so we always dispatch to
/// the always-available scalar Teddy kernel.
#[cfg(not(feature = "std"))]
#[must_use]
pub fn scan_offsets_in<'a>(source: &str, arena: &'a Bump) -> BumpVec<'a, u32> {
    let mut out = BumpVec::new_in(arena);
    BackendChoice::ScalarTeddy.scan(source, &mut out);
    out
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    /// On any modern x86_64 dev/CI host (~2026) the dispatcher
    /// should land on Teddy-AVX2. Asserting it directly catches the
    /// next-most-likely refactor mistake: silently downgrading the
    /// dispatcher to a slower backend by reordering the cfg branches.
    #[test]
    #[cfg(all(feature = "std", target_arch = "x86_64"))]
    fn dispatcher_picks_avx2_when_supported() {
        // AVX2 is universal on `x86_64-v3` and above — every
        // 2026-era CI runner ships it. If this assertion ever fires
        // it means `is_x86_feature_detected!("avx2")` returned false
        // even though AVX2 *should* be available, which is a real
        // regression worth investigating, not a flaky test.
        if std::is_x86_feature_detected!("avx2") {
            assert_eq!(BackendChoice::detect().name(), "teddy-avx2");
        }
    }

    #[test]
    fn dispatcher_returns_byte_identical_results() {
        // Whatever backend the dispatcher picks, it must agree with
        // the brute-force naive reference on a representative
        // mixed-Japanese sample (ruby, refmark, square brackets,
        // hash, corner brackets — 8 triggers).
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let dispatched = scan_offsets(s);
        let naive = NaiveScanner.scan_offsets(s);
        assert_eq!(dispatched, naive);
        assert_eq!(dispatched.len(), 8, "sample has 8 triggers");
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
}
