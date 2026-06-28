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
//! owned, lifetime-free, and `Send + Sync` (an `OwnedLexOutput` backed
//! by a flat `NodeStore`: a string interner plus content / segment
//! pools addressed by `u32` handles). The interner deduplicates
//! repeated string content; dropping the tree frees the store in one
//! step, with no per-node `Drop`.
//!
//! Internal build-block crates (`aozora-spec`, `aozora-syntax`,
//! `aozora-pipeline`, `aozora-render`, `aozora-encoding`) are
//! `publish = false` and reachable only through this meta crate's
//! [`pipeline`] / [`syntax`] / [`render`] / [`encoding`] / [`json`]
//! modules. Depend on `aozora` alone; see the
//! [Architecture chapter of the handbook](https://p4suta.github.io/aozora/arch/pipeline.html)
//! for the layered design.
//!
//! ---
//!
//! The project README follows; its Quickstart example is compiled and
//! run as a doctest so it can never drift from the live API.
#![allow(
    clippy::doc_markdown,
    reason = "the included README is human-facing prose; proper nouns (PyO3, x86_64, macOS, …) are intentionally not code-spanned"
)]
#![doc = include_str!("../../../README.md")]
#![forbid(unsafe_code)]

use core::ops::Range;

pub use aozora_pipeline::{NodeRefOwned, OwnedLexOutput, SourceNodeOwned, lex};
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
    BlockStyles, BoutenKind, BoutenPosition, ColumnCount, DirectiveKind, FontShift, Format,
    ForwardAttr, ForwardOrigin, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, Kumi,
    LineFormat, LineWidth, NodeKind, RegionClose, RegionFormat, RubySide, SectionKind,
    owned::{ContentOwned, NodeOwned, NodeStore},
};

mod diagnostics_text;
mod document;
mod incremental_owned;
mod splice;

#[cfg(feature = "json")]
pub mod json;

/// Plain-text diagnostic rendering (`miette`-free, every target).
pub use diagnostics_text::diagnostics_text;
pub use document::{DiagnosticPolicy, Document, ParseOptions, Tree};
/// Source-region ownership and minimal-diff source splicing (#202).
pub use splice::{CoupledKind, Coupling, OwnedRegion, RegionRole, SpliceError, SpliceSafety};

pub use incremental_owned::{
    DiagBaseRef, DiagSplice, OwnedSplice, PieceSeq, RegionIndex, SanitizedSrc,
};

/// **UNSTABLE — not subject to semver until v0.5.0.**
///
/// Owned-AST incremental re-parse entry point (#237 Stage B'): the production
/// incremental path the LSP routes its debounced diagnostics through, and the
/// surface the `corpus_incremental_merge` differential gate proves byte-for-byte
/// equivalent to a full re-parse.
///
/// This is the #237 incremental API. It is exposed deliberately for the
/// in-workspace LSP consumer but its shape (the [`OwnedSplice`] result, the
/// sanitized-coordinate contract) may change without a major version bump until
/// the v0.5.0 normalization-waist release stabilises it; external callers must
/// not depend on it.
///
/// Builds the [`OwnedSplice`] (the spliced [`OwnedLexOutput`] plus reuse counts)
/// for `new_sanitized` (a sanitized fixed point) from `cached` and the single
/// sanitized-coordinate edit `edit_old`, by re-lexing only the minimal balanced
/// region around the edit and splicing the owned tables. Returns `None` for any
/// edit whose locality it cannot prove from the cached tables (the caller then
/// full-parses, trivially correct); that the edit truly changes only bytes
/// inside `edit_old` is a caller precondition, checked in debug rather than
/// gated at runtime.
#[must_use]
pub fn reparse_incremental_owned(
    cached: &OwnedLexOutput,
    new_sanitized: &str,
    edit_old: Range<usize>,
) -> Option<OwnedSplice> {
    incremental_owned::reparse_incremental_owned(cached, new_sanitized, edit_old)
}

/// **UNSTABLE — not subject to semver until v0.5.0.**
///
/// Diagnostics-only incremental re-parse — the LSP's per-keystroke hot path
/// (#237 Tier 1/2). Splices the maintained [`PieceSeq`] (the next edit's
/// region-find base, from which the LSP flattens this edit's diagnostics) from
/// the store-free [`DiagBaseRef`] of the prior parse, **without building an
/// [`OwnedLexOutput`]** — no normalized/sanitized string rebuild, no store
/// clone/graft, no registry or container-pairs rebuild, and no whole-table
/// re-materialization or `RegionIndex` rebuild. Cost is `O(region + #pieces)`:
/// the maintained sequence is spliced (prefix/suffix pieces shared by `Arc`),
/// not rebuilt, versus the owned splice's `O(doc)`.
///
/// Returns [`DiagSplice`], or `None` for any edit whose locality it cannot prove
/// from the cached tables (the caller then full-parses, trivially correct); that
/// the edit truly changes only bytes inside `edit_old` is a caller precondition,
/// checked in debug rather than gated at runtime. Its diagnostics are
/// byte-identical to [`reparse_incremental_owned`]'s — the
/// `corpus_incremental_merge` differential gate pins both engines together and
/// to a full parse. The full tree is materialised lazily, only when a
/// structural request (rename) needs it.
///
/// Like [`reparse_incremental_owned`], this is exposed for the in-workspace LSP
/// consumer only; its shape may change without a major version bump until
/// v0.5.0.
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
    incremental_owned::reparse_incremental_diagnostics_only(&base, &new_sanitized, edit_old)
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
    incremental_owned::reparse_incremental_diagnostics_only(base, new_sanitized, edit_old)
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

/// Re-export of [`aozora_syntax`] — owned AST node types, the
/// `NodeStore`, and the string interner.
///
/// External callers normally reach through [`Document`] /
/// [`Tree`] for the parsed-AST surface; this module exposes
/// the underlying types when they need to construct nodes directly
/// (custom renderers, owned-tree transforms).
pub mod syntax {
    pub use aozora_syntax::*;
}

/// Re-export of [`aozora_render`] — owned-AST HTML / source emitters.
///
/// `Tree::to_html` / `Tree::to_source` cover the common cases; custom
/// downstream renderers (EPUB, plain text, LaTeX, …) walk the owned
/// `OwnedLexOutput` (its `source_nodes` + `NodeStore`) and can reuse the
/// shared byte-spelling helpers re-exported through this module.
pub mod render {
    pub use aozora_render::*;
}

/// Re-export of [`aozora_encoding`] — Shift_JIS decoding and gaiji
/// resolution.
///
/// The sanitize stage of the lex pipeline runs encoding detection
/// first; callers that want to drive encoding without parsing can reach
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
    /// Runs the sanitize stage internally — `source_nodes` coordinates
    /// live in sanitized bytes, so we re-derive that text here rather
    /// than asking callers to thread it through. Sanitize is a pure
    /// function; calling it again is cheap.
    #[must_use]
    pub fn from_tree(tree: &crate::Tree<'_>) -> SyntaxNode {
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
