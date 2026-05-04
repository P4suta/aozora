//! aarch64 NEON inner kernel for the Teddy outer driver.
//!
//! The algorithm mirrors `arch::x86_64::Ssse3Kernel` exactly — the
//! Teddy bucket logic in [`crate::kernel::teddy`] doesn't change
//! per architecture; only the SIMD instruction set used to compute
//! the per-byte lookups does. Mapping each x86 intrinsic to its
//! NEON equivalent:
//!
//! | x86 SSSE3              | aarch64 NEON         | Role                                |
//! |------------------------|----------------------|-------------------------------------|
//! | `_mm_loadu_si128`      | `vld1q_u8`           | 16-byte load                        |
//! | `_mm_set1_epi8`        | `vdupq_n_u8`         | broadcast scalar                    |
//! | `_mm_and_si128`        | `vandq_u8`           | bitwise AND                         |
//! | `_mm_srli_epi16(_, 4)` | `vshrq_n_u8::<4>`    | per-byte right shift (NEON is u8)   |
//! | `_mm_shuffle_epi8`     | `vqtbl1q_u8`         | 16-byte LUT shuffle                 |
//! | `_mm_unpacklo/hi_epi8` | `vzip1q_u8 / vzip2q_u8` | byte-pair interleave             |
//! | `_mm_storeu_si128`     | `vst1q_u16` (after reinterpret) | 16-byte store           |
//!
//! Two improvements over the x86 mapping fall out of NEON's API:
//!
//! 1. `vshrq_n_u8::<4>` shifts each byte lane independently and
//!    zero-fills the high nibble, so the explicit `& 0x0F` mask the
//!    SSSE3 path needs after `_mm_srli_epi16` is unnecessary.
//! 2. `vqtbl1q_u8` returns 0 for any out-of-range index, so we
//!    never need a bounds-clearing AND before the shuffle either.
//!
//! Every aarch64 host has NEON on by default (the ABI mandates it),
//! so no runtime feature detection is required — the dispatcher
//! picks `TeddyNeon` whenever the binary was compiled for aarch64.

#![allow(
    unsafe_code,
    reason = "aarch64 NEON intrinsics are unsafe by Rust's safety model; \
              every block carries a SAFETY: comment naming the precondition"
)]

use core::arch::aarch64::{
    vandq_u8, vdupq_n_u8, vld1q_u8, vqtbl1q_u8, vreinterpretq_u16_u8, vshrq_n_u8, vst1q_u16,
    vzip1q_u8, vzip2q_u8,
};

use crate::kernel::teddy::{
    LEAD_HI_LUT_HI_BYTES, LEAD_HI_LUT_LO_BYTES, LEAD_LO_LUT_HI_BYTES, LEAD_LO_LUT_LO_BYTES,
    TeddyInner,
};

/// NEON 16-byte chunk inner kernel.
///
/// Caller does not need to verify NEON availability — the aarch64
/// ABI mandates NEON on every conformant CPU, so this kernel is
/// always safe to dispatch when `cfg(target_arch = "aarch64")`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NeonKernel;

impl TeddyInner for NeonKernel {
    const CHUNK: usize = 16;

    #[inline]
    fn lead_mask_chunk(bytes: &[u8], out: &mut [u16]) {
        debug_assert_eq!(bytes.len(), Self::CHUNK);
        debug_assert_eq!(out.len(), Self::CHUNK);
        // SAFETY: NEON is mandated by the aarch64 ABI; debug_assert
        // above pins the slice lengths the unsafe intrinsics rely
        // on. The unaligned-load / unaligned-store NEON intrinsics
        // we use have no alignment requirement on their operands.
        unsafe {
            lead_mask_chunk_neon(bytes, out);
        }
    }
}

