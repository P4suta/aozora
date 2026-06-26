//! HTML / Aozora-source renderers — borrowed-only.
//!
//! Consumes [`aozora_pipeline::LexOutput`] directly and emits
//! semantic HTML5 or canonical Aozora source text.
//!
//! # Public surface
//!
//! - [`html::render_to_string`] / [`html::render_into`] — borrowed-AST
//!   HTML rendering. Pair with [`aozora_pipeline::lex`].
//! - [`html_owned::render_html_owned`] — owned-AST HTML rendering, the
//!   byte-identical mirror over an [`aozora_syntax::owned::OwnedLexOutput`].
//! - [`serialize::serialize`] / [`serialize::serialize_into`] —
//!   round-trip the parsed tree back to Aozora source text.
//! - [`render_node::render`] — per-node HTML renderer; usually
//!   called via the block walker but exposed for visitor-style
//!   consumers.

#![forbid(unsafe_code)]

pub mod classes;
pub mod html;
pub mod html_owned;
pub mod render_node;
mod render_node_owned;
pub mod serialize;
pub mod serialize_owned;
pub mod visitor;
mod walk;

pub use classes::AOZORA_CLASSES;
pub use html_owned::render_html_owned;
pub use serialize_owned::serialize_owned;
pub use visitor::{AozoraVisitor, dispatch_node};

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;

    #[test]
    fn html_renders_plain_text_in_paragraph() {
        let arena = Arena::new();
        let out = aozora_pipeline::lex("hello, world", &arena);
        let html = html::render_to_string(&out);
        assert!(html.contains("hello, world"), "html: {html}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let arena = Arena::new();
        let out = aozora_pipeline::lex("plain text", &arena);
        let s = serialize::serialize(&out);
        assert_eq!(s, "plain text");
    }
}
