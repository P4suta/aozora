//! HTML / Aozora-source renderers.
//!
//! Move 3 ships this crate as the public namespace for output
//! rendering. The implementation re-exports `aozora-parser`'s
//! existing `html` module and `serialize` function until Move 2's
//! fused lex engine produces borrowed AST natively, at which point
//! the visitor-pattern rendering strategy of Innovation I-10 lands
//! here.
//!
//! ## Public surface
//!
//! - [`html::render_to_string`] — render a [`ParseResult`] to a
//!   semantic-HTML5 string.
//! - [`serialize`] — invert the lex pipeline and emit Aozora source
//!   text from a [`ParseResult`].
//! - [`ParseResult`] / [`ParseArtifacts`] — the legacy parse output
//!   types these renderers consume.

#![forbid(unsafe_code)]

pub use aozora_parser::{ParseArtifacts, ParseResult, html, serialize, serialize_from_artifacts};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_to_string_handles_plain_text() {
        // `render_to_string` re-parses internally; it takes the raw
        // source text rather than a pre-parsed result. Use
        // `render_from_artifacts` for the post-parse path.
        let out = html::render_to_string("hello, world");
        assert!(out.contains("hello, world"), "render output: {out}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let parsed = aozora_parser::parse("plain text");
        let out = serialize(&parsed);
        assert_eq!(out, "plain text");
    }
}
