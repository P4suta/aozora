//! WASM SIMD128 trigger scanner.
//!
//! ## Status
//!
//! Scaffold only as of this commit. The development host's Rust
//! toolchain does not currently ship the `wasm32-unknown-unknown`
//! target (`rustup target add wasm32-unknown-unknown` deferred to
//! the WASM CI integration commit). When the target is available
//! this file fills out with the same algorithm as
//! [`super::avx2::Avx2Scanner`] using `core::arch::wasm32`
//! SIMD128 intrinsics:
//!
//! - `v128_load` for the 16-byte chunk load
//! - `i8x16_eq` for byte-equal compares
//! - `v128_or` for the OR fold across the three needles
//! - `i8x16_bitmask` for the movemask projection (WASM SIMD has the
//!   movemask analogue natively, unlike NEON)
//!
//! ## Output equivalence
//!
//! Pinned by the same proptest pattern as
//! [`super::avx2::proptests::byte_identical_to_scalar`], gated on
//! `cfg(target_arch = "wasm32")` and only run during
//! `wasm-pack test --headless`.

#![allow(
    dead_code,
    reason = "scaffold; full implementation lands once the wasm32 target is available"
)]

/// WASM SIMD128-driven [`crate::TriggerScanner`].
///
/// Stateless. The wasm32 target's SIMD128 features are statically
/// enabled (no runtime detection equivalent), so callers can
/// instantiate this scanner unconditionally on `cfg(target_arch =
/// "wasm32")` builds compiled with `-Ctarget-feature=+simd128`.
#[derive(Debug, Clone, Copy, Default)]
pub struct WasmSimdScanner;
