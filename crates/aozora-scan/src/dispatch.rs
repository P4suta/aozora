//! Runtime backend selection for the hand-rolled Teddy stack.
//!
//! [`BackendChoice`] is a `Copy` enum carrying exactly one variant
//! per kernel currently compiled into the build. `detect()` runs
//! once at process start, picks the fastest variant the host CPU
//! supports, and returns it; `scan` then dispatches statically via
//! `match`, which lets monomorphisation inline the SIMD inner kernel
//! through the outer driver into the call site (the `phase1_events`
//! tokeniser).
//!
//! The dispatcher intentionally does not go through a trait object:
//! `&dyn TriggerScanner` cannot carry a generic `S: OffsetSink`
//! method, which would force a per-call heap `Vec<u32>` allocation
//! the redesign exists to remove. Holding the choice as a `Copy`
//! enum and `match`-ing in `scan` gives us the static dispatch we
//! want without surrendering the runtime CPU detection that lets a
//! single binary adapt to its host.

use crate::kernel::teddy::{ScalarTeddyKernel, teddy_outer};
use crate::trait_def::OffsetSink;

#[cfg(target_arch = "aarch64")]
use crate::arch::aarch64::NeonKernel;
#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::{Avx2Kernel, Ssse3Kernel};

/// Available scanning backends.
///
/// Variants enumerate the *set of kernels currently compiled into
/// the build*. Per-platform impls cfg-gate themselves out, so a
/// `BackendChoice` value never names a kernel the binary cannot
/// run; `match` arms collapse cleanly to the available paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    /// AVX2 32-byte Teddy. x86_64 only; selected when the host
    /// reports AVX2 at runtime.
    TeddyAvx2,
    /// SSSE3 16-byte Teddy. x86_64 only; the fallback when AVX2 is
    /// missing but SSSE3 is available (every `x86_64-v2` host).
    TeddySsse3,
    /// NEON 16-byte Teddy. aarch64 only; always available there
    /// since the ABI mandates NEON.
    TeddyNeon,
    /// Pure-Rust Teddy reference. Always available; the dispatch
    /// target on non-SIMD hosts and the `no_std` last resort.
    ScalarTeddy,
}

impl BackendChoice {
    /// Detect the best backend for the current host.
    ///
    /// Cheap: a couple of `is_x86_feature_detected!` checks plus
    /// returning a one-byte enum. Callers that hit a hot path
    /// should still cache the result (the
    /// [`crate::scan_offsets_in`] entry point already does so via
    /// the runtime CPU dispatcher).
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(all(feature = "std", target_arch = "x86_64"))]
        {
            if std::is_x86_feature_detected!("avx2") {
                return Self::TeddyAvx2;
            }
            if std::is_x86_feature_detected!("ssse3") {
                return Self::TeddySsse3;
            }
        }
        // aarch64 ABI mandates NEON; no runtime check needed.
        #[cfg(target_arch = "aarch64")]
        {
            return Self::TeddyNeon;
        }
        #[cfg(not(target_arch = "aarch64"))]
        Self::ScalarTeddy
    }

    /// Stable string name for this backend, suitable for logs and
    /// `eprintln!`-style introspection.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TeddyAvx2 => "teddy-avx2",
            Self::TeddySsse3 => "teddy-ssse3",
            Self::TeddyNeon => "teddy-neon",
            Self::ScalarTeddy => "scalar-teddy",
        }
    }

    /// Run the chosen backend's scan over `source`, pushing every
    /// trigger byte offset into `sink`.
    ///
    /// Static dispatch: the `match` arm collapses to a direct call
    /// into the appropriate inner kernel's monomorphised
    /// [`teddy_outer`] instantiation, leaving no virtual call on
    /// the hot path.
    #[inline]
    pub fn scan<S: OffsetSink>(self, source: &str, sink: &mut S) {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::TeddyAvx2 => teddy_outer::<Avx2Kernel, _>(source, sink),
            #[cfg(target_arch = "x86_64")]
            Self::TeddySsse3 => teddy_outer::<Ssse3Kernel, _>(source, sink),
            #[cfg(target_arch = "aarch64")]
            Self::TeddyNeon => teddy_outer::<NeonKernel, _>(source, sink),
            // SIMD variants the active target can't run are
            // unreachable from `detect()`, but the `match` must
            // still be exhaustive; collapse them to the scalar
            // reference so a directly-constructed value still
            // scans correctly.
            #[cfg(not(target_arch = "x86_64"))]
            Self::TeddyAvx2 | Self::TeddySsse3 => {
                teddy_outer::<ScalarTeddyKernel, _>(source, sink);
            }
            #[cfg(not(target_arch = "aarch64"))]
            Self::TeddyNeon => teddy_outer::<ScalarTeddyKernel, _>(source, sink),
            Self::ScalarTeddy => teddy_outer::<ScalarTeddyKernel, _>(source, sink),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TriggerScanner;
    use crate::naive::NaiveScanner;
    use alloc::vec::Vec;

    fn dispatch_offsets(choice: BackendChoice, source: &str) -> Vec<u32> {
        let mut sink: Vec<u32> = Vec::new();
        choice.scan(source, &mut sink);
        sink
    }

    #[test]
    fn detect_returns_a_variant_compiled_into_the_build() {
        // A literal sanity check: detect() must return one of the
        // four named variants, regardless of host.
        let choice = BackendChoice::detect();
        let _ = choice.name(); // exercises the match arms
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn detect_picks_neon_on_aarch64() {
        // The aarch64 ABI mandates NEON, so the dispatcher must
        // always pick the NEON kernel on this target. Catches
        // dispatch-table edits that accidentally collapse the
        // aarch64 path into ScalarTeddy.
        assert_eq!(BackendChoice::detect(), BackendChoice::TeddyNeon);
    }

    #[test]
    fn every_variant_matches_naive_on_handcrafted_sample() {
        let s = "漢《かん》字、※［＃ここまで］「終わり」";
        let naive = NaiveScanner.scan_offsets(s);
        for choice in [
            BackendChoice::TeddyAvx2,
            BackendChoice::TeddySsse3,
            BackendChoice::TeddyNeon,
            BackendChoice::ScalarTeddy,
        ] {
            // Skip variants the host doesn't actually support to
            // keep the test runnable on non-x86 / non-aarch64 CI.
            #[cfg(target_arch = "x86_64")]
            {
                if matches!(choice, BackendChoice::TeddyAvx2)
                    && !std::is_x86_feature_detected!("avx2")
                {
                    continue;
                }
                if matches!(choice, BackendChoice::TeddySsse3)
                    && !std::is_x86_feature_detected!("ssse3")
                {
                    continue;
                }
            }
            assert_eq!(
                dispatch_offsets(choice, s),
                naive,
                "{} did not match naive",
                choice.name(),
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn detect_dispatch_matches_naive_on_aozora_fragments(
            s in aozora_proptest::generators::aozora_fragment(64),
        ) {
            let actual = dispatch_offsets(BackendChoice::detect(), &s);
            let expected = NaiveScanner.scan_offsets(&s);
            proptest::prop_assert_eq!(actual, expected);
        }

        #[test]
        fn scalar_dispatch_matches_naive_on_pathological(
            s in aozora_proptest::generators::pathological_aozora(8),
        ) {
            let actual = dispatch_offsets(BackendChoice::ScalarTeddy, &s);
            let expected = NaiveScanner.scan_offsets(&s);
            proptest::prop_assert_eq!(actual, expected);
        }
    }
}
