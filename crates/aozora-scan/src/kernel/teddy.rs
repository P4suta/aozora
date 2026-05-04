//! Hand-rolled Teddy-style trigger-trigram matcher.
//!
//! Background: the canonical Teddy is the small-string multi-pattern
//! algorithm from Intel's [Hyperscan]; the BurntSushi port lives in
//! `aho_corasick::packed::teddy`. The aho-corasick implementation is
//! SSSE3-only — it never compiles a NEON or WASM SIMD variant, and it
//! decides at construction time whether to build at all. Both
//! constraints conflict with the goals here: aozora ships portable
//! SIMD ports, and an algorithm that can refuse to build forces a
//! production fallback path that we'd prefer not to maintain.
//!
//! What this module ships:
//!
//! - **Const-built nibble LUTs** (`LEAD_HI_LUT`, `LEAD_LO_LUT`) keyed
//!   on the leading byte of every trigram. Each LUT entry packs the
//!   16-bit `1 << bucket` bitmap of every trigger pattern whose lead
//!   byte hi/lo nibble equals the index.
//! - **`TeddyInner`** — a trait every per-platform inner kernel
//!   implements. The trait method is "given a `CHUNK`-byte window,
//!   write the per-byte lead candidate masks into the caller's
//!   slice." That is the entire SIMD payload, ~30 LOC per platform.
//! - **`teddy_outer`** — the platform-agnostic outer driver. Walks
//!   the source in `CHUNK`-sized chunks, calls the inner kernel,
//!   and verifies each candidate position against the canonical
//!   trigram table. Verification is exact: a `1`-bit on lead-byte
//!   match still requires confirming the full 3-byte trigram, so
//!   false positives never escape into the sink.
//! - **`ScalarTeddyKernel`** — pure-Rust SIMD-free reference
//!   implementation. Used as the proptest oracle for SIMD ports and
//!   as the `no_std` last-resort dispatch target. The four byte
//!   trigram count keeps the candidate density low enough that the
//!   scalar kernel still beats the existing DFA fallback on
//!   throughput.
//!
//! ## Why bucket = pattern index (no Hyperscan-style packing)
//!
//! The trigger set is exactly 11 patterns, well under the 16-bit mask
//! width. Assigning bucket `i` to pattern `i` (1:1 instead of the
//! Hyperscan-style "fat finger" packing of multiple patterns into the
//! same bucket) makes the LUT construction a one-liner per pattern
//! and eliminates the bucket-collision verification step that
//! `aho-corasick`'s Teddy spends LOC on. The cost — wasted bits at
//! mask positions 11..16 — is invisible at runtime; both 16-bit AND /
//! `trailing_zeros` cost the same regardless of how many bits
//! actually carry data.
//!
//! [Hyperscan]: https://github.com/intel/hyperscan

use aozora_spec::trigger::ALL_TRIGGER_TRIGRAMS;

use crate::trait_def::OffsetSink;

/// Number of trigger trigrams the matcher recognises.
const NUM_PATTERNS: usize = 11;

const _: () = {
    assert!(
        ALL_TRIGGER_TRIGRAMS.len() == NUM_PATTERNS,
        "Teddy bucket layout is sized for exactly the 11 Aozora trigger trigrams; \
         updating this constant requires regenerating the LUTs",
    );
};

const fn build_lead_lut(nibble_high: bool) -> [u16; 16] {
    let mut lut = [0u16; 16];
    let mut i = 0;
    while i < NUM_PATTERNS {
        let lead = ALL_TRIGGER_TRIGRAMS[i][0];
        let n = if nibble_high { lead >> 4 } else { lead & 0x0F };
        // Each pattern occupies exactly one bucket bit; bucket i
        // carries pattern i. Indexing in `ALL_TRIGGER_TRIGRAMS`
        // doubles as the bucket index.
        lut[n as usize] |= 1u16 << i;
        i += 1;
    }
    lut
}

/// Per-pattern bucket bitmap of patterns whose lead byte's hi
/// nibble equals the index.
pub(crate) const LEAD_HI_LUT: [u16; 16] = build_lead_lut(true);

/// Per-pattern bucket bitmap of patterns whose lead byte's lo
/// nibble equals the index.
pub(crate) const LEAD_LO_LUT: [u16; 16] = build_lead_lut(false);

/// Split a 16-entry `u16` LUT into low / high byte planes so a
/// byte-wise SIMD shuffle (pshufb / vqtbl1q_u8 / i8x16_swizzle)
/// can index it. Each platform's inner kernel runs four shuffles
/// (LEAD_HI low / high byte, LEAD_LO low / high byte) and ANDs
/// the results plane-by-plane.
const fn lut_byte_plane(lut: [u16; 16], high_byte: bool) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = if high_byte {
            (lut[i] >> 8) as u8
        } else {
            (lut[i] & 0xFF) as u8
        };
        i += 1;
    }
    out
}

