//! HTML / Aozora-source renderers over the semantic AST.
//!
//! Rendering options used by [`crate::Snapshot`] to emit semantic HTML5 or
//! canonical Aozora source text.
//!
//! # Public surface
//!
//! - [`RenderOptions`] configures [`crate::Snapshot::to_html_with`].
//! - [`SerializeOptions`] configures [`crate::Snapshot::to_source_with`].
//! - [`DirectiveNormalization`] controls opt-in canonical directive handling.

#![forbid(unsafe_code)]

mod classes;
mod html;
mod render_node;
mod serialize;
pub(crate) mod spelling;
mod walk;

pub use classes::AOZORA_CLASSES;
pub use html::RenderOptions;
pub(crate) use html::{render_html, render_html_normalized};
pub use serialize::{DirectiveNormalization, SerializeOptions};
pub(crate) use serialize::{requires_verbatim_recovery, serialize, serialize_with};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::lex;

    #[test]
    fn html_renders_plain_text_in_paragraph() {
        let out = lex("hello, world");
        let html = render_html(&out);
        assert!(html.contains("hello, world"), "html: {html}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let out = lex("plain text");
        let s = serialize(&out);
        assert_eq!(s, "plain text");
    }
}
