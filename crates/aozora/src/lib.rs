//! `aozora` — the public meta crate.
//!
//! Single front door for parsing Aozora Bunko notation. Downstream
//! consumers should depend on this crate alone; everything they need
//! is re-exported through this surface or accessed via [`Document`]
//! and [`AozoraTree`].
//!
//! ```no_run
//! use aozora::Document;
//!
//! let source = std::fs::read_to_string("crime_and_punishment.txt").unwrap();
//! let doc = Document::new(source);
//! let tree = doc.parse();
//! let html = tree.to_html();
//! println!("{html}");
//! ```
//!
//! Tunable parses go through the builder chain:
//!
//! ```
//! use aozora::{Document, DiagnosticPolicy};
//!
//! let doc = Document::options()
//!     .arena_capacity(64 * 1024)
//!     .diagnostic_policy(DiagnosticPolicy::DropInternal)
//!     .build("｜青梅《おうめ》");
//! let tree = doc.parse();
//! assert!(!tree.serialize().is_empty());
//! ```
//!
//! # Architecture
//!
//! [`Document`] owns the source buffer plus a `bumpalo`-backed
//! arena. [`AozoraTree`] borrows from that arena via the `&self`
//! lifetime returned by [`Document::parse`]. Every per-node
//! allocation lives inside the arena, with the
//! [`Interner`](aozora_syntax::borrowed::Interner) deduplicating
//! repeated string content; dropping the `Document` releases the
//! entire tree in a single `Bump::reset` step.
//!
//! Internal build-block crates (`aozora-spec`, `aozora-syntax`,
//! `aozora-pipeline`, `aozora-render`, `aozora-encoding`) are
//! `publish = false` and reachable only through this meta crate's
//! [`pipeline`] / [`syntax`] / [`render`] / [`encoding`] / [`wire`]
//! modules. Depend on `aozora` alone; see the
//! [Architecture chapter of the handbook](https://p4suta.github.io/aozora/arch/pipeline.html)
//! for the layered design.

#![forbid(unsafe_code)]

pub use aozora_pipeline::{BorrowedLexOutput, NodeRef, SourceNode, lex_into_arena};
pub use aozora_render::{html, serialize};
pub use aozora_spec::{
    ALL_SENTINELS, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic,
    DiagnosticSource, INLINE_SENTINEL, InternalCheckCode, NormalizedOffset, PairKind, PairLink,
    SLUGS, Sentinel, Severity, SlugEntry, SlugFamily, SourceOffset, Span, TriggerKind,
    canonicalise_slug, codes,
};
/// Borrowed-AST node types editor surfaces match against (LSP inlay
/// hints, hover, completion, code actions, semantic tokens).
/// Re-exported so external consumers don't have to depend on
/// `aozora-syntax` directly — `aozora` is the single editor-facing
/// front door.
pub use aozora_syntax::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, ContainerKind, Indent,
    NodeKind, SectionKind,
    borrowed::{
        Annotation, AozoraHeading, AozoraNode, Bouten, Content, DoubleRuby, Gaiji, HeadingHint,
        Kaeriten, Ruby, Sashie, Segment, TateChuYoko, Warichu,
    },
};

mod document;

#[cfg(feature = "wire")]
pub mod wire;

pub use document::{AozoraTree, DiagnosticPolicy, Document, ParseOptions};

/// Re-export of [`aozora_pipeline`] under a stable name.
///
/// Editor integrations that want per-phase access
/// (`pipeline::lexer::*` for the phase functions, `pipeline::Pipeline`
/// for the type-state machine) reach through this module so the
/// wider workspace can keep `aozora` as the single front door. The
/// `aozora-pipeline` crate is `publish = false` and only callable
/// via this re-export.
pub mod pipeline {
    pub use aozora_pipeline::*;
}

/// Re-export of [`aozora_syntax`] — AST node types, arena, interner.
///
/// External callers normally reach through [`Document`] /
/// [`AozoraTree`] for the borrowed-AST surface; this module exposes
/// the underlying types when they need to construct nodes directly
/// (visitor implementations, custom renderers).
pub mod syntax {
    pub use aozora_syntax::*;
}

