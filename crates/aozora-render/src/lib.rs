//! HTML / Aozora-source renderers (Plan B.5 — borrowed-only).
//!
//! Consumes [`aozora_lex::BorrowedLexOutput`] directly and emits
//! semantic HTML5 or canonical Aozora source text. The legacy
//! owned-AST `aozora_parser` renderers were retired in Plan B.5; the
//! `aozora-parser` crate remains as an internal-test artifact and is
//! not re-exported through any public surface here.
//!
//! # Public surface
//!
//! - [`html::render_to_string`] / [`html::render_into`] — borrowed-AST
//!   HTML rendering. Pair with [`aozora_lex::lex_into_arena`].
//! - [`serialize::serialize`] / [`serialize::serialize_into`] —
//!   round-trip the parsed tree back to Aozora source text.
//! - [`render_node::render`] — per-node HTML renderer; usually
//!   called via the block walker but exposed for visitor-style
//!   consumers.

#![forbid(unsafe_code)]

mod bouten;
pub mod html;
pub mod render_node;
pub mod serialize;
pub mod visitor;

pub use visitor::{AozoraVisitor, dispatch_node};

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;

    #[test]
    fn html_renders_plain_text_in_paragraph() {
        let arena = Arena::new();
        let out = aozora_lex::lex_into_arena("hello, world", &arena);
        let html = html::render_to_string(&out);
        assert!(html.contains("hello, world"), "html: {html}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let arena = Arena::new();
        let out = aozora_lex::lex_into_arena("plain text", &arena);
        let s = serialize::serialize(&out);
        assert_eq!(s, "plain text");
    }
}
