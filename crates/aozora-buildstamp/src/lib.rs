//! Compile-time build identity for the aozora front-ends (the `aozora` /
//! `aozora-lsp` binaries and the `aozora-wasm` playground driver).
//!
//! The string is resolved in `build.rs` (channel + git sha) and injected through
//! `rustc-env`; consumers read [`VERSION`] instead of `CARGO_PKG_VERSION` so a
//! contributor's local build (`…-dev+g<sha>`) is distinguishable from a nightly
//! (`…-nightly.<date>+g<sha>`) or a clean stable release (`X.Y.Z`). The
//! playground surfaces it in its footer so a deployed build is traceable to a
//! commit.
//!
//! This crate is a leaf depended on ONLY by those front-ends — never by the hot
//! library crates (`aozora`, `aozora-syntax`, …) — so the `build.rs` git probe
//! never forces them to recompile. (`aozora-wasm` pulls it in only on the
//! wasm32 target, so host workspace builds skip it entirely.)

#![forbid(unsafe_code)]

/// The channel-aware build version, e.g. `0.4.1-dev+g3672e3f` (local checkout),
/// `0.4.1-nightly.20260629+g3672e3f` (scheduled build), or a clean `0.4.1`
/// (stable release / crates.io install).
pub const VERSION: &str = env!("AOZORA_VERSION_STRING");
