//! `aozora` — the public meta crate.
//!
//! Single front door for parsing Aozora Bunko notation. Downstream
//! consumers should depend on this crate alone; everything they need
//! is re-exported through this surface or accessed via [`Document`]
//! and [`Snapshot`].
//!
//! ```
//! use aozora::Document;
//!
//! let doc = Document::new("｜青空《あおぞら》文庫");
//! let snapshot = doc.snapshot();
//! let html = snapshot.to_html();
//! assert!(html.contains("青空")); // the ruby base survives into the HTML
//! ```
//!
//! Tunable parses go through the builder chain:
//!
//! ```
//! use aozora::{Document, DiagnosticPolicy};
//!
//! let doc = Document::options()
//!     .diagnostic_policy(DiagnosticPolicy::DropInternal)
//!     .build("｜青梅《おうめ》");
//! let snapshot = doc.snapshot();
//! assert!(!snapshot.to_source().is_empty());
//! ```
//!
//! # Architecture
//!
//! [`Document`] owns the source and incremental parse state.
//! [`Document::snapshot`] returns an immutable, owned [`Snapshot`] that
//! is `Clone + Send + Sync`.
//!
//! The parse/render chain (`spec`, `syntax`, `scan`, `encoding`,
//! `collections`, `pipeline`, `render`) lives as private modules inside
//! this crate; the crate root re-exports a *curated* surface (never a
//! `pub use …::*` glob), so the internal layering can be refactored
//! without reshaping what `aozora` consumers see. The parsed-AST types
//! live at the crate root ([`Document`], [`Snapshot`], [`Node`], [`NodeRef`],
//! …); the [`syntax::ast`] / [`render`] / [`encoding`] / [`json`] modules
//! expose the few extra types those surfaces document as stable.
//!
//! ---
//!
//! The crate README follows; its Quickstart example is compiled as a
//! doctest so it can never drift from the live API.
#![allow(
    clippy::doc_markdown,
    reason = "the included README is human-facing prose; proper nouns (PyO3, x86_64, macOS, …) are intentionally not code-spanned"
)]
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
// Emit "Available on crate feature `…`" badges on docs.rs (and the
// GitHub-Pages build when RUSTDOCFLAGS carries `--cfg docsrs`). Inert on
// stable — `docsrs` is unset, so this never trips `feature(doc_cfg)`.
#![cfg_attr(docsrs, feature(doc_cfg))]

// The core parse/render chain. Leaf primitives (`spec`, `collections`,
// `scan`) and the lex `pipeline` stay private; `encoding` / `syntax` /
// `render` carry the curated facade surface (see their re-exports below).
mod catalogue;
mod collections;
mod pipeline;
mod scan;
mod spec;

pub mod encoding;
pub mod render;
pub mod syntax;

pub use catalogue::{Catalogue, CatalogueMatch};

/// Canonical diagnostic / span / pair types — the single definitions live in
/// the crate-internal `spec` module; re-exported here as the stable crate-root
/// facade.
pub use crate::spec::{
    Diagnostic, DiagnosticInfo, DiagnosticSource, InternalCheckCode, NormalizedOffset, PairKind,
    PairLink, RENDER_SLUGS, RenderSlug, Severity, SourceOffset, Span,
};
pub use crate::spec::{SlugEntry as CatalogueEntry, SlugFamily as CatalogueFamily};
/// Owned-AST node types editor surfaces match against (hover, completion,
/// code actions, semantic tokens), plus the shared `Copy` style/format enums.
pub use crate::syntax::{
    BlockStyles, BoutenKind, BoutenPosition, ColumnCount, DirectiveKind, EnclosureKind, FontShift,
    Format, ForwardAttr, ForwardOrigin, GaijiCanonical, HeadingKind, HeadingStyle, IndentBlock,
    IndentLayout, Kumi, LineFormat, LineWidth, MenKuTen, NodeKind, RegionClose, RegionFormat,
    Resolved, RubySide, SectionKind,
    ast::{Content, Node, NodeRef},
};

mod diagnostics_text;
mod document;
mod splice;

#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub mod json;

