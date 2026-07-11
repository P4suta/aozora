//! `aozora` — the public meta crate.
//!
//! Single front door for parsing Aozora Bunko notation. Downstream
//! consumers should depend on this crate alone; everything they need
//! is re-exported through this surface or accessed via [`Document`]
//! and [`Tree`].
//!
//! ```
//! use aozora::Document;
//!
//! let doc = Document::new("｜青空《あおぞら》文庫");
//! let tree = doc.parse();
//! let html = tree.to_html();
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
//! let tree = doc.parse();
//! assert!(!tree.to_source().is_empty());
//! ```
//!
//! # Architecture
//!
//! [`Document`] owns the source buffer plus a `Copy` diagnostic
//! policy. [`Document::parse`] returns a [`Tree`] whose `&self`
//! lifetime tracks only that source borrow — the AST data itself is
//! owned, lifetime-free, and `Send + Sync` (an `LexOutput` backed
//! by a flat `NodeStore`: a string interner plus content / segment
//! pools addressed by `u32` handles). The interner deduplicates
//! repeated string content; dropping the tree frees the store in one
//! step, with no per-node `Drop`.
//!
//! The build-block crates (`aozora-spec`, `aozora-syntax`,
//! `aozora-pipeline`, `aozora-render`, `aozora-encoding`) are each
//! published in their own right, but carry **no API-stability contract
//! of their own** — they are free to churn between minor versions.
//! This umbrella is the stable seam: it re-exports a *curated* surface
//! (never a `pub use …::*` glob), so a refactor inside a build-block
//! crate cannot silently reshape what `aozora` consumers see. The
//! parsed-AST types live at the crate root ([`Document`], [`Tree`],
//! [`Node`], [`NodeRef`], [`LexOutput`], …); the [`syntax::ast`] /
//! [`render`] / [`encoding`] / [`json`] modules expose the few extra
//! types those surfaces document as stable. See the
//! [Architecture chapter of the handbook](https://p4suta.github.io/aozora/arch/pipeline.html)
//! for the layered design.
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

use core::ops::Range;

pub use aozora_pipeline::{LexOutput, NodeRef, SourceNode, lex};
pub use aozora_spec::{
    ALL_SENTINELS, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic,
    DiagnosticInfo, DiagnosticSource, INLINE_SENTINEL, InternalCheckCode, NormalizedOffset,
    PairKind, PairLink, RENDER_SLUGS, RenderSlug, SLUGS, Sentinel, Severity, SlugEntry, SlugFamily,
    SourceOffset, Span, TriggerKind, canonicalise_slug, codes, roman_slug,
};
/// Owned-AST node types editor surfaces match against (LSP inlay hints, hover,
/// completion, code actions, semantic tokens). Re-exported so external
/// consumers don't have to depend on `aozora-syntax` directly — `aozora` is the
/// single editor-facing front door.
pub use aozora_syntax::{
    BlockStyles, BoutenKind, BoutenPosition, ColumnCount, DirectiveKind, EnclosureKind, FontShift,
    Format, ForwardAttr, ForwardOrigin, GaijiCanonical, HeadingKind, HeadingStyle, IndentBlock,
    IndentLayout, Kumi, LineFormat, LineWidth, MenKuTen, NodeKind, RegionClose, RegionFormat,
    Resolved, RubySide, SectionKind,
    ast::{Content, Node, NodeStore},
};

mod diagnostics_text;
mod document;
mod incremental;
mod splice;

#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub mod json;

/// Plain-text diagnostic rendering (`miette`-free, every target).
pub use diagnostics_text::diagnostics_text;
pub use document::{DiagnosticPolicy, Document, ParseOptions, Tree};
/// Source-region ownership and minimal-diff source splicing (#202).
pub use splice::{CoupledKind, Coupling, Region, RegionRole, SpliceError, SpliceSafety};

pub use incremental::{DiagBaseRef, DiagSplice, PieceSeq, SanitizedSrc};

