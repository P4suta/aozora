//! Aozora Bunko notation — canonical specification crate.
//!
//! This crate is the **single source of truth** for facts that every other
//! crate in the workspace needs to agree on:
//!
//! - **PUA sentinel codepoints** — the four `U+E001..U+E004` markers the
//!   lexer injects into normalized text (see [`sentinels`]).
//! - **[`Span`]** — `(u32, u32)` byte-range over a UTF-8 source.
//! - **[`TriggerKind`]** — the set of Aozora notation marker characters
//!   (`｜《》［］＃※〔〕「」`) plus the const-PHF byte-sequence lookup
//!   table that maps a UTF-8 trigger byte sequence to its kind.
//! - **[`PairKind`]** — categories of balanced open/close delimiters.
//! - **[`Diagnostic`]** — every non-fatal observation any phase can emit.
//!
//! ## Why a separate crate
//!
//! Pre-0.2 these types lived scattered between `aozora-syntax` (`Span`)
//! and `aozora-lexer` (`Diagnostic`, sentinels, `TriggerKind`,
//! `PairKind`). The result: any crate that wanted a [`Diagnostic`] had
//! to depend on the full lexer, which transitively dragged in the
//! 7-phase pipeline. The new layered architecture (ADR-0009) requires
//! `aozora-syntax`, `aozora-scan`, `aozora-lex`, `aozora-render`,
//! `aozora-parallel`, and `aozora` itself to all reference these
//! shared types without depending on the engine.
//!
//! Concretely, this crate has **no internal dependency** on any other
//! `aozora-*` crate, only on `phf`/`miette`/`thiserror`. Every other
//! crate may depend on `aozora-spec` and re-export from it.

#![forbid(unsafe_code)]

pub mod diagnostic;
pub mod pair;
pub mod sentinels;
pub mod slugs;
pub mod span;
pub mod trigger;

pub use diagnostic::Diagnostic;
pub use pair::{PairKind, PairLink};
pub use sentinels::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
pub use slugs::{SLUGS, SlugEntry, SlugFamily, canonicalise_slug};
pub use span::Span;
pub use trigger::{TriggerKind, classify_trigger_bytes};
