//! SIMD-friendly trigger-byte scanner for the Aozora notation lexer.
//!
//! Phase 1 of the legacy [`aozora_lexer`] tokeniser walked the source
//! one Unicode scalar at a time, calling `match` on every character to
//! decide whether it was a trigger marker. Per the corpus profile
//! (2026-04-25), trigger characters appear at < 0.5% density in real
//! Aozora text — so 99.5% of the work was rejecting non-trigger
//! characters.
//!
//! This crate replaces that hot path with a **bulk byte scan** that
//! never decodes UTF-8, working entirely on the raw `&[u8]` view of
//! the source. Every Aozora trigger character (`｜《》［］＃※〔〕「」`)
//! is a 3-byte UTF-8 sequence whose leading byte falls in the
//! 3-element set [`aozora_spec::trigger::TRIGGER_LEADING_BYTES`] =
//! `{0xE2, 0xE3, 0xEF}`. We use [`memchr::memchr3`] to skip ahead to
//! the next candidate, then validate the 3-byte window via
//! [`aozora_spec::classify_trigger_bytes`] (a constant `phf::Map` —
//! see Innovation I-9 of the 0.2.0 plan).
//!
//! ## Design (current ship + future SIMD)
//!
//! - [`TriggerScanner`] is a trait so multiple backends can coexist.
//! - [`ScalarScanner`] is the always-available implementation built on
//!   `memchr::memchr3`. Internally `memchr` already dispatches to AVX2
//!   on `x86_64` / NEON on aarch64, giving us cache-friendly bulk
//!   skipping without any `unsafe` of our own.
//! - Future Move 2 commits will add `Avx2Scanner` / `NeonScanner` /
//!   `WasmSimdScanner` that build a "structural bitmap" (1 bit per
//!   source byte indicating "candidate trigger here?") and use BMI2
//!   `pext` to extract bit positions in batches — the simdjson-style
//!   Innovation I-1 of the 0.2.0 plan. Those backends fit behind the
//!   same trait so the lex layer never has to know which one it's
//!   using.
//!
//! ## Output shape
//!
//! Scanning produces a sorted list of **byte offsets** at which a
//! trigger character begins. The lex driver walks the offsets, calls
//! [`aozora_spec::classify_trigger_bytes`] on the 3-byte window at
//! each one, and weaves them with the surrounding plain text. Double
//! triggers (`《《`, `》》`) are detected at the lex layer by adjacent
//! single-trigger offsets, not here.

#![forbid(unsafe_code)]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

mod scalar;

pub use scalar::ScalarScanner;

/// A backend that finds trigger-byte candidate positions in a UTF-8
/// source buffer.
///
/// Implementations are stateless; instantiate one and reuse it across
/// scans. The trait is `dyn`-compatible (no generic methods) so the
/// lex layer can hold a `&'static dyn TriggerScanner` selected at
/// runtime via CPU feature detection.
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
/// Today this is always [`ScalarScanner`] (which already vectorises
/// internally via `memchr::memchr3`). Future commits will add SIMD
/// backends and runtime CPU dispatch (`x86_64` AVX2 vs SSE2; aarch64
/// NEON; wasm32 simd128); this function's signature is the contract
/// the lex layer holds onto.
#[must_use]
pub fn best_scanner() -> &'static dyn TriggerScanner {
    &ScalarScanner
}
