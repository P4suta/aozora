//! Diagnostic stream produced by the lexer.
//!
//! The canonical type definition lives in [`aozora_spec::Diagnostic`].
//! This module re-exports it for backward compatibility with downstream
//! consumers that historically imported `aozora_lexer::Diagnostic`. New
//! code should reach for `aozora_spec::Diagnostic` directly.
//!
//! Tests for the variant constructors and `Display` impls live alongside
//! the canonical definition (see `crates/aozora-spec/src/diagnostic.rs`).

pub use aozora_spec::Diagnostic;
