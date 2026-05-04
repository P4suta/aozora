//! wasm32 SIMD128 inner kernel for the Teddy outer driver.
//!
//! Same algorithm as `arch::x86_64::Ssse3Kernel` and
//! `arch::aarch64::NeonKernel`; only the SIMD instruction set
//! changes. The mapping mirrors the NEON port closely because both
//! ISAs offer a single 16-byte LUT shuffle (`vqtbl1q_u8` /
//! `i8x16_swizzle`) and a per-byte right shift; the only structural
//! difference is the byte-pair interleave, where WASM has no `vzip`
//! analogue and the standard idiom is a constant-index
//! `u8x16_shuffle`.
//!
//! WASM SIMD128 has been baseline-supported in every modern browser
//! engine (V8, SpiderMonkey, JavaScriptCore) since 2022, and in
//! `wasmtime` since 2.0. Compiling for `wasm32-*` targets with
//! `-C target-feature=+simd128` (the rustc default for Rust 1.85+
//! when targeting wasi/wasm browsers) statically enables the
//! intrinsics; no runtime feature detection is required, so the
//! dispatcher picks `TeddyWasm` whenever the binary was compiled
//! for wasm32.

#![allow(
    unsafe_code,
    reason = "wasm32 SIMD intrinsics are unsafe by Rust's safety model; \
              every block carries a SAFETY: comment naming the precondition"
)]

use core::arch::wasm32::{
    i8x16_splat, i8x16_swizzle, u8x16_shr, u8x16_shuffle, v128, v128_and, v128_load, v128_store,
};

use crate::kernel::teddy::{
    LEAD_HI_LUT_HI_BYTES, LEAD_HI_LUT_LO_BYTES, LEAD_LO_LUT_HI_BYTES, LEAD_LO_LUT_LO_BYTES,
    TeddyInner,
};

/// WASM SIMD128 16-byte chunk inner kernel.
///
/// Caller does not need to verify SIMD availability — the
/// `wasm32-*` targets we compile for mandate `+simd128`, so this
/// kernel is always safe to dispatch when `cfg(target_arch =
/// "wasm32")` is active.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WasmKernel;

impl TeddyInner for WasmKernel {
    const CHUNK: usize = 16;

    #[inline]
    fn lead_mask_chunk(bytes: &[u8], out: &mut [u16]) {
        debug_assert_eq!(bytes.len(), Self::CHUNK);
        debug_assert_eq!(out.len(), Self::CHUNK);
        // SAFETY: the wasm32 SIMD intrinsics below are `unsafe fn`
        // because they assume the host engine supports SIMD128 —
        // already required by the build profile that compiles
        // `target_arch = "wasm32"` with `+simd128`. The unaligned
        // 16-byte loads / stores have no alignment requirement.
        // `debug_assert` above pins the slice lengths.
        unsafe {
            lead_mask_chunk_wasm(bytes, out);
        }
    }
}

#[target_feature(enable = "simd128")]
unsafe fn lead_mask_chunk_wasm(bytes: &[u8], out: &mut [u16]) {
    // SAFETY: precondition checked by debug_assert in caller —
    // bytes.len() >= 16 and out.len() >= 16. Pointer-cast loads
    // / stores read 16 bytes per call.
    unsafe {
        let v = v128_load(bytes.as_ptr().cast::<v128>());

        let mask_low = i8x16_splat(0x0F);
        let lo_nibbles = v128_and(v, mask_low);
        // `u8x16_shr` is logical; the upper nibble zero-fills, so
        // no follow-up `& 0x0F` is needed.
        let hi_nibbles = u8x16_shr(v, 4);

        let lead_hi_lut_lo = v128_load(LEAD_HI_LUT_LO_BYTES.as_ptr().cast::<v128>());
        let lead_hi_lut_hi = v128_load(LEAD_HI_LUT_HI_BYTES.as_ptr().cast::<v128>());
        let lead_lo_lut_lo = v128_load(LEAD_LO_LUT_LO_BYTES.as_ptr().cast::<v128>());
        let lead_lo_lut_hi = v128_load(LEAD_LO_LUT_HI_BYTES.as_ptr().cast::<v128>());

        // `i8x16_swizzle` returns 0 for any index >= 16, so out-of-
        // range nibbles never alias into the LUT (same property as
        // NEON's `vqtbl1q_u8`).
        let hi_lookup_lo = i8x16_swizzle(lead_hi_lut_lo, hi_nibbles);
        let hi_lookup_hi = i8x16_swizzle(lead_hi_lut_hi, hi_nibbles);
        let lo_lookup_lo = i8x16_swizzle(lead_lo_lut_lo, lo_nibbles);
        let lo_lookup_hi = i8x16_swizzle(lead_lo_lut_hi, lo_nibbles);

        let mask_lo_byte = v128_and(hi_lookup_lo, lo_lookup_lo);
        let mask_hi_byte = v128_and(hi_lookup_hi, lo_lookup_hi);

        // WASM SIMD has no `vzip` analogue; emulate the NEON
        // interleave with a constant-index `u8x16_shuffle`. The
        // index list `[0, 16, 1, 17, ...]` builds a vector where
        // even bytes come from `mask_lo_byte` and odd bytes come
        // from `mask_hi_byte`, i.e. the source positions' u16
        // candidate masks in source order.
        let interleaved_lo = u8x16_shuffle::<0, 16, 1, 17, 2, 18, 3, 19, 4, 20, 5, 21, 6, 22, 7, 23>(
            mask_lo_byte,
            mask_hi_byte,
        );
        let interleaved_hi =
            u8x16_shuffle::<8, 24, 9, 25, 10, 26, 11, 27, 12, 28, 13, 29, 14, 30, 15, 31>(
                mask_lo_byte,
                mask_hi_byte,
            );

        v128_store(out.as_mut_ptr().cast::<v128>(), interleaved_lo);
        v128_store(out.as_mut_ptr().add(8).cast::<v128>(), interleaved_hi);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::teddy::ScalarTeddyKernel;

    fn wasm_chunk_matches_scalar(bytes: &[u8; 16]) {
        let mut out_simd = [0u16; 16];
        let mut out_scalar = [0u16; 16];
        WasmKernel::lead_mask_chunk(bytes, &mut out_simd);
        ScalarTeddyKernel::lead_mask_chunk(bytes, &mut out_scalar);
        assert_eq!(out_simd, out_scalar, "wasm ≠ scalar for {bytes:02X?}");
    }

    #[test]
    fn wasm_kernel_matches_scalar_on_canonical_lead_bytes() {
        let mut bytes = [0u8; 16];
        for byte in 0u8..=255 {
            bytes.fill(byte);
            wasm_chunk_matches_scalar(&bytes);
        }
    }

    #[test]
    fn wasm_kernel_matches_scalar_on_per_position_lead() {
        for lead in [0xE2u8, 0xE3, 0xEF] {
            for pos in 0..16 {
                let mut bytes = [b'a'; 16];
                bytes[pos] = lead;
                wasm_chunk_matches_scalar(&bytes);
            }
        }
    }
}
