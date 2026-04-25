//! AVX2 trigger scanner — `x86_64` only.
//!
//! ## Algorithm
//!
//! Each iteration loads 32 source bytes into a YMM register and
//! computes a 32-bit "candidate" mask whose set bits mark byte
//! positions that match one of the three trigger leading-bytes
//! `{0xE2, 0xE3, 0xEF}`. The mask is the bitwise OR of three
//! `_mm256_cmpeq_epi8` results, projected to a `u32` via
//! `_mm256_movemask_epi8`. We then peel set bits via
//! `trailing_zeros` + `mask &= mask - 1` (Brian Kernighan's bit
//! trick) and validate each candidate window through the const-PHF
//! lookup [`aozora_spec::classify_trigger_bytes`].
//!
//! ## Why no BMI2 PEXT yet
//!
//! simdjson uses BMI2 `pext` to compress a sparse bitmap into
//! contiguous candidate-index batches in a single instruction. For
//! the Aozora trigger density (< 1 candidate per 32-byte chunk on
//! average), the `trailing_zeros` loop is already at the lower bound
//! of what the candidate-validation step needs anyway. We can swap
//! in `pext` when a profile shows mask-walk overhead matters.
//!
//! ## Output equivalence
//!
//! The scalar property test in `src/scalar.rs` runs on the SAME
//! input as the AVX2 path here (via the `byte_identical_to_scalar`
//! proptest below) — every offset must match by-position with the
//! `memchr3` baseline.

#![allow(
    unsafe_code,
    reason = "AVX2 intrinsics are unsafe; quarantined to this file with #[target_feature]"
)]

use alloc::vec::Vec;
use core::arch::x86_64::{
    __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256,
    _mm256_set1_epi8,
};

use aozora_spec::classify_trigger_bytes;
use aozora_spec::trigger::TRIGGER_LEADING_BYTES;

use crate::TriggerScanner;

/// AVX2-driven [`TriggerScanner`].
///
/// Constructed via the unit-struct literal `Avx2Scanner`; stateless.
/// Caller MUST verify AVX2 is available on the host (via
/// `std::is_x86_feature_detected!("avx2")`) before invoking
/// [`Avx2Scanner::scan_offsets`] — calling without AVX2 is undefined
/// behaviour at the hardware level.
#[derive(Debug, Clone, Copy, Default)]
pub struct Avx2Scanner;

impl TriggerScanner for Avx2Scanner {
    fn scan_offsets(&self, source: &str) -> Vec<u32> {
        // SAFETY: callers reach this via `crate::best_scanner()`
        // which only returns `Avx2Scanner` after the
        // `is_x86_feature_detected!("avx2")` runtime check. Direct
        // construction is safe as a value (no unsafe ops happen
        // until `scan_offsets`); the contract is that this trait
        // method is called only when AVX2 is available.
        unsafe { scan_offsets_avx2(source.as_bytes()) }
    }
}

