//! aarch64 NEON trigger scanner.
//!
//! ## Status
//!
//! Scaffold only as of this commit. The development host is
//! `x86_64`; NEON development is gated on access to an `aarch64`
//! workstation (Apple Silicon, Linux ARM, etc.), at which point this
//! file fills out with the same algorithm as
//! [`super::avx2::Avx2Scanner`] but using NEON intrinsics:
//!
//! - `vld1q_u8` for the 16-byte chunk load (NEON has 128-bit
//!   registers, half the width of AVX2's 256-bit YMM)
//! - `vceqq_u8` for byte-equal compares
//! - `vorrq_u8` for the OR fold across the three needles
//! - bit-pack the resulting mask via `vshrn_n_u16` + `vget_lane_u64`
//!   (the equivalent of AVX2's movemask, with two extra shifts on
//!   ARM since NEON has no native movemask instruction)
//!
//! ## Output equivalence
//!
//! When the implementation lands, the proptest in
//! `super::avx2::proptests` gets a sibling for NEON gated on
//! `cfg(target_arch = "aarch64")`, pinning byte-identical behaviour
//! against [`crate::ScalarScanner`].

#![allow(
    dead_code,
    reason = "scaffold; full implementation lands once aarch64 dev host is available"
)]

/// aarch64 NEON-driven [`crate::TriggerScanner`].
///
/// Stateless. Caller MUST verify NEON availability via
/// `std::is_aarch64_feature_detected!("neon")` before invoking.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeonScanner;
