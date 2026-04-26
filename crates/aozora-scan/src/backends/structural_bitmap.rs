//! Structural-bitmap backend — simdjson-style two-byte filter.
//!
//! ## Algorithm
//!
//! Per 32-byte AVX2 chunk:
//!
//! 1. Load 32 source bytes into a YMM register.
//! 2. Build `mask_lead` — 32-bit bitmap, bit `i` set iff
//!    `byte[i] ∈ {0xE2, 0xE3, 0xEF}` (a trigger leading byte).
//! 3. Build `mask_mid`  — 32-bit bitmap, bit `i` set iff
//!    `byte[i] ∈ {0x80, 0xBC, 0xBD}` (a trigger middle byte).
//! 4. `mask_trigger = mask_lead & (mask_mid >> 1)` — bit `i` set iff
//!    position `i` is a leading byte AND position `i+1` is a middle
//!    byte. This is a *candidate* trigger trigram start.
//! 5. Iterate set bits via Kernighan's `mask &= mask - 1` and PHF-
//!    validate the 3-byte window starting at each candidate.
//!
//! At ~1.8 % trigger density (corpus measurement, T2.0), the
//! candidate density after the two-byte filter is low enough that
//! per-bit Kernighan extraction is faster than the BMI2 `_pext_u64`
//! batch-compaction simdjson uses. PEXT was therefore evaluated and
//! rejected for *this* workload — see ADR-0015.
//!
//! ## Citation
//!
//! - Langdale & Lemire, "Parsing Gigabytes of JSON per Second"
//!   (VLDB Journal 2019) — the structural-bitmap construction.
//!   simdjson uses *full-pattern* compares for JSON delimiters
//!   (single bytes); we do a two-byte filter (lead × middle) and
//!   verify via PHF. Same shape, lighter SIMD per chunk.
//!
//! ## Boundary handling
//!
//! Position 31 of a chunk may be a leading byte whose middle byte
//! sits at position 32 (the first byte of the *next* chunk). We
//! patch this in by checking `bytes[chunk_offset + 32]` whenever
//! `mask_lead` bit 31 is set. Cleaner than overlapping chunks.
//!
//! Tail (< 32 bytes after the last full chunk) hands off to the
//! same brute-force walker [`crate::NaiveScanner`] uses.

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
use aozora_spec::trigger::{TRIGGER_LEADING_BYTES, TRIGGER_MIDDLE_BYTES};

use crate::TriggerScanner;

/// Structural-bitmap [`TriggerScanner`].
///
/// Stateless — instantiate via the unit literal `StructuralBitmapScanner`.
/// Caller MUST verify AVX2 is available before invoking
/// `scan_offsets`; `crate::best_scanner` does that check.
#[derive(Debug, Clone, Copy, Default)]
pub struct StructuralBitmapScanner;

impl TriggerScanner for StructuralBitmapScanner {
    fn scan_offsets(&self, source: &str) -> Vec<u32> {
        // SAFETY: see module docstring + crate::best_scanner: this
        // method is only called once AVX2 has been runtime-detected.
        unsafe { scan_offsets_avx2(source.as_bytes()) }
    }
}

#[target_feature(enable = "avx2")]
unsafe fn scan_offsets_avx2(bytes: &[u8]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(bytes.len() / 56);

    // Splat the six needles into YMMs once, outside the loop.
    let lead0 = _mm256_set1_epi8(TRIGGER_LEADING_BYTES[0] as i8);
    let lead1 = _mm256_set1_epi8(TRIGGER_LEADING_BYTES[1] as i8);
    let lead2 = _mm256_set1_epi8(TRIGGER_LEADING_BYTES[2] as i8);
    let mid0 = _mm256_set1_epi8(TRIGGER_MIDDLE_BYTES[0] as i8);
    let mid1 = _mm256_set1_epi8(TRIGGER_MIDDLE_BYTES[1] as i8);
    let mid2 = _mm256_set1_epi8(TRIGGER_MIDDLE_BYTES[2] as i8);

    let mut chunk_offset = 0usize;
    while chunk_offset + 32 <= bytes.len() {
        // SAFETY: loop invariant guarantees a 32-byte window in bounds;
        // `_mm256_loadu_si256` is unaligned so alignment is irrelevant.
        let chunk: __m256i =
            unsafe { _mm256_loadu_si256(bytes.as_ptr().add(chunk_offset).cast::<__m256i>()) };

        // SAFETY: still inside the AVX2-feature-enabled outer fn.
        let mask_lead = unsafe { movemask3(chunk, lead0, lead1, lead2) };
        let mask_mid = unsafe { movemask3(chunk, mid0, mid1, mid2) };

        // Combine into the candidate-trigger mask.
        let mut mask_trigger = mask_lead & (mask_mid >> 1);

        // Boundary patch: bit 31 of mask_lead pairs with the FIRST
        // byte of the next chunk for the middle-byte test.
        if (mask_lead & (1u32 << 31)) != 0 && chunk_offset + 32 < bytes.len() {
            let next_byte = bytes[chunk_offset + 32];
            if next_byte == TRIGGER_MIDDLE_BYTES[0]
                || next_byte == TRIGGER_MIDDLE_BYTES[1]
                || next_byte == TRIGGER_MIDDLE_BYTES[2]
            {
                mask_trigger |= 1u32 << 31;
            }
        }

        // Kernighan-iterate the sparse mask. PHF validates each
        // candidate against the 11-entry trigram set.
        while mask_trigger != 0 {
            let bit = mask_trigger.trailing_zeros() as usize;
            let pos = chunk_offset + bit;
            // The 3-byte window must fit. A leading byte at
            // bytes.len()-2 or bytes.len()-1 cannot be a trigger.
            if pos + 3 <= bytes.len() {
                let window: [u8; 3] = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
                if classify_trigger_bytes(window).is_some() {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "lex pipeline asserts source ≤ u32::MAX upstream"
                    )]
                    out.push(pos as u32);
                }
            }
            mask_trigger &= mask_trigger - 1;
        }

        chunk_offset += 32;
    }

    // Tail (< 32 bytes remaining): brute-force PHF walk. Identical
    // shape to NaiveScanner but appends to `out` rather than
    // allocating its own Vec.
    while chunk_offset + 3 <= bytes.len() {
        let window = [
            bytes[chunk_offset],
            bytes[chunk_offset + 1],
            bytes[chunk_offset + 2],
        ];
        if classify_trigger_bytes(window).is_some() {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "lex pipeline asserts source ≤ u32::MAX upstream"
            )]
            out.push(chunk_offset as u32);
        }
        chunk_offset += 1;
    }

    out
}

