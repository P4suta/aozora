//! SIMD backends for [`crate::TriggerScanner`].
//!
//! Each backend implements the same trait shape over a different
//! target ISA:
//!
//! - `avx2` — `x86_64` AVX2 + BMI2. 32 bytes / iteration via
//!   `_mm256_cmpeq_epi8`, candidate position extraction via the
//!   movemask-and-popcount pattern (BMI2 PEXT not yet used because
//!   the candidate density is low enough that the trailing-zeros
//!   loop dominates the actual classify cost).
//! - `neon` (planned) — aarch64 NEON. Same 16-byte chunk shape but
//!   without movemask: bit-pack via shifted bytes + horizontal OR.
//! - `wasm_simd` (planned) — wasm32 simd128 via `core::arch::wasm32`.
//!   Same algorithm as AVX2 but 128-bit chunks; pairs with the
//!   v8 / SpiderMonkey runtime SIMD support.
//!
//! All backends produce **byte-identical output** to
//! [`super::ScalarScanner`]; the property tests in
//! `src/scalar.rs` cross-check that invariant on randomly generated
//! input.

#[cfg(target_arch = "x86_64")]
mod avx2;

#[cfg(target_arch = "aarch64")]
mod neon;

#[cfg(target_arch = "wasm32")]
mod wasm_simd;

#[cfg(target_arch = "x86_64")]
pub use avx2::Avx2Scanner;

#[cfg(target_arch = "aarch64")]
pub use neon::NeonScanner;

#[cfg(target_arch = "wasm32")]
pub use wasm_simd::WasmSimdScanner;