/// Plain-text diagnostic rendering (`miette`-free, every target).
pub use diagnostics_text::diagnostics_text;
pub use document::{
    DiagnosticPolicy, Document, EditError, ParseOptions, Parser, Snapshot, TextEdit,
};
/// Source-region ownership and minimal-diff source splicing (#202).
pub use splice::{CoupledKind, Coupling, Region, RegionRole, SpliceError, SpliceSafety};

/// Parse source with default settings.
#[must_use]
pub fn parse(source: impl Into<Box<str>>) -> Document {
    Parser::new().parse(source)
}

/// Eagerly initialise the parser's process-global lazy tables.
///
/// The *first* [`Document::snapshot`] then does not pay the one-time build
/// cost on its critical path.
///
/// Parsing is lazy by default: a consumer that never parses — or parses
/// no annotations — pays nothing. `prewarm` is **opt-in**. Call it once,
/// early, from a latency-sensitive front end (e.g. a WASM editor warming
/// the parser before the first keystroke). It is idempotent and
/// thread-safe; redundant calls are effectively free.
///
/// It warms the SIMD trigger-scan backend selection (tokenize stage) and
/// the annotation-classifier Aho-Corasick DFA (classify stage) — the latter is the
/// bulk of the cost (~150 microseconds).
///
/// ```
/// aozora::prewarm();
/// let doc = aozora::Document::new("｜青梅《おうめ》");
/// let _ = doc.snapshot().to_html();
/// ```
pub fn prewarm() {
    pipeline::prewarm();
}

/// Source-canonicalising formatter algorithm.
///
/// The `parse ∘ to_source` round trip; its output byte-identity is inherited
/// from [`Document::snapshot`] + [`Snapshot::to_source_with`]. The CLI plumbing that
/// wraps it lives in `aozora-cli`.
#[cfg(feature = "fmt")]
#[cfg_attr(docsrs, doc(cfg(feature = "fmt")))]
pub mod fmt;

/// Pandoc AST projection — emit a [`pandoc_ast::Pandoc`] document consumable by
/// every Pandoc output format (HTML, EPUB, LaTeX/PDF, DOCX, …).
#[cfg(feature = "pandoc")]
#[cfg_attr(docsrs, doc(cfg(feature = "pandoc")))]
pub mod pandoc;

/// Lossless concrete syntax tree.
///
/// A rowan-backed `SyntaxNode` projection under the `cst` feature. Enables
/// editor-grade surfaces (LSP servers, source-faithful refactoring / formatting
/// tools) without pulling rowan into the dep tree of plain library consumers.
///
/// ```
/// use aozora::Document;
/// let cst = aozora::cst::from_snapshot(&Document::new("｜青梅《おうめ》").snapshot());
/// // Walk the rowan SyntaxNode tree …
/// assert_eq!(cst.kind(), aozora::cst::SyntaxKind::Document);
/// ```
#[cfg(feature = "cst")]
#[cfg_attr(docsrs, doc(cfg(feature = "cst")))]
pub mod cst;

/// Tree-sitter-flavoured pattern queries over the CST.
///
/// A tiny selector DSL under the `query` feature (implies `cst`). Editor
/// surfaces (`textDocument/documentHighlight`, "find all ruby annotations")
/// compose against the DSL instead of re-implementing tree walks.
#[cfg(feature = "query")]
#[cfg_attr(docsrs, doc(cfg(feature = "query")))]
pub mod query;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_parse_returns_a_tree() {
        let doc = Document::new("hello, world");
        let tree = doc.snapshot();
        // Plain text round-trips intact.
        assert_eq!(tree.to_source(), "hello, world");
    }

    #[test]
    fn document_parse_handles_ruby() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.snapshot();
        // Canonical right-side ruby is the bare form — the redundant `｜`
        // (all-kanji base at line start) is dropped (ADR 0002/0003);
        // `to_source_verbatim` preserves the author's `｜`.
        assert_eq!(tree.to_source(), "青梅《おうめ》");
        assert_eq!(tree.to_source_verbatim(), "｜青梅《おうめ》");
    }

    #[test]
    fn document_to_html_renders_plain_text() {
        let doc = Document::new("hello");
        let tree = doc.snapshot();
        let html = tree.to_html();
        assert!(html.contains("hello"), "html: {html}");
    }
}
