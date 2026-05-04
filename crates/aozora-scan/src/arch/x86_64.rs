//! x86_64 SIMD inner kernels for the Teddy outer driver.
//!
//! Two kernels ship:
//!
//! - [`Ssse3Kernel`] uses `pshufb` (`_mm_shuffle_epi8`) with 16-byte
//!   chunks. SSSE3 is the baseline for `x86_64-v2` (Haswell, 2013+),
//!   so every modern x86 host can run it.
//! - [`Avx2Kernel`] doubles the chunk to 32 bytes via
//!   `_mm256_shuffle_epi8`, broadcasting the 16-byte LUT into both
//!   128-bit lanes. AVX2 is mainstream from Haswell onward.
//!
//! ## Algorithm
//!
//! Both kernels run the same nibble-LUT lookup. The Teddy LUT is a
//! 16-entry `u16` table indexed by the lead byte's nibble; we split
//! each entry into a low byte and a high byte so the byte-wise
//! `pshufb` can index into a 16-byte vector. For each source byte:
//!
//! 1. Compute its hi and lo nibble (`v >> 4` and `v & 0x0F`),
//!    masked to the lower 4 bits of every byte position.
//! 2. Run four `pshufb`s — `LEAD_HI_LUT[hi]` and `LEAD_LO_LUT[lo]`,
//!    each split into low / high LUT byte planes.
//! 3. AND the hi LUT result with the lo LUT result, giving the
//!    candidate `u16` mask split across two byte vectors.
//! 4. Interleave the two byte vectors into a `u16` stream and store
//!    into the caller's `out` slice.
//!
//! The 4-shuffle pattern is the same shape Hyperscan's Teddy uses;
//! we just hand-roll it because `aho_corasick::packed::teddy` is
//! SSSE3-only and refuses to build on hosts without it.
//!
//! ## Verification
//!
//! Cross-validated by proptest in
//! `tests/property_backend_equiv.rs` against the
//! [`crate::NaiveScanner`] reference. Each kernel is exercised
//! end-to-end through the Teddy outer driver over
//! `aozora_fragment` / `pathological_aozora` /
//! `unicode_adversarial` strategies, with the runner also asserting
//! per-kernel byte-identical agreement.

#![allow(
    unsafe_code,
    reason = "x86_64 SIMD intrinsics are unsafe by Rust's safety model; \
              every block carries a SAFETY: comment naming the precondition"
)]

use core::arch::x86_64::{
    __m128i, __m256i, _mm_and_si128, _mm_loadu_si128, _mm_set1_epi8, _mm_shuffle_epi8,
    _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8, _mm256_and_si256,
    _mm256_broadcastsi128_si256, _mm256_loadu_si256, _mm256_permute2x128_si256, _mm256_set1_epi8,
    _mm256_shuffle_epi8, _mm256_srli_epi16, _mm256_storeu_si256, _mm256_unpackhi_epi8,
    _mm256_unpacklo_epi8,
};

use crate::kernel::teddy::{
    LEAD_HI_LUT_HI_BYTES, LEAD_HI_LUT_LO_BYTES, LEAD_LO_LUT_HI_BYTES, LEAD_LO_LUT_LO_BYTES,
    TeddyInner,
};

/// SSSE3 (16-byte chunk) inner kernel.
///
/// Caller MUST verify SSSE3 availability via
/// `is_x86_feature_detected!("ssse3")` before passing this kernel
/// to [`crate::kernel::teddy::teddy_outer`]. The dispatcher in
/// [`crate::dispatch`] does the check once at process start.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Ssse3Kernel;

impl TeddyInner for Ssse3Kernel {
    const CHUNK: usize = 16;

    #[inline]
    fn lead_mask_chunk(bytes: &[u8], out: &mut [u16]) {
        debug_assert_eq!(bytes.len(), Self::CHUNK);
        debug_assert_eq!(out.len(), Self::CHUNK);
        // SAFETY: caller guarantees SSSE3 availability per
        // `Ssse3Kernel`'s contract; debug_assert above pins the
        // slice lengths the unsafe code reads from / writes to.
        unsafe {
            lead_mask_chunk_ssse3(bytes, out);
        }
    }
}

