//! Legacy trigger-scan backends — pre-Teddy-redesign implementations
//! that the runtime dispatcher used to fan out to via
//! `&'static dyn TriggerScanner`. The new dispatch path
//! ([`crate::BackendChoice`]) goes through the hand-rolled Teddy
//! kernels under `crate::kernel` + `crate::arch`; these modules are
//! retained because:
//!
//! - **Teddy** (`teddy.rs`) — Hyperscan multi-pattern fingerprint
//!   matcher via `aho_corasick::packed::Searcher`. Bake-off baseline
//!   for the new self-rolled Teddy.
//! - **Structural bitmap** (`structural_bitmap.rs`, `x86_64` only) —
//!   simdjson-style two-byte (lead × middle) AVX2 candidate filter.
//!   Cross-validation oracle for the new Teddy on AVX2 hosts.
//! - **DFA** (`dfa.rs`) — Hoehrmann-style multi-pattern byte DFA via
//!   `regex_automata::dfa::dense`. Reference for "what an
//!   SIMD-free implementation looks like".
//!
//! All three remain `proptest`-validated against
//! [`crate::NaiveScanner`] (the brute-force PHF reference).
//! Subsequent G5-S6 cleanup moves them under a `bench-baselines`
//! Cargo feature so default builds stop pulling
//! `aho_corasick` / `regex_automata` into the dep tree.

#[cfg(feature = "std")]
mod teddy;

#[cfg(feature = "std")]
mod dfa;

#[cfg(target_arch = "x86_64")]
mod structural_bitmap;

#[cfg(feature = "std")]
pub use teddy::TeddyScanner;

#[cfg(feature = "std")]
pub use dfa::DfaScanner;

#[cfg(target_arch = "x86_64")]
pub use structural_bitmap::StructuralBitmapScanner;