/// **UNSTABLE — not subject to semver until v0.5.0.**
///
/// Diagnostics-only incremental re-parse — the LSP's per-keystroke hot path
/// (#237 Tier 1/2). Splices the maintained [`PieceSeq`] (the next edit's
/// region-find base, from which the LSP flattens this edit's diagnostics) from
/// the store-free [`DiagBaseRef`] of the prior parse, **without building an
/// [`LexOutput`]** — no normalized/sanitized string rebuild, no store
/// clone/graft, no registry or container-pairs rebuild, and no whole-table
/// re-materialization. Cost is `O(region + #pieces)`: the maintained sequence is
/// spliced (prefix/suffix pieces shared by `Arc`), not rebuilt, versus a full
/// parse's `O(doc)`.
///
/// Returns [`DiagSplice`], or `None` for any edit whose locality it cannot prove
/// from the cached tables (the caller then full-parses, trivially correct); that
/// the edit truly changes only bytes inside `edit_old` is a caller precondition,
/// checked in debug rather than gated at runtime. Its diagnostics are pinned
/// byte-identical to a full parse by the `corpus_incremental_merge` differential
/// gate. The full tree is materialised lazily, only when a structural request
/// (rename) needs it.
///
/// This is exposed for the in-workspace LSP consumer only; its shape may change
/// without a major version bump until v0.5.0.
#[must_use]
#[allow(
    clippy::needless_pass_by_value,
    reason = "the lightweight DiagBaseRef (a sanitized source plus one PieceSeq borrow) is taken by value so the in-workspace LSP caller passes its temporary `DiagBaseRef { .. }` literal unchanged; it is forwarded by reference to the generic engine"
)]
pub fn reparse_incremental_diagnostics_only(
    base: DiagBaseRef<'_>,
    new_sanitized: &str,
    edit_old: Range<usize>,
) -> Option<DiagSplice> {
    incremental::reparse_incremental_diagnostics_only(&base, &new_sanitized, edit_old)
}

/// **UNSTABLE — not subject to semver until v0.5.0.**
///
/// Generic-source variant of [`reparse_incremental_diagnostics_only`]: the same
/// diagnostics-only hot path, but over any [`SanitizedSrc`] byte source `S`
/// rather than the `&str`-backed [`DiagBaseRef`]. This is the entry the
/// in-workspace LSP routes a `ropey`-backed sanitized buffer through, so it can
/// splice the rope incrementally instead of flattening it to a `String` per
/// edit; the `&str` [`reparse_incremental_diagnostics_only`] is retained for the
/// `corpus_incremental_merge` differential gate and the existing callers.
///
/// Same contract and fallbacks as [`reparse_incremental_diagnostics_only`]:
/// returns [`DiagSplice`], or `None` for any edit whose locality it cannot prove
/// from the cached tables (the caller then full-parses, trivially correct); that
/// the edit truly changes only bytes inside `edit_old` is a caller precondition,
/// checked in debug rather than gated at runtime. Like that function it is
/// exposed for the in-workspace LSP consumer only; its shape may change without a
/// major version bump until v0.5.0.
///
/// The base is taken by reference (not by value like the `&str` entry) because a
/// rope source holds a cursor and is not `Copy`; callers pass `&DiagBaseRef { .. }`.
#[must_use]
pub fn reparse_incremental_diagnostics_only_in<S: SanitizedSrc>(
    base: &DiagBaseRef<'_, S>,
    new_sanitized: &S,
    edit_old: Range<usize>,
) -> Option<DiagSplice> {
    incremental::reparse_incremental_diagnostics_only(base, new_sanitized, edit_old)
}

/// Eagerly initialise the parser's process-global lazy tables.
///
/// The *first* [`Document::parse`] then does not pay the one-time build
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
/// bulk of the cost (~150 microseconds; the `aozora-pipeline` `boot`
/// bench measures it).
///
/// ```
/// aozora::prewarm();
/// let doc = aozora::Document::new("｜青梅《おうめ》");
/// let _ = doc.parse().to_html();
/// ```
pub fn prewarm() {
    aozora_pipeline::prewarm();
}

