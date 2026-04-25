//! `Document<'src>` — the single owning handle to a parsed Aozora
//! source buffer, and `AozoraTree<'src>` — the borrowed view a
//! caller walks for output.
//!
//! ## Move 3 façade phase
//!
//! Today both types are thin wrappers around
//! [`aozora_parser::ParseResult`]. The lifetime parameter `'src`
//! tracks the source string's lifetime as the future-state design
//! requires; the wrapper does not (yet) own a bumpalo arena because
//! the legacy `ParseResult` allocates with the global allocator. When
//! Move 2's fused engine starts producing arena-backed
//! [`aozora_syntax::borrowed::AozoraNode`] values, `Document` will
//! gain the arena and `AozoraTree` will switch to borrowing from it
//! — without changing this module's public API shape.

use aozora_parser::ParseResult;
use aozora_render::legacy::html::render_from_artifacts;
use aozora_render::serialize as render_serialize;

/// Single owning handle to a parsed Aozora source.
///
/// `'src` is the lifetime of the source string this `Document`
/// borrows. Consumers typically own the source themselves and pass
/// it in via [`Document::new`]; the parser produces arena-allocated
/// or source-borrowed structures whose lifetime is bounded by `'src`.
#[derive(Debug)]
pub struct Document<'src> {
    source: &'src str,
}

impl<'src> Document<'src> {
    /// Wrap a source string in a `Document` ready for parsing.
    #[must_use]
    pub fn new(source: &'src str) -> Self {
        Self { source }
    }

    /// The source text this document borrows.
    #[must_use]
    pub fn source(&self) -> &'src str {
        self.source
    }

    /// Parse the document, returning a tree view of the result.
    ///
    /// Today this delegates to the legacy [`aozora_parser::parse`]
    /// pipeline; once Move 2 ships the fused engine the tree will
    /// borrow from a bumpalo arena owned by `self`.
    #[must_use]
    pub fn parse(&self) -> AozoraTree<'src> {
        AozoraTree {
            source: self.source,
            inner: aozora_parser::parse(self.source),
        }
    }
}

/// Borrowed view into a parsed Aozora document.
///
/// Owns the legacy [`ParseResult`] internally (Move 3 façade); the
/// rendering surface (`to_html`, `serialize`) walks the result
/// without exposing the wrapping type.
#[derive(Debug)]
pub struct AozoraTree<'src> {
    /// Source text this tree was parsed from. Borrowed back to
    /// callers that want to slice spans without re-loading the file.
    source: &'src str,
    /// Underlying parse output. Crate-private — outside callers
    /// reach for it via the renderer methods.
    inner: ParseResult,
}

impl<'src> AozoraTree<'src> {
    /// The source text this tree was parsed from.
    #[must_use]
    pub fn source(&self) -> &'src str {
        self.source
    }

    /// Diagnostics emitted during parsing. Empty on the happy path.
    #[must_use]
    pub fn diagnostics(&self) -> &[aozora_spec::Diagnostic] {
        &self.inner.diagnostics
    }

    /// Render the tree to a semantic-HTML5 string.
    #[must_use]
    pub fn to_html(&self) -> String {
        // We use `render_from_artifacts` so the source is not re-parsed
        // a second time inside the renderer; `render_to_string` would
        // call `parse(input)` again.
        render_from_artifacts(&self.inner.artifacts)
    }

    /// Re-emit Aozora source text from the parsed tree. Round-trips
    /// to a fixed point after one parse-serialize cycle (ADR-0005
    /// corpus invariant I3).
    #[must_use]
    pub fn serialize(&self) -> String {
        render_serialize(&self.inner)
    }

    /// The underlying [`ParseResult`]. Exposed for callers that need
    /// to reach into the legacy shape during the Move 3 transition;
    /// new code should prefer [`AozoraTree::to_html`] /
    /// [`AozoraTree::serialize`] / [`AozoraTree::diagnostics`].
    #[must_use]
    pub fn into_parse_result(self) -> ParseResult {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_borrows_source() {
        let s = "hello";
        let d = Document::new(s);
        assert_eq!(d.source(), s);
    }

    #[test]
    fn tree_exposes_source_back() {
        let s = "world";
        let d = Document::new(s);
        let t = d.parse();
        assert_eq!(t.source(), s);
    }

    #[test]
    fn tree_diagnostics_empty_for_clean_input() {
        let d = Document::new("plain");
        let t = d.parse();
        assert!(t.diagnostics().is_empty());
    }

    #[test]
    fn tree_diagnostics_populated_for_pua_collision() {
        let d = Document::new("contains \u{E001} sentinel");
        let t = d.parse();
        assert!(!t.diagnostics().is_empty());
    }

    #[test]
    fn round_trip_through_serialize_is_a_fixed_point() {
        let s = "｜青梅《おうめ》";
        let first = Document::new(s).parse().serialize();
        let second = Document::new(&first).parse().serialize();
        assert_eq!(first, second, "round-trip must be a fixed point");
    }

    #[test]
    fn into_parse_result_yields_underlying_data() {
        let d = Document::new("x");
        let t = d.parse();
        let pr = t.into_parse_result();
        assert_eq!(pr.artifacts.normalized, "x");
    }
}