/// Low byte of every entry in [`LEAD_HI_LUT`], packed into a
/// 16-byte array so a byte-wise SIMD shuffle can index it.
pub(crate) const LEAD_HI_LUT_LO_BYTES: [u8; 16] = lut_byte_plane(LEAD_HI_LUT, false);
/// High byte of every entry in [`LEAD_HI_LUT`].
pub(crate) const LEAD_HI_LUT_HI_BYTES: [u8; 16] = lut_byte_plane(LEAD_HI_LUT, true);
/// Low byte of every entry in [`LEAD_LO_LUT`].
pub(crate) const LEAD_LO_LUT_LO_BYTES: [u8; 16] = lut_byte_plane(LEAD_LO_LUT, false);
/// High byte of every entry in [`LEAD_LO_LUT`].
pub(crate) const LEAD_LO_LUT_HI_BYTES: [u8; 16] = lut_byte_plane(LEAD_LO_LUT, true);

/// Per-byte lead-candidate bitmap for a single source byte.
///
/// `LEAD_HI[hi(b)] & LEAD_LO[lo(b)]` returns the bitmap of patterns
/// whose lead byte equals `b`. With 1:1 bucket packing this is also
/// the candidate set for "trigram starts at this position", subject
/// to the trigram body matching, which the outer driver verifies.
#[inline]
pub(crate) const fn lead_mask(byte: u8) -> u16 {
    LEAD_HI_LUT[(byte >> 4) as usize] & LEAD_LO_LUT[(byte & 0x0F) as usize]
}

/// Verify that the trigram starting at the given byte triple matches
/// one of the canonical trigger patterns. The candidate mask only
/// pins the lead byte; the body bytes still have to confirm.
#[inline]
fn verify_trigram(b0: u8, b1: u8, b2: u8) -> bool {
    let mut i = 0;
    while i < NUM_PATTERNS {
        let p = &ALL_TRIGGER_TRIGRAMS[i];
        if p[0] == b0 && p[1] == b1 && p[2] == b2 {
            return true;
        }
        i += 1;
    }
    false
}

/// Per-platform inner-loop interface.
///
/// Each kernel scans a fixed-size window of source bytes and writes
/// the per-byte lead-candidate bitmap into the caller's output slice.
/// Production SIMD impls (`Ssse3Kernel`, `Avx2Kernel`, `NeonKernel`,
/// `WasmKernel`) plug in via the same trait so the outer driver
/// stays platform-agnostic.
pub(crate) trait TeddyInner {
    /// Window size in bytes. Must be a power of two and at most 32
    /// (the upper bound corresponds to the AVX2 256-bit register).
    const CHUNK: usize;

    /// Compute lead-candidate masks for `bytes` and write them into
    /// `out`. Both slices have length `CHUNK`. Implementations may
    /// assume the lengths and skip bounds checking inside the loop.
    fn lead_mask_chunk(bytes: &[u8], out: &mut [u16]);
}

/// Pure-Rust reference kernel.
///
/// Walks the chunk byte-by-byte, indexing the const LUTs once per
/// byte. The compiler converts the two `[u16; 16]` LUTs into a
/// 32-byte read-only data object that fits in a single cache line,
/// so even this scalar form is cache-friendly.
pub(crate) struct ScalarTeddyKernel;

impl TeddyInner for ScalarTeddyKernel {
    const CHUNK: usize = 16;

    #[inline]
    fn lead_mask_chunk(bytes: &[u8], out: &mut [u16]) {
        debug_assert_eq!(bytes.len(), Self::CHUNK);
        debug_assert_eq!(out.len(), Self::CHUNK);
        for (slot, &byte) in out.iter_mut().zip(bytes.iter()) {
            *slot = lead_mask(byte);
        }
    }
}

