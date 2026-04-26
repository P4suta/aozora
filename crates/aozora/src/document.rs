//! `Document` — single owning handle to a parsed Aozora source
//! buffer, and `AozoraTree<'a>` — borrowed view a caller walks for
//! output rendering.
//!
//! ## Plan B.4 / B.5 — borrowed AST production surface
//!
//! `Document` owns both the source buffer and a [`bumpalo`]-backed
//! [`Arena`]; [`Document::parse`] returns an [`AozoraTree<'_>`]
//! that borrows from the arena via the `&self` lifetime. Owning
//! source removes the self-referential-struct problem that would
//! otherwise plague driver wrappers (FFI/WASM/Py): callers can hold
//! a `Document` inside any wrapper without juggling source lifetimes.
//!
//! Every borrowed-AST allocation lives inside the arena, with the
//! [`Interner`](aozora_syntax::borrowed::Interner) deduplicating
//! repeated string content (Innovation I-7). Dropping the `Document`
//! frees the entire tree in a single `Bump::reset` step; no per-node
//! `Drop` runs.
//!
//! Plan B.5 has retired the legacy owned-AST `ParseResult` path that
//! pre-Plan-B `Document::parse_owned` exposed; the borrowed
//! [`Document::parse`] is now the only public entry.

use core::fmt;

use aozora_lex::{lex_into_arena, BorrowedLexOutput};
use aozora_render::{html as borrowed_html, serialize as borrowed_serialize};
use aozora_spec::Diagnostic;
use aozora_syntax::borrowed::Arena;

/// Pre-size the document arena as `source.len() * ARENA_CAPACITY_FACTOR`
/// bytes. Picked from the full-corpus `allocator_pressure` probe (N6
/// finding, 17435 docs): the median AST footprint is 3.4× the source
/// size, p99 is 8.25×, max 15.4×. Factor 4 covers the median + a
/// margin while keeping small-doc overhead minimal (a 1 KB doc gets
/// a 4 KB arena, the bumpalo default chunk size).
const ARENA_CAPACITY_FACTOR: usize = 4;

/// Single owning handle to a parsed Aozora source.
///
/// Owns both the source buffer and a [`bumpalo`]-backed [`Arena`].
/// The `&self` lifetime parameterises every borrowed-AST view
/// returned from [`Document::parse`]; consumers hold the tree only
/// as long as they hold a `&Document` reference.
pub struct Document {
    source: Box<str>,
    arena: Arena,
}

impl Document {
    /// Wrap a source string in a `Document`. The source is copied
    /// into a `Box<str>` so the document is fully self-contained
    /// (no external lifetime).
    ///
    /// The arena is pre-sized to `source.len() * ARENA_CAPACITY_FACTOR`
    /// bytes (a corpus-profile-driven estimate of the AST footprint).
    /// N6 finding (full-corpus allocator_pressure probe): p50 arena/source
    /// ratio is 3.4×, p99 is 8.25×; pre-sizing eliminates the early
    /// chunk-grow churn that hits large docs hardest. Callers that
    /// know the AST is unusually small can fall back to
    /// [`Self::with_arena_capacity`] with a smaller hint.
    #[must_use]
    pub fn new(source: impl Into<Box<str>>) -> Self {
        let source: Box<str> = source.into();
        let capacity = source.len().saturating_mul(ARENA_CAPACITY_FACTOR);
        Self {
            source,
            arena: Arena::with_capacity(capacity),
        }
    }

    /// Wrap a source string with a pre-sized arena.
    #[must_use]
    pub fn with_arena_capacity(source: impl Into<Box<str>>, capacity_hint: usize) -> Self {
        Self {
            source: source.into(),
            arena: Arena::with_capacity(capacity_hint),
        }
    }

    /// The source text owned by this document.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Arena bytes currently committed. Diagnostic / benchmarking only.
    #[must_use]
    pub fn arena_bytes(&self) -> usize {
        self.arena.allocated_bytes()
    }

    /// Parse the document, returning a borrowed-AST view bound to
    /// `&self`'s lifetime.
    #[must_use]
    pub fn parse(&self) -> AozoraTree<'_> {
        AozoraTree {
            source: &self.source,
            inner: lex_into_arena(&self.source, &self.arena),
        }
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("source_len", &self.source.len())
            .field("arena_bytes", &self.arena.allocated_bytes())
            .finish()
    }
}

/// Borrowed view into a parsed Aozora document (Plan B.4).
///
/// Wraps a [`BorrowedLexOutput`] whose normalized text and registry
/// borrow from the parent [`Document`]'s arena. Renderer methods
/// dispatch to `aozora_render`'s borrowed-AST implementations.
#[derive(Debug)]
pub struct AozoraTree<'a> {
    source: &'a str,
    inner: BorrowedLexOutput<'a>,
}

impl<'a> AozoraTree<'a> {
    /// The source text this tree was parsed from.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// Diagnostics emitted during parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.inner.diagnostics
    }

    /// Borrow the underlying [`BorrowedLexOutput`].
    #[must_use]
    pub fn lex_output(&self) -> &BorrowedLexOutput<'a> {
        &self.inner
    }

    /// Render the tree to a semantic-HTML5 string.
    #[must_use]
    pub fn to_html(&self) -> String {
        borrowed_html::render_to_string(&self.inner)
    }

    /// Re-emit Aozora source text from the parsed tree.
    #[must_use]
    pub fn serialize(&self) -> String {
        borrowed_serialize::serialize(&self.inner)
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
    fn parse_returns_borrowed_tree_with_same_source() {
        let s = "world";
        let d = Document::new(s);
        let t = d.parse();
        assert_eq!(t.source(), s);
    }

    #[test]
    fn diagnostics_empty_for_clean_input() {
        let d = Document::new("plain");
        let t = d.parse();
        assert!(t.diagnostics().is_empty());
    }

    #[test]
    fn diagnostics_populated_for_pua_collision() {
        let d = Document::new("contains \u{E001} sentinel");
        let t = d.parse();
        assert!(!t.diagnostics().is_empty());
    }

    #[test]
    fn round_trip_through_serialize_is_a_fixed_point() {
        let s = "｜青梅《おうめ》";
        let first = Document::new(s).parse().serialize();
        let second = Document::new(first.clone()).parse().serialize();
        assert_eq!(first, second, "round-trip must be a fixed point");
    }

    #[test]
    fn arena_grows_with_source_size() {
        let small = Document::new("a");
        drop(small.parse());
        let big_src = "｜青梅《おうめ》".repeat(100);
        let big = Document::new(big_src);
        drop(big.parse());
        assert!(big.arena_bytes() > small.arena_bytes());
    }
}
