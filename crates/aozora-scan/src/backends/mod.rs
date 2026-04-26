//! Trigger-scan backends for [`crate::TriggerScanner`].
//!
//! ADR-0015 (the v2 bake-off result) settles on the three-backend
//! production set:
//!
//! - **Teddy** (`teddy.rs`) — Hyperscan multi-pattern fingerprint
//!   matcher via `aho_corasick::packed::Searcher`. The bake-off
//!   winner; primary production scanner on every modern x86_64 host.
//! - **Structural bitmap** (`structural_bitmap.rs`, `x86_64` only) —
//!   simdjson-style two-byte (lead × middle) AVX2 candidate filter.
//!   Production fallback when Teddy can't build (no SSSE3) but AVX2
//!   is still available.
//! - **DFA** (`dfa.rs`) — Hoehrmann-style multi-pattern byte DFA via
//!   `regex_automata::dfa::dense`. Universal SIMD-free fallback;
//!   also serves as a correctness-by-construction baseline for
//!   `[crate::NaiveScanner]` cross-validation.
//! - **NEON / wasm-simd** are placeholder scaffolds for future
//!   non-x86 ports.
//!
//! All backends produce **byte-identical output**, validated by the
//! cross-backend proptests in each module that compare against
//! [`crate::NaiveScanner`] (the brute-force PHF reference).

#[cfg(target_arch = "aarch64")]
mod neon;

#[cfg(target_arch = "wasm32")]
mod wasm_simd;

#[cfg(feature = "std")]
mod teddy;

#[cfg(feature = "std")]
mod dfa;

#[cfg(target_arch = "x86_64")]
mod structural_bitmap;

#[cfg(target_arch = "aarch64")]
pub use neon::NeonScanner;

#[cfg(target_arch = "wasm32")]
pub use wasm_simd::WasmSimdScanner;

#[cfg(feature = "std")]
pub use teddy::TeddyScanner;

#[cfg(feature = "std")]
pub use dfa::DfaScanner;

#[cfg(target_arch = "x86_64")]
pub use structural_bitmap::StructuralBitmapScanner;
