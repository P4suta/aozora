//! Compile-time build identity for the `aozora` binary.
//!
//! The string is resolved in this crate's `build.rs` (channel + git sha) and
//! injected through `rustc-env`; consumers read [`VERSION`] instead of
//! `CARGO_PKG_VERSION` so a contributor's local build (`…-dev+g<sha>`) is
//! distinguishable from a nightly (`…-nightly.<date>+g<sha>`) or a clean stable
//! release (`X.Y.Z`).

/// The channel-aware build version, e.g. `1.2.3-dev+g3672e3f` (local checkout),
/// `1.2.3-nightly.20260629+g3672e3f` (scheduled build), or a clean `1.2.3`
/// (stable release / crates.io install).
pub(crate) const VERSION: &str = env!("AOZORA_VERSION_STRING");
