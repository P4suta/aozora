#![forbid(unsafe_code)]

//! Shared test utilities for the afm workspace.
//!
//! This crate exists to collect proptest [`Strategy`]s and
//! [`ProptestConfig`] defaults that were previously duplicated across
//! `afm-parser`'s integration tests. It is **not published** and is
//! consumed only via `[dev-dependencies]` — production code must not
//! pull it in.
//!
//! The split between predicates (hosted in `aozora_parser::test_support`)
//! and generators (here) is deliberate: predicates are pure `fn(&str)`
//! helpers with no runtime dependencies, while generators need
//! `proptest` and so their dependency lives here alone.
//!
//! [`Strategy`]: proptest::prelude::Strategy
//! [`ProptestConfig`]: proptest::prelude::ProptestConfig

pub mod config;
pub mod generators;
