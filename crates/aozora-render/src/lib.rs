//! HTML / Aozora-source renderers — owned-AST.
//!
//! Consumes an [`aozora_syntax::owned::OwnedLexOutput`] and emits semantic
//! HTML5 or canonical Aozora source text.
//!
//! # Public surface
//!
//! - [`html_owned::render_html_owned`] — owned-AST HTML rendering. Pair with
//!   [`aozora_pipeline::lex`].
//! - [`html_owned::render_html_owned_normalized`] — the opt-in twin that first
//!   normalises Tier1 directive near-misses to canonical form (via the
//!   formatter rewrite) so a known 揺れ renders non-inert. See
//!   [`html_owned::RenderOptions`] and ADR-0022's fourth role.
//! - `serialize_owned` — round-trip the parsed tree back to
//!   Aozora source text.
//!
//! The [`render_node`] / `html` / [`serialize`] modules hold the shared,
//! lifetime-free byte-spelling helpers both owned renderers reuse (container
//! tags, marker spellings, the text escaper); `serialize::container_close_source`
//! / `container_open_source` are the splice layer's canonical marker source.

#![forbid(unsafe_code)]

pub mod classes;
mod html;
pub mod html_owned;
pub mod render_node;
mod render_node_owned;
pub mod serialize;
pub mod serialize_owned;
mod walk;

pub use classes::AOZORA_CLASSES;
pub use html_owned::{RenderOptions, render_html_owned, render_html_owned_normalized};
pub use serialize_owned::{SerializeOptions, serialize_owned, serialize_owned_with};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_renders_plain_text_in_paragraph() {
        let out = aozora_pipeline::lex("hello, world");
        let html = render_html_owned(&out);
        assert!(html.contains("hello, world"), "html: {html}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let out = aozora_pipeline::lex("plain text");
        let s = serialize_owned(&out);
        assert_eq!(s, "plain text");
    }
}