#[target_feature(enable = "avx2")]
#[allow(
    clippy::cast_sign_loss,
    reason = "movemask returns a bit pattern; sign is irrelevant"
)]
unsafe fn movemask3(chunk: __m256i, n0: __m256i, n1: __m256i, n2: __m256i) -> u32 {
    let m0 = _mm256_cmpeq_epi8(chunk, n0);
    let m1 = _mm256_cmpeq_epi8(chunk, n1);
    let m2 = _mm256_cmpeq_epi8(chunk, n2);
    let combined = _mm256_or_si256(_mm256_or_si256(m0, m1), m2);
    _mm256_movemask_epi8(combined) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NaiveScanner;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;

    fn skip_if_no_avx2() -> bool {
        !std::is_x86_feature_detected!("avx2")
    }

    #[test]
    fn empty_input_yields_nothing() {
        if skip_if_no_avx2() {
            return;
        }
        assert!(StructuralBitmapScanner.scan_offsets("").is_empty());
    }

    #[test]
    fn finds_each_singleton_trigger() {
        if skip_if_no_avx2() {
            return;
        }
        // Pad each trigger so the AVX2 chunk fires at least once.
        for trigger in [
            "｜", "《", "》", "［", "］", "＃", "※", "〔", "〕", "「", "」",
        ] {
            let pad: String = "x".repeat(30);
            let s = format!("{pad}{trigger}tail");
            let got = StructuralBitmapScanner.scan_offsets(&s);
            let want = NaiveScanner.scan_offsets(&s);
            assert_eq!(got, want, "trigger={trigger}");
            assert_eq!(got, vec![30], "trigger={trigger}");
        }
    }

    #[test]
    fn matches_naive_at_chunk_boundaries() {
        if skip_if_no_avx2() {
            return;
        }
        // Boundary-sensitive: a trigger straddling positions 30..33
        // (chunk boundary 32) exercises the bit-31 carry path.
        for n in [0usize, 29, 30, 31, 32, 33, 63, 64, 65, 95, 96, 97] {
            let mut s = String::with_capacity(n + 16);
            for _ in 0..n {
                s.push('x');
            }
            s.push_str("｜tail");
            let got = StructuralBitmapScanner.scan_offsets(&s);
            let want = NaiveScanner.scan_offsets(&s);
            assert_eq!(got, want, "n={n}");
        }
    }

    #[test]
    fn skips_lookalikes_with_same_leading_byte() {
        if skip_if_no_avx2() {
            return;
        }
        let s = "あこんにちは、世界！".repeat(20);
        let got = StructuralBitmapScanner.scan_offsets(&s);
        let want = NaiveScanner.scan_offsets(&s);
        assert_eq!(got, want);
        assert!(got.is_empty(), "got {got:?}");
    }

    #[test]
    fn finds_dense_triggers() {
        if skip_if_no_avx2() {
            return;
        }
        let s = "《》《》《》《》《》《》《》《》《》《》"; // 60 bytes, 20 triggers
        let got = StructuralBitmapScanner.scan_offsets(s);
        let want = NaiveScanner.scan_offsets(s);
        assert_eq!(got, want);
        assert_eq!(got.len(), 20);
    }

    #[test]
    fn double_ruby_yields_two_adjacent_offsets() {
        if skip_if_no_avx2() {
            return;
        }
        let s = "《《X》》";
        let got = StructuralBitmapScanner.scan_offsets(s);
        let want = NaiveScanner.scan_offsets(s);
        assert_eq!(got, want);
        assert_eq!(got, vec![0, 3, 7, 10]);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::NaiveScanner;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Byte-identical against the NaiveScanner ground truth.
        /// Strategy spans 0..2400 codepoints (~0..16 KiB bytes) so
        /// the AVX2 chunk loop, the bit-31 carry path, and the
        /// scalar tail all see traffic.
        #[test]
        fn byte_identical_to_naive(
            s in r"(\PC|｜|《|》|［|］|＃|※|〔|〕|「|」){0,2400}",
        ) {
            if !std::is_x86_feature_detected!("avx2") {
                return Ok(());
            }
            let got = StructuralBitmapScanner.scan_offsets(&s);
            let want = NaiveScanner.scan_offsets(&s);
            prop_assert_eq!(got, want);
        }
    }
}
