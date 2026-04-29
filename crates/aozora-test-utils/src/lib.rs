#![forbid(unsafe_code)]

//! Shared test utilities for the aozora workspace.
//!
//! This crate collects proptest [`Strategy`]s and [`ProptestConfig`]
//! defaults shared across the workspace's integration tests. It is
//! **not published** and is consumed only via `[dev-dependencies]` —
//! production code must not pull it in.
//!
//! [`Strategy`]: proptest::prelude::Strategy
//! [`ProptestConfig`]: proptest::prelude::ProptestConfig

pub mod config;
pub mod generators;