/// Re-export of [`aozora_render`] — HTML / serialize emitters and
/// the visitor trait.
///
/// Custom downstream renderers (EPUB, plain text, LaTeX, …)
/// implement [`syntax::borrowed::AozoraVisitor`](crate::syntax::borrowed)
/// and route through this module.
pub mod render {
    pub use aozora_render::*;
}

/// Re-export of [`aozora_encoding`] — Shift_JIS decoding and gaiji
/// resolution.
///
/// Phase 0 of the lex pipeline runs encoding detection first;
/// callers that want to drive encoding without parsing can reach
/// through this module.
pub mod encoding {
    pub use aozora_encoding::*;
}

/// Lossless concrete syntax tree.
///
/// Re-export of [`aozora_cst`] under the `cst` feature. Enables
/// editor-grade surfaces (LSP servers, source-faithful
/// refactoring / formatting tools) without pulling rowan into the
/// dep tree of plain library consumers.
///
/// ```rust,ignore
/// use aozora::Document;
/// let doc = Document::new("｜青梅《おうめ》");
/// let cst = aozora::cst::from_tree(&doc.parse());
/// // Walk the rowan SyntaxNode tree …
/// ```
#[cfg(feature = "cst")]
pub mod cst {
    pub use aozora_cst::*;

    /// Convenience wrapper over [`aozora_cst::build_cst`].
    ///
    /// Runs Phase 0 sanitize internally — `source_nodes` coordinates
    /// live in sanitized bytes, so we re-derive that text here rather
    /// than asking callers to thread it through. Sanitize is a pure
    /// function; calling it again is cheap.
    #[must_use]
    pub fn from_tree(tree: &crate::AozoraTree<'_>) -> SyntaxNode {
        use crate::pipeline::lexer::sanitize;
        let sanitized = sanitize(tree.source());
        build_cst(&sanitized.text, tree.source_nodes())
    }
}

/// Tree-sitter-flavoured pattern queries over the CST.
///
/// Re-export of [`aozora_query`] under the `query` feature.
/// Editor surfaces (`textDocument/documentHighlight`, "find all
/// ruby annotations") compose against the DSL instead of
/// re-implementing tree walks.
///
/// ```rust,ignore
/// use aozora::Document;
/// use aozora::query::compile;
///
/// let doc = Document::new("｜青梅《おうめ》");
/// let cst = aozora::cst::from_tree(&doc.parse());
/// let q = compile("(Construct @ruby)").unwrap();
/// let captures = q.captures(&cst);
/// ```
#[cfg(feature = "query")]
pub mod query {
    pub use aozora_query::*;
}

/// Aozora-shaped `proptest` strategies.
///
/// Downstream renderer / visitor authors writing their own property
/// tests reach through this module instead of pulling
/// `aozora-proptest` directly. Enabled by the `proptest` Cargo
/// feature on the `aozora` crate; both `aozora::proptest::*` and
/// the `proptest` crate itself are then in scope for the consumer.
///
/// The generators here cover the same shapes the workspace's
/// `tests/property_*` suites rely on, so any regression noticed
/// inside the parser also surfaces inside the consumer's test
/// harness.
#[cfg(feature = "proptest")]
pub mod proptest {
    pub use aozora_proptest::*;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_parse_returns_a_tree() {
        let doc = Document::new("hello, world");
        let tree = doc.parse();
        // Plain text round-trips intact.
        assert_eq!(tree.serialize(), "hello, world");
    }

    #[test]
    fn document_parse_handles_ruby() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        // Round-trip preserves the canonical form.
        assert_eq!(tree.serialize(), "｜青梅《おうめ》");
    }

    #[test]
    fn document_to_html_renders_plain_text() {
        let doc = Document::new("hello");
        let tree = doc.parse();
        let html = tree.to_html();
        assert!(html.contains("hello"), "html: {html}");
    }
}
