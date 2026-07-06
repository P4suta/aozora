//! HTML / Aozora-source renderers over the semantic AST.
//!
//! Consumes an [`aozora_syntax::ast::LexOutput`] and emits semantic
//! HTML5 or canonical Aozora source text.
//!
//! # Public surface
//!
//! - [`html::render_html`] — HTML rendering. Pair with
//!   [`aozora_pipeline::lex`].
//! - [`html::render_html_normalized`] — the opt-in twin that first
//!   normalises Tier1 directive near-misses to canonical form (via the
//!   formatter rewrite) so a known 揺れ renders non-inert. See
//!   [`html::RenderOptions`] and ADR-0022's fourth role.
//! - [`serialize::serialize`] — round-trip the parsed tree back to
//!   Aozora source text.
//!
//! The [`spelling`] module holds the shared, lifetime-free byte-spelling
//! helpers every renderer reuses (container tags, marker spellings, the text
//! escaper); [`spelling::source::container_close_source`] /
//! `container_open_source` are the splice layer's canonical marker source.

#![forbid(unsafe_code)]

pub mod classes;
pub mod html;
mod render_node;
pub mod serialize;
pub mod spelling;
mod walk;

pub use classes::AOZORA_CLASSES;
pub use html::{RenderOptions, render_html, render_html_normalized};
pub use serialize::{SerializeOptions, serialize, serialize_with};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_renders_plain_text_in_paragraph() {
        let out = aozora_pipeline::lex("hello, world");
        let html = render_html(&out);
        assert!(html.contains("hello, world"), "html: {html}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let out = aozora_pipeline::lex("plain text");
        let s = serialize(&out);
        assert_eq!(s, "plain text");
    }
}
