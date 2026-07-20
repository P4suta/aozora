//! `aozora` — the public meta crate.
//!
//! Single front door for parsing Aozora Bunko notation. Downstream
//! consumers should depend on this crate alone; everything they need
//! is re-exported through this surface or accessed via [`Document`]
//! and [`Snapshot`].
//!
//! ```
//! use aozora::parse;
//!
//! let doc = parse("｜青空《あおぞら》文庫")?;
//! let snapshot = doc.snapshot();
//! let html = snapshot.to_html();
//! assert!(html.contains("青空")); // the ruby base survives into the HTML
//! # Ok::<(), aozora::ParseError>(())
//! ```
//!
//! # Architecture
//!
//! [`Document`] owns the source and incremental parse state.
//! [`Document::snapshot`] returns an immutable, owned [`Snapshot`] that
//! is `Clone + Send + Sync`.
//!
//! The parsing and rendering implementation is private. Consumers use the
//! crate-root façade: [`Parser`], [`Document`], immutable [`Snapshot`],
//! original-source spans and stable projection views. The feature-gated
//! [`json`] module is the only public low-level wire surface.
//!
//! ---
//!
//! The crate README follows; its Quickstart example is compiled as a
//! doctest so it can never drift from the live API.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
// Emit "Available on crate feature `…`" badges on docs.rs (and the
// GitHub-Pages build when RUSTDOCFLAGS carries `--cfg docsrs`). Inert on
// stable — `docsrs` is unset, so this never trips `feature(doc_cfg)`.
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::sync::Arc;

mod catalogue;
mod collections;
mod pipeline;
mod scan;
mod spec;

mod encoding;
mod incremental;
mod render;
mod syntax;

pub use catalogue::{Catalogue, CatalogueMatch};
pub use encoding::gaiji::GaijiResolution;
pub use encoding::{
    DecodeError, decode_auto, decode_auto_into, decode_sjis, decode_sjis_into, has_utf8_bom,
};
pub use render::{AOZORA_CLASSES, DirectiveNormalization, RenderOptions, SerializeOptions};

/// Canonical diagnostic / span / pair types — the single definitions live in
/// the crate-internal `spec` module; re-exported here as the stable crate-root
/// facade.
pub use crate::spec::{
    Diagnostic, DiagnosticInfo, DiagnosticSource, InternalCheckCode, PairKind, PairLink, Severity,
    Span,
};
pub use crate::spec::{SlugEntry as CatalogueEntry, SlugFamily as CatalogueFamily};
pub use crate::syntax::{NodeKind, RubySide};

mod diagnostics_text;
mod document;
mod splice;

#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub mod json;

/// Plain-text diagnostic rendering (`miette`-free, every target).
pub use diagnostics_text::diagnostics_text;
pub use document::{
    ContainerKind, ContainerPair, DirectiveClass, DirectiveView, Document, EditError,
    LiteralMarkupKind, LiteralMarkupView, NodeView, ParseError, Parser, RubyView, Snapshot,
    TextEdit,
};

/// Parse source with default settings.
///
/// # Errors
///
/// Returns [`ParseError::SourceTooLarge`] when the source cannot be represented
/// by the parser's byte spans.
pub fn parse(source: impl Into<Arc<str>>) -> Result<Document, ParseError> {
    Parser::new().parse(source)
}

/// Resolve one gaiji description and optional mencode to a glyph sequence.
#[must_use]
pub fn resolve_gaiji(mencode: Option<&str>, description: &str) -> Option<String> {
    encoding::gaiji::lookup(None, mencode, description).map(|resolved| {
        let mut value = String::new();
        _ = resolved.write_to(&mut value);
        value
    })
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
/// let doc = aozora::parse("｜青梅《おうめ》")?;
/// let _ = doc.snapshot().to_html();
/// # Ok::<(), aozora::ParseError>(())
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
    fn public_gaiji_resolution_returns_exact_glyphs() {
        assert_eq!(resolve_gaiji(None, "々").as_deref(), Some("々"));
        assert_eq!(
            resolve_gaiji(Some("第3水準1-85-54"), "木＋吶のつくり").as_deref(),
            Some("枘")
        );
        assert_eq!(resolve_gaiji(None, "未知の字形"), None);
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
