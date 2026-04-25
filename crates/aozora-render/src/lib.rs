//! HTML / Aozora-source renderers.
//!
//! Plan B.3 adds borrowed-AST native renderers.
//!
//! These consume [`aozora_lex::BorrowedLexOutput`] directly.
//! The legacy owned-AST renderers continue to live in
//! `aozora-parser` and are re-exported through the [`legacy`]
//! module for now.
//!
//! # Public surface
//!
//! - [`html::render_to_string`] / [`html::render_into`] — borrowed-AST
//!   HTML rendering. Pair with [`aozora_lex::lex_into_arena`].
//! - [`render_node::render`] — per-node renderer; usually called via
//!   the block walker but exposed for visitor-style consumers.
//! - [`legacy`] — the pre-Plan-B owned-AST renderers,
//!   re-exported from `aozora-parser` for downstream that have not
//!   migrated yet.

#![forbid(unsafe_code)]

mod bouten;
pub mod html;
pub mod render_node;

/// Pre-Plan-B owned-AST renderers, re-exported from `aozora-parser`.
///
/// New code should consume the borrowed-AST [`html`] module instead;
/// these re-exports remain only until Plan B.5 retires the legacy
/// implementation.
pub mod legacy {
    pub use aozora_parser::{
        ParseArtifacts, ParseResult, html, serialize, serialize_from_artifacts,
    };
}

// Backwards-compat: prior crate revisions exposed `aozora-parser`'s
// `html` / `serialize` directly under `aozora-render`. Keep the old
// import paths working until Plan B.5 retires them.
pub use legacy::{ParseArtifacts, ParseResult, serialize, serialize_from_artifacts};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_render_to_string_handles_plain_text() {
        let out = legacy::html::render_to_string("hello, world");
        assert!(out.contains("hello, world"), "render output: {out}");
    }

    #[test]
    fn legacy_serialize_round_trips_plain_text() {
        let parsed = aozora_parser::parse("plain text");
        let out = serialize(&parsed);
        assert_eq!(out, "plain text");
    }

    #[test]
    fn borrowed_html_matches_owned_for_plain() {
        use aozora_syntax::borrowed::Arena;
        // Spot-check that the borrowed renderer produces the same
        // bytes as the legacy one for trivial input. A full corpus
        // sweep + proptest lives in tests/byte_identical_html.rs.
        let arena = Arena::new();
        let out = aozora_lex::lex_into_arena("Hello.", &arena);
        let borrowed = html::render_to_string(&out);
        let owned = legacy::html::render_to_string("Hello.");
        assert_eq!(borrowed, owned);
    }
}