// Modern stdarch makes most AVX2 intrinsics callable directly when
// the enclosing fn carries `#[target_feature(enable = "avx2")]`.
// The exception is `_mm256_loadu_si256`, which takes a raw pointer
// and must therefore be wrapped in `unsafe { ... }` even inside an
// `unsafe fn`. `unsafe fn` itself remains because the function as a
// whole must only be entered when AVX2 is runtime-detected.
#[target_feature(enable = "avx2")]
unsafe fn scan_offsets_avx2(bytes: &[u8]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(bytes.len() / 200);

    // Splat each trigger leading byte across a YMM register once,
    // outside the loop. The compiler hoists these but being explicit
    // pins them in registers for the body's duration.
    let needle0 = _mm256_set1_epi8(TRIGGER_LEADING_BYTES[0] as i8);
    let needle1 = _mm256_set1_epi8(TRIGGER_LEADING_BYTES[1] as i8);
    let needle2 = _mm256_set1_epi8(TRIGGER_LEADING_BYTES[2] as i8);

    let mut chunk_offset = 0usize;
    while chunk_offset + 32 <= bytes.len() {
        // SAFETY: the loop invariant guarantees a 32-byte window is
        // in-bounds; `_mm256_loadu_si256` is the unaligned load
        // intrinsic so alignment is not required. Pointer arithmetic
        // requires `unsafe` even inside the AVX2-feature-enabled fn.
        let chunk: __m256i = unsafe {
            _mm256_loadu_si256(bytes.as_ptr().add(chunk_offset).cast::<__m256i>())
        };

        // Three byte-equal compares, OR'd together: mask bit `i` is
        // set iff byte `chunk_offset + i` equals one of the three
        // leading bytes.
        let m0 = _mm256_cmpeq_epi8(chunk, needle0);
        let m1 = _mm256_cmpeq_epi8(chunk, needle1);
        let m2 = _mm256_cmpeq_epi8(chunk, needle2);
        let combined = _mm256_or_si256(_mm256_or_si256(m0, m1), m2);

        // Project the 32-byte mask to a 32-bit movemask. Each set
        // bit corresponds to a byte position whose 3-byte window
        // is a *candidate* trigger.
        #[allow(
            clippy::cast_sign_loss,
            reason = "movemask returns a bit pattern; sign is irrelevant"
        )]
        let mut mask = _mm256_movemask_epi8(combined) as u32;

        while mask != 0 {
            let bit = mask.trailing_zeros() as usize;
            let pos = chunk_offset + bit;
            // The 3-byte window must be in-bounds. A leading byte
            // at the very last 1 or 2 bytes of the source cannot be
            // a real trigger, so we skip it.
            if pos + 3 <= bytes.len() {
                let window: [u8; 3] = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
                if classify_trigger_bytes(window).is_some() {
                    // The lex pipeline asserts source.len() ≤ u32::MAX upstream.
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "lex pipeline asserts source ≤ u32::MAX upstream"
                    )]
                    out.push(pos as u32);
                }
            }
            // Brian Kernighan's bit trick: clear lowest set bit.
            mask &= mask - 1;
        }

        chunk_offset += 32;
    }

    // Tail (< 32 bytes remaining): hand off to the scalar path so
    // the byte-identical equivalence is preserved without
    // duplicating logic.
    crate::scalar::scan_offsets_scalar_with_offset(bytes, chunk_offset, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScalarScanner;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;

    #[test]
    fn avx2_matches_scalar_on_simple_inputs() {
        if !std::is_x86_feature_detected!("avx2") {
            // AVX2 not available on this host; skip silently.
            return;
        }
        let cases = [
            "",
            "plain text",
            "あいうえお",
            "｜青梅《おうめ》",
            "abc｜def《ghi》jkl",
            "［＃ここから2字下げ］content［＃ここで字下げ終わり］",
            "※［＃「木＋吶のつくり」、第3水準1-85-54］",
        ];
        for case in cases {
            let avx2 = Avx2Scanner.scan_offsets(case);
            let scalar = ScalarScanner.scan_offsets(case);
            assert_eq!(avx2, scalar, "diverged on {case:?}");
        }
    }

    #[test]
    fn avx2_handles_inputs_at_chunk_boundaries() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        // Build inputs of length 31, 32, 33, 63, 64, 65 — exercising
        // the chunk boundary (32) and tail handover.
        for n in [31usize, 32, 33, 63, 64, 65, 95, 96, 97] {
            let mut s = String::with_capacity(n);
            while s.len() < n {
                s.push('x');
            }
            // Now sprinkle a trigger near the boundary.
            let with_trigger = format!("{}｜tail", &s[..n.min(s.len())]);
            let avx2 = Avx2Scanner.scan_offsets(&with_trigger);
            let scalar = ScalarScanner.scan_offsets(&with_trigger);
            assert_eq!(avx2, scalar, "diverged at boundary n={n}");
        }
    }

    #[test]
    fn avx2_finds_dense_triggers() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let s = "《》《》《》《》《》《》《》《》《》《》"; // 10 pairs = 60 bytes
        let avx2 = Avx2Scanner.scan_offsets(s);
        let scalar = ScalarScanner.scan_offsets(s);
        assert_eq!(avx2, scalar);
        assert_eq!(avx2.len(), 20);
    }

    #[test]
    fn avx2_skips_lookalikes_with_same_leading_byte() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        // 'あ' = E3 81 82, 'こ' = E3 81 93 share leading byte 0xE3
        // with several triggers but classify as None. Long input so
        // the AVX2 chunk loop fires.
        let s = "あこんにちは、世界！".repeat(20);
        let avx2 = Avx2Scanner.scan_offsets(&s);
        let scalar = ScalarScanner.scan_offsets(&s);
        assert_eq!(avx2, scalar);
        assert!(avx2.is_empty(), "got {avx2:?}");
    }

    #[test]
    fn avx2_identifies_each_singleton_trigger_at_chunk_seam() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        // Pad with 30 ASCII bytes so the trigger sits right at the
        // chunk boundary (offset 30 .. 33; chunk boundary is 32).
        let triggers = ["｜", "《", "》", "［", "］", "＃", "※", "〔", "〕", "「", "」"];
        for trigger in triggers {
            let pad: String = "x".repeat(30);
            let s = format!("{pad}{trigger}tail");
            let avx2 = Avx2Scanner.scan_offsets(&s);
            let scalar = ScalarScanner.scan_offsets(&s);
            assert_eq!(avx2, scalar, "diverged on trigger {trigger:?}");
            assert_eq!(avx2, vec![30], "expected trigger at offset 30 for {trigger:?}");
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::ScalarScanner;
    use proptest::prelude::*;

    proptest! {
        /// Byte-identical equivalence with the scalar path on
        /// arbitrary aozora-shaped input. This is the load-bearing
        /// property test for the AVX2 backend; a divergence here
        /// would silently corrupt the lex pipeline.
        #[test]
        fn byte_identical_to_scalar(
            s in r"(\PC|｜|《|》|［|］|＃|※|〔|〕|「|」){0,300}",
        ) {
            if !std::is_x86_feature_detected!("avx2") {
                return Ok(());
            }
            let avx2 = Avx2Scanner.scan_offsets(&s);
            let scalar = ScalarScanner.scan_offsets(&s);
            prop_assert_eq!(avx2, scalar);
        }
    }
}