#[target_feature(enable = "ssse3")]
unsafe fn lead_mask_chunk_ssse3(bytes: &[u8], out: &mut [u16]) {
    // SAFETY: precondition checked by debug_assert in caller —
    // bytes.len() >= 16 and out.len() >= 16. The function is
    // `unsafe fn` + `target_feature(ssse3)`; the intrinsics below
    // operate on 16-byte windows from those slices and on
    // statically-sized const LUTs whose pointers are well-aligned
    // for the unaligned-load intrinsics we use.
    unsafe {
        let v = _mm_loadu_si128(bytes.as_ptr().cast::<__m128i>());

        let mask_low = _mm_set1_epi8(0x0F);
        let lo_nibbles = _mm_and_si128(v, mask_low);
        // `srli_epi16` shifts u16 lanes right; the 4-bit mask above
        // strips the cross-byte spill so each byte holds its hi nibble.
        let hi_nibbles = _mm_and_si128(_mm_srli_epi16(v, 4), mask_low);

        let lead_hi_lut_lo = _mm_loadu_si128(LEAD_HI_LUT_LO_BYTES.as_ptr().cast());
        let lead_hi_lut_hi = _mm_loadu_si128(LEAD_HI_LUT_HI_BYTES.as_ptr().cast());
        let lead_lo_lut_lo = _mm_loadu_si128(LEAD_LO_LUT_LO_BYTES.as_ptr().cast());
        let lead_lo_lut_hi = _mm_loadu_si128(LEAD_LO_LUT_HI_BYTES.as_ptr().cast());

        // Look up LEAD_HI_LUT[hi] and LEAD_LO_LUT[lo], split across
        // low / high byte planes so the byte-wise shuffle can index them.
        let hi_lookup_lo = _mm_shuffle_epi8(lead_hi_lut_lo, hi_nibbles);
        let hi_lookup_hi = _mm_shuffle_epi8(lead_hi_lut_hi, hi_nibbles);
        let lo_lookup_lo = _mm_shuffle_epi8(lead_lo_lut_lo, lo_nibbles);
        let lo_lookup_hi = _mm_shuffle_epi8(lead_lo_lut_hi, lo_nibbles);

        // AND the two LUT results plane by plane: result_low_byte[i] =
        // hi_LUT_low[hi(b)] & lo_LUT_low[lo(b)] (and likewise for the
        // high byte plane).
        let mask_lo_byte = _mm_and_si128(hi_lookup_lo, lo_lookup_lo);
        let mask_hi_byte = _mm_and_si128(hi_lookup_hi, lo_lookup_hi);

        // Interleave the two byte planes into a u16 stream. After the
        // unpack, lane 0 holds positions 0..8 (16 bytes = 8 u16) and
        // lane 1 holds positions 8..16.
        let interleaved_lo = _mm_unpacklo_epi8(mask_lo_byte, mask_hi_byte);
        let interleaved_hi = _mm_unpackhi_epi8(mask_lo_byte, mask_hi_byte);

        _mm_storeu_si128(out.as_mut_ptr().cast::<__m128i>(), interleaved_lo);
        _mm_storeu_si128(out.as_mut_ptr().add(8).cast::<__m128i>(), interleaved_hi);
    }
}

/// AVX2 (32-byte chunk) inner kernel.
///
/// Caller MUST verify AVX2 availability via
/// `is_x86_feature_detected!("avx2")` before passing this kernel
/// to the outer driver.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Avx2Kernel;

impl TeddyInner for Avx2Kernel {
    const CHUNK: usize = 32;

    #[inline]
    fn lead_mask_chunk(bytes: &[u8], out: &mut [u16]) {
        debug_assert_eq!(bytes.len(), Self::CHUNK);
        debug_assert_eq!(out.len(), Self::CHUNK);
        // SAFETY: caller guarantees AVX2 availability per
        // `Avx2Kernel`'s contract; debug_assert above pins the slice
        // lengths the unsafe intrinsics read from / write to.
        unsafe {
            lead_mask_chunk_avx2(bytes, out);
        }
    }
}