#[target_feature(enable = "neon")]
unsafe fn lead_mask_chunk_neon(bytes: &[u8], out: &mut [u16]) {
    // SAFETY: precondition checked by debug_assert in caller —
    // bytes.len() >= 16 and out.len() >= 16. The function is
    // `unsafe fn` + `target_feature(neon)`; intrinsics below
    // operate on 16-byte windows from those slices.
    unsafe {
        let v = vld1q_u8(bytes.as_ptr());

        let mask_low = vdupq_n_u8(0x0F);
        let lo_nibbles = vandq_u8(v, mask_low);
        // `vshrq_n_u8::<4>` zero-fills the high nibble; no extra
        // mask needed (unlike the SSSE3 path).
        let hi_nibbles = vshrq_n_u8::<4>(v);

        let lead_hi_lut_lo = vld1q_u8(LEAD_HI_LUT_LO_BYTES.as_ptr());
        let lead_hi_lut_hi = vld1q_u8(LEAD_HI_LUT_HI_BYTES.as_ptr());
        let lead_lo_lut_lo = vld1q_u8(LEAD_LO_LUT_LO_BYTES.as_ptr());
        let lead_lo_lut_hi = vld1q_u8(LEAD_LO_LUT_HI_BYTES.as_ptr());

        let hi_lookup_lo = vqtbl1q_u8(lead_hi_lut_lo, hi_nibbles);
        let hi_lookup_hi = vqtbl1q_u8(lead_hi_lut_hi, hi_nibbles);
        let lo_lookup_lo = vqtbl1q_u8(lead_lo_lut_lo, lo_nibbles);
        let lo_lookup_hi = vqtbl1q_u8(lead_lo_lut_hi, lo_nibbles);

        let mask_lo_byte = vandq_u8(hi_lookup_lo, lo_lookup_lo);
        let mask_hi_byte = vandq_u8(hi_lookup_hi, lo_lookup_hi);

        // `vzip1q_u8` / `vzip2q_u8` interleave the two byte planes:
        // vzip1 -> [lo[0], hi[0], lo[1], hi[1], ..., lo[7], hi[7]]
        //          (= 8 u16, source positions 0..8)
        // vzip2 -> [lo[8], hi[8], ...]
        //          (= 8 u16, source positions 8..16)
        let interleaved_lo = vzip1q_u8(mask_lo_byte, mask_hi_byte);
        let interleaved_hi = vzip2q_u8(mask_lo_byte, mask_hi_byte);

        vst1q_u16(out.as_mut_ptr(), vreinterpretq_u16_u8(interleaved_lo));
        vst1q_u16(
            out.as_mut_ptr().add(8),
            vreinterpretq_u16_u8(interleaved_hi),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::teddy::ScalarTeddyKernel;

    /// Helper: scan one 16-byte chunk through both NeonKernel and
    /// ScalarTeddyKernel and compare the masks position-by-position.
    fn neon_chunk_matches_scalar(bytes: &[u8; 16]) {
        let mut out_simd = [0u16; 16];
        let mut out_scalar = [0u16; 16];
        NeonKernel::lead_mask_chunk(bytes, &mut out_simd);
        ScalarTeddyKernel::lead_mask_chunk(bytes, &mut out_scalar);
        assert_eq!(out_simd, out_scalar, "neon ≠ scalar for {bytes:02X?}");
    }

    #[test]
    fn neon_kernel_matches_scalar_on_canonical_lead_bytes() {
        let mut bytes = [0u8; 16];
        for byte in 0u8..=255 {
            bytes.fill(byte);
            neon_chunk_matches_scalar(&bytes);
        }
    }

    #[test]
    fn neon_kernel_matches_scalar_on_per_position_lead() {
        // Each position individually carries a canonical lead while
        // the rest of the chunk is filler ASCII.
        for lead in [0xE2u8, 0xE3, 0xEF] {
            for pos in 0..16 {
                let mut bytes = [b'a'; 16];
                bytes[pos] = lead;
                neon_chunk_matches_scalar(&bytes);
            }
        }
    }
}
