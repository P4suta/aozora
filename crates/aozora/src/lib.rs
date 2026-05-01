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
//! ## Architecture
//!
//! [`Document`] owns the source buffer plus a [`bumpalo`]-backed
//! arena. [`AozoraTree`] borrows from that arena via the `&self`
//! lifetime returned by [`Document::parse`]. Every per-node
//! allocation lives inside the arena, with the
//! [`Interner`](aozora_syntax::borrowed::Interner) deduplicating
//! repeated string content; dropping the `Document` releases the
//! entire tree in a single `Bump::reset` step. See
//! the [Architecture chapter of the handbook](https://p4suta.github.io/aozora/arch/pipeline.html)
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
/// wider workspace can keep [`aozora`] as the single front door.
///
/// Pre-Phase-G consumers depended on the `aozora-pipeline` crate
/// directly; that crate is now `publish = false` and only callable
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