#[target_feature(enable = "avx2")]
unsafe fn lead_mask_chunk_avx2(bytes: &[u8], out: &mut [u16]) {
    // SAFETY: precondition checked by debug_assert in caller —
    // bytes.len() >= 32 and out.len() >= 32. The function is
    // `unsafe fn` + `target_feature(avx2)`; intrinsics below
    // operate on 32-byte windows from those slices and on
    // statically-sized const LUTs whose pointers are well-aligned
    // for the unaligned-load intrinsics we use.
    unsafe {
        let v = _mm256_loadu_si256(bytes.as_ptr().cast::<__m256i>());

        let mask_low = _mm256_set1_epi8(0x0F);
        let lo_nibbles = _mm256_and_si256(v, mask_low);
        let hi_nibbles = _mm256_and_si256(_mm256_srli_epi16(v, 4), mask_low);

        // Broadcast each 16-byte LUT into both 128-bit lanes —
        // `_mm256_shuffle_epi8` is per-lane, so each lane needs its own
        // copy of the table.
        let lead_hi_lut_lo_128 = _mm_loadu_si128(LEAD_HI_LUT_LO_BYTES.as_ptr().cast());
        let lead_hi_lut_hi_128 = _mm_loadu_si128(LEAD_HI_LUT_HI_BYTES.as_ptr().cast());
        let lead_lo_lut_lo_128 = _mm_loadu_si128(LEAD_LO_LUT_LO_BYTES.as_ptr().cast());
        let lead_lo_lut_hi_128 = _mm_loadu_si128(LEAD_LO_LUT_HI_BYTES.as_ptr().cast());

        let lead_hi_lut_lo = _mm256_broadcastsi128_si256(lead_hi_lut_lo_128);
        let lead_hi_lut_hi = _mm256_broadcastsi128_si256(lead_hi_lut_hi_128);
        let lead_lo_lut_lo = _mm256_broadcastsi128_si256(lead_lo_lut_lo_128);
        let lead_lo_lut_hi = _mm256_broadcastsi128_si256(lead_lo_lut_hi_128);

        let hi_lookup_lo = _mm256_shuffle_epi8(lead_hi_lut_lo, hi_nibbles);
        let hi_lookup_hi = _mm256_shuffle_epi8(lead_hi_lut_hi, hi_nibbles);
        let lo_lookup_lo = _mm256_shuffle_epi8(lead_lo_lut_lo, lo_nibbles);
        let lo_lookup_hi = _mm256_shuffle_epi8(lead_lo_lut_hi, lo_nibbles);

        let mask_lo_byte = _mm256_and_si256(hi_lookup_lo, lo_lookup_lo);
        let mask_hi_byte = _mm256_and_si256(hi_lookup_hi, lo_lookup_hi);

        // Per-lane interleave: lane 0 of `interleaved_lo` carries the
        // first 8 u16 positions of lane 0 (i.e. source positions 0..8),
        // lane 1 of `interleaved_lo` carries the first 8 u16 positions
        // of lane 1 (i.e. source positions 16..24). To get the natural
        // [0..16, 16..32] order, permute the two lanes back into place.
        let interleaved_lo = _mm256_unpacklo_epi8(mask_lo_byte, mask_hi_byte);
        let interleaved_hi = _mm256_unpackhi_epi8(mask_lo_byte, mask_hi_byte);

        // permute2x128 control 0x20: low half from interleaved_lo lane 0
        //   + high half from interleaved_hi lane 0  → positions 0..16.
        // permute2x128 control 0x31: low half from interleaved_lo lane 1
        //   + high half from interleaved_hi lane 1  → positions 16..32.
        let positions_0_16 = _mm256_permute2x128_si256::<0x20>(interleaved_lo, interleaved_hi);
        let positions_16_32 = _mm256_permute2x128_si256::<0x31>(interleaved_lo, interleaved_hi);

        _mm256_storeu_si256(out.as_mut_ptr().cast::<__m256i>(), positions_0_16);
        _mm256_storeu_si256(out.as_mut_ptr().add(16).cast::<__m256i>(), positions_16_32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::teddy::{ScalarTeddyKernel, lead_mask};

    /// Helper: scan one 16-byte chunk through both Ssse3Kernel and
    /// ScalarTeddyKernel and compare the masks position-by-position.
    fn ssse3_chunk_matches_scalar(bytes: &[u8; 16]) {
        let mut out_simd = [0u16; 16];
        let mut out_scalar = [0u16; 16];
        Ssse3Kernel::lead_mask_chunk(bytes, &mut out_simd);
        ScalarTeddyKernel::lead_mask_chunk(bytes, &mut out_scalar);
        assert_eq!(out_simd, out_scalar, "ssse3 ≠ scalar for {bytes:02X?}");
    }

    fn avx2_chunk_matches_scalar(bytes: &[u8; 32]) {
        if !std::is_x86_feature_detected!("avx2") {
            return; // skip on hosts without AVX2
        }
        let mut out_simd = [0u16; 32];
        let mut out_scalar_lo = [0u16; 16];
        let mut out_scalar_hi = [0u16; 16];
        Avx2Kernel::lead_mask_chunk(bytes, &mut out_simd);
        let (lo, hi) = bytes.split_at(16);
        let lo_arr: [u8; 16] = lo.try_into().unwrap();
        let hi_arr: [u8; 16] = hi.try_into().unwrap();
        ScalarTeddyKernel::lead_mask_chunk(&lo_arr, &mut out_scalar_lo);
        ScalarTeddyKernel::lead_mask_chunk(&hi_arr, &mut out_scalar_hi);
        let mut expected = [0u16; 32];
        expected[..16].copy_from_slice(&out_scalar_lo);
        expected[16..].copy_from_slice(&out_scalar_hi);
        assert_eq!(out_simd, expected, "avx2 ≠ scalar for {bytes:02X?}");
    }

    #[test]
    fn ssse3_kernel_matches_scalar_on_canonical_lead_bytes() {
        if !std::is_x86_feature_detected!("ssse3") {
            return;
        }
        let mut bytes = [0u8; 16];
        for byte in 0u8..=255 {
            bytes.fill(byte);
            ssse3_chunk_matches_scalar(&bytes);
        }
    }

    #[test]
    fn ssse3_kernel_matches_scalar_on_per_position_lead() {
        if !std::is_x86_feature_detected!("ssse3") {
            return;
        }
        // Each position individually carries a canonical lead while
        // the rest of the chunk is filler ASCII; the SIMD path must
        // surface the lead at the right position.
        for lead in [0xE2u8, 0xE3, 0xEF] {
            for pos in 0..16 {
                let mut bytes = [b'a'; 16];
                bytes[pos] = lead;
                ssse3_chunk_matches_scalar(&bytes);
            }
        }
    }

    #[test]
    fn avx2_kernel_matches_scalar_on_canonical_lead_bytes() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let mut bytes = [0u8; 32];
        for byte in 0u8..=255 {
            bytes.fill(byte);
            avx2_chunk_matches_scalar(&bytes);
        }
    }

    #[test]
    fn avx2_kernel_matches_scalar_on_lead_bytes_at_each_position() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        for lead in [0xE2u8, 0xE3, 0xEF] {
            for pos in 0..32 {
                let mut bytes = [b'a'; 32];
                bytes[pos] = lead;
                avx2_chunk_matches_scalar(&bytes);
            }
        }
    }

    #[test]
    fn lead_mask_const_table_round_trips_through_byte_planes() {
        // The 4 byte-planes the SIMD kernels consume must reconstruct
        // the original 16-entry u16 LUTs they were derived from.
        use crate::kernel::teddy::{LEAD_HI_LUT, LEAD_LO_LUT};
        for i in 0..16 {
            let hi = u16::from(LEAD_HI_LUT_HI_BYTES[i]) << 8 | u16::from(LEAD_HI_LUT_LO_BYTES[i]);
            let lo = u16::from(LEAD_LO_LUT_HI_BYTES[i]) << 8 | u16::from(LEAD_LO_LUT_LO_BYTES[i]);
            assert_eq!(hi, LEAD_HI_LUT[i]);
            assert_eq!(lo, LEAD_LO_LUT[i]);
        }
        // And the scalar `lead_mask` round-trips trivially: every
        // canonical lead byte must surface at least one bucket bit.
        for byte in [0xE2, 0xE3, 0xEF] {
            assert_ne!(lead_mask(byte), 0);
        }
    }
}