/// Owned AST node types — [`crate::Node`] and its payload structs, the
/// [`crate::NodeStore`], `Segment` / `ContentRange` handles, and the
/// string interner — under the `syntax::ast` path.
///
/// The parsed-AST types most callers need are already at the crate
/// root (reached through [`crate::Document`] / [`crate::Tree`]); this
/// module is the explicit re-export of the underlying
/// [`aozora_syntax::ast`] surface
/// for code that constructs or walks nodes directly (custom renderers,
/// owned-tree transforms). It is a named re-export of `ast`, **not** a
/// glob of the whole no-contract `aozora-syntax` crate: the lint /
/// degraded-lowering helpers and other internals stay private to the
/// umbrella. Workspace tools that need them depend on `aozora-syntax`
/// directly.
pub mod syntax {
    pub use aozora_syntax::ast;
}

/// Rendering options for the owned-AST emitters.
///
/// [`crate::Tree::to_html`] / [`crate::Tree::to_source`] cover the
/// common cases; [`crate::Tree::to_html_with`] /
/// [`crate::Tree::to_source_with`] take the [`crate::render::RenderOptions`] /
/// [`crate::render::SerializeOptions`] re-exported here (a
/// [`crate::render::DirectiveNormalization`] level selects how an
/// `Unknown` directive is lowered). These three
/// option types are the stable render surface. The renderer *functions*
/// themselves live in the no-contract `aozora-render` crate; a
/// downstream renderer (EPUB, plain text, LaTeX, …) that drives them —
/// or reuses the byte-spelling helpers — depends on `aozora-render`
/// directly.
pub mod render {
    pub use aozora_render::{DirectiveNormalization, RenderOptions, SerializeOptions};
}

/// Shift_JIS / UTF-8 source decoding.
///
/// The parser proper is strictly UTF-8; decode a Shift_JIS archive
/// with [`crate::encoding::decode_sjis`] (force) or
/// [`crate::encoding::decode_auto`] (sniff) before handing the `String`
/// to [`crate::Document::new`]. Both are strict — they error on
/// malformed bytes rather than substituting replacement characters. The
/// [`crate::encoding::Suijun`] helpers classify a gaiji reference by its
/// JIS X 0213 level. Gaiji *resolution* is the parser's job: it is read
/// off the `Gaiji` node (see [`crate::Resolved`]), not called through
/// this module, so the resolver internals stay in the no-contract
/// `aozora-encoding` crate.
pub mod encoding {
    pub use aozora_encoding::{
        DecodeError, Suijun, decode_auto, decode_auto_into, decode_sjis, decode_sjis_into,
        has_utf8_bom, is_platform_dependent, jis_level, level_table_sizes,
    };
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
#[cfg_attr(docsrs, doc(cfg(feature = "cst")))]
pub mod cst {
    pub use aozora_cst::{AozoraLanguage, SyntaxKind, SyntaxNode, SyntaxToken, build_cst};

    /// Convenience wrapper over [`aozora_cst::build_cst`].
    ///
    /// Runs the sanitize stage internally — `source_nodes` coordinates
    /// live in sanitized bytes, so we re-derive that text here rather
    /// than asking callers to thread it through. Sanitize is a pure
    /// function; calling it again is cheap.
    #[must_use]
    pub fn from_tree(tree: &crate::Tree<'_>) -> SyntaxNode {
        use aozora_pipeline::lexer::sanitize;
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
#[cfg_attr(docsrs, doc(cfg(feature = "query")))]
pub mod query {
    pub use aozora_query::{Capture, Query, QueryError, compile};
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
#[cfg_attr(docsrs, doc(cfg(feature = "proptest")))]
pub mod proptest {
    pub use aozora_proptest::{config, generators};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_parse_returns_a_tree() {
        let doc = Document::new("hello, world");
        let tree = doc.parse();
        // Plain text round-trips intact.
        assert_eq!(tree.to_source(), "hello, world");
    }

    #[test]
    fn document_parse_handles_ruby() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        // Canonical right-side ruby is the bare form — the redundant `｜`
        // (all-kanji base at line start) is dropped (ADR 0002/0003);
        // `to_source_verbatim` preserves the author's `｜`.
        assert_eq!(tree.to_source(), "青梅《おうめ》");
        assert_eq!(tree.to_source_verbatim(), "｜青梅《おうめ》");
    }

    #[test]
    fn document_to_html_renders_plain_text() {
        let doc = Document::new("hello");
        let tree = doc.parse();
        let html = tree.to_html();
        assert!(html.contains("hello"), "html: {html}");
    }
}