/// Platform-agnostic outer driver.
///
/// Walks `source` in `I::CHUNK`-byte windows, calls the inner kernel
/// to compute lead masks, and verifies every candidate position
/// against the canonical trigram table. Trailing bytes that don't
/// fit a full chunk are scanned with the scalar [`lead_mask`] +
/// [`verify_trigram`] path — three tail bytes are below any SIMD
/// register width, so a scalar tail loop is the simplest and the
/// most-correct shape.
pub(crate) fn teddy_outer<I: TeddyInner, S: OffsetSink>(source: &str, sink: &mut S) {
    const TRIGRAM_LEN: usize = 3;
    let bytes = source.as_bytes();
    if bytes.len() < TRIGRAM_LEN {
        return;
    }

    // Stack scratch sized for the largest possible CHUNK (AVX2 = 32);
    // SSSE3 / NEON / WASM kernels write into the front 16 entries.
    let mut lead_buf = [0u16; 32];

    let chunk = I::CHUNK;
    let bound = bytes.len().saturating_sub(TRIGRAM_LEN - 1);

    let mut i = 0;
    while i + chunk <= bytes.len() {
        I::lead_mask_chunk(&bytes[i..i + chunk], &mut lead_buf[..chunk]);
        // The lead mask covers `chunk` byte positions; only those
        // whose trigram body lies within `bound` can match.
        let limit = chunk.min(bound.saturating_sub(i));
        for (k, &mask) in lead_buf[..limit].iter().enumerate() {
            if mask != 0 {
                let pos = i + k;
                if verify_trigram(bytes[pos], bytes[pos + 1], bytes[pos + 2]) {
                    let offset =
                        u32::try_from(pos).expect("source longer than u32::MAX is unsupported");
                    sink.push(offset);
                }
            }
        }
        i += chunk;
    }

    // Scalar tail for the residual `<chunk` bytes. Iterate up to the
    // last position where a full trigram still fits.
    while i < bound {
        if lead_mask(bytes[i]) != 0 && verify_trigram(bytes[i], bytes[i + 1], bytes[i + 2]) {
            let offset = u32::try_from(i).expect("source longer than u32::MAX is unsupported");
            sink.push(offset);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naive::NaiveScanner;
    use alloc::vec::Vec;

    /// Helper: run the scalar Teddy kernel against `source` and
    /// collect the offsets into a `Vec` for direct comparison.
    fn scalar_teddy_offsets(source: &str) -> Vec<u32> {
        let mut sink: Vec<u32> = Vec::new();
        teddy_outer::<ScalarTeddyKernel, _>(source, &mut sink);
        sink
    }

    #[test]
    fn lead_lut_classifies_every_canonical_lead_byte() {
        // Every lead byte of a real trigger has a non-zero lead
        // mask; bytes outside the canonical lead set produce zero.
        for trigram in &ALL_TRIGGER_TRIGRAMS {
            assert_ne!(
                lead_mask(trigram[0]),
                0,
                "canonical lead byte {:02X} produced zero lead mask",
                trigram[0],
            );
        }
        for byte in 0u8..=255 {
            let canonical = ALL_TRIGGER_TRIGRAMS.iter().any(|t| t[0] == byte);
            if !canonical {
                assert_eq!(
                    lead_mask(byte),
                    0,
                    "non-canonical lead byte {byte:02X} produced non-zero lead mask",
                );
            }
        }
    }

    #[test]
    fn lead_lut_bucket_bits_are_unique_per_pattern() {
        // Each pattern occupies one bucket bit; lead masks for two
        // distinct lead bytes must not share bucket bits.
        for (i, ti) in ALL_TRIGGER_TRIGRAMS.iter().enumerate() {
            let mi = 1u16 << i;
            assert_eq!(
                lead_mask(ti[0]) & mi,
                mi,
                "lead mask for pattern {i} did not include its own bucket bit",
            );
        }
    }

    #[test]
    fn empty_source_yields_no_offsets() {
        assert_eq!(scalar_teddy_offsets(""), Vec::<u32>::new());
    }

    #[test]
    fn ascii_only_source_yields_no_offsets() {
        let s = "the quick brown fox jumps over the lazy dog 1234567890";
        assert_eq!(scalar_teddy_offsets(s), Vec::<u32>::new());
    }

    #[test]
    fn finds_every_canonical_trigger_in_isolation() {
        for trigram in &ALL_TRIGGER_TRIGRAMS {
            let s = core::str::from_utf8(trigram).expect("canonical trigram is valid UTF-8");
            assert_eq!(
                scalar_teddy_offsets(s),
                Vec::from([0u32]),
                "scalar Teddy missed canonical trigram {trigram:02X?}",
            );
        }
    }

    #[test]
    fn matches_naive_on_handcrafted_sample() {
        // 8 triggers in a representative mixed-Japanese fragment.
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let teddy = scalar_teddy_offsets(s);
        let naive = NaiveScanner.scan_offsets(s);
        assert_eq!(teddy, naive);
        assert_eq!(teddy.len(), 8, "sample has 8 triggers");
    }

    #[test]
    fn sub_trigram_sources_are_safe() {
        // Bounds-check edge: sources shorter than one trigram (3
        // bytes) must not panic and must yield no offsets, since
        // the smallest matchable window is exactly that wide.
        for s in ["", "a", "ab"] {
            assert_eq!(scalar_teddy_offsets(s), Vec::<u32>::new());
        }
    }

    proptest::proptest! {
        #[test]
        fn scalar_teddy_matches_naive_on_aozora_fragments(
            s in aozora_proptest::generators::aozora_fragment(64),
        ) {
            let teddy = scalar_teddy_offsets(&s);
            let naive = NaiveScanner.scan_offsets(&s);
            proptest::prop_assert_eq!(teddy, naive);
        }

        #[test]
        fn scalar_teddy_matches_naive_on_pathological(
            s in aozora_proptest::generators::pathological_aozora(8),
        ) {
            let teddy = scalar_teddy_offsets(&s);
            let naive = NaiveScanner.scan_offsets(&s);
            proptest::prop_assert_eq!(teddy, naive);
        }

        #[test]
        fn scalar_teddy_matches_naive_on_unicode_adversarial(
            s in aozora_proptest::generators::unicode_adversarial(),
        ) {
            let teddy = scalar_teddy_offsets(&s);
            let naive = NaiveScanner.scan_offsets(&s);
            proptest::prop_assert_eq!(teddy, naive);
        }
    }
}
