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
//! The parse/render chain (`spec`, `syntax`, `scan`, `encoding`,
//! `collections`, `pipeline`, `render`) lives as private modules inside
//! this crate; the crate root re-exports a *curated* surface (never a
//! `pub use …::*` glob), so the internal layering can be refactored
//! without reshaping what `aozora` consumers see. The parsed-AST types
//! live at the crate root ([`Document`], [`Tree`], [`Node`], [`NodeRef`],
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

#[cfg(feature = "unstable-internals")]
use core::ops::Range;

// The core parse/render chain. Leaf primitives (`spec`, `collections`,
// `scan`) and the lex `pipeline` stay private; `encoding` / `syntax` /
// `render` carry the curated facade surface (see their re-exports below).
mod collections;
mod pipeline;
mod scan;
mod spec;

pub mod encoding;
pub mod render;
pub mod syntax;

/// Canonical diagnostic / span / pair types — the single definitions live in
/// the crate-internal `spec` module; re-exported here as the stable crate-root
/// facade.
pub use crate::spec::{
    Diagnostic, DiagnosticInfo, DiagnosticSource, NormalizedOffset, PairKind, PairLink, Severity,
    SourceOffset, Span,
};
/// Owned-AST node types editor surfaces match against (hover, completion,
/// code actions, semantic tokens), plus the shared `Copy` style/format enums.
pub use crate::syntax::{
    BlockStyles, BoutenKind, BoutenPosition, ColumnCount, DirectiveKind, EnclosureKind, FontShift,
    Format, ForwardAttr, ForwardOrigin, GaijiCanonical, HeadingKind, HeadingStyle, IndentBlock,
    IndentLayout, Kumi, LineFormat, LineWidth, MenKuTen, NodeKind, RegionClose, RegionFormat,
    Resolved, RubySide, SectionKind,
    ast::{Content, Node, NodeRef, NodeStore},
};

/// **UNSTABLE — no semver contract.** The internal parse-stage surface plus
/// the sentinel / slug / classifier tables the in-workspace aozora-cli /
/// aozora-bench / aozora-xtask consumers reach for. Hidden from
/// docs (the serde `__private` model): always compiled but never advertised and
/// carrying no stability contract, so it stays off the *documented* minimal
/// facade while keeping the demoted-off-root items reachable (`dead_code`
/// satisfied in every configuration). The heavier, seal-bearing incremental
/// engine is the one piece that is additionally gated behind the
/// `unstable-internals` feature (see below).
#[doc(hidden)]
pub mod unstable {
    pub use crate::pipeline::lexer::{classify, pair, sanitize, token, tokenize};
    pub use crate::pipeline::{Paired, Pipeline, Sanitized, Source, Tokenized, lex};
    pub use crate::scan::NaiveScanner;
    pub use crate::spec::{
        ALL_SENTINELS, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL,
        INLINE_SENTINEL, InternalCheckCode, RENDER_SLUGS, RenderSlug, SLUGS, Sentinel, SlugEntry,
        SlugFamily, TriggerKind, canonicalise_slug, classify_trigger_bytes, codes, roman_slug,
    };
    pub use crate::syntax::alloc::Allocator;
    pub use crate::syntax::ast::InternStats;
    pub use crate::syntax::ast::{LexOutput, SourceNode};
    pub use crate::syntax::{degraded, lint};
}

/// **UNSTABLE — no semver contract.** Classify-stage timing instrumentation.
/// The read side (`TimingTable::snapshot`,
/// `Subsystem::ordered`, `snapshot_replay_sizes`, …) is consumed by the
/// in-workspace `aozora-bench` probes; the record side is compiled into the
/// classify hot path only under this feature (dead code otherwise). Hidden
/// from docs and gated behind `classify-instrument`.
#[cfg(feature = "classify-instrument")]
#[doc(hidden)]
pub use crate::pipeline::lexer::instrumentation;

mod diagnostics_text;
mod document;
#[cfg(feature = "unstable-internals")]
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

/// **UNSTABLE — no semver contract.** The diagnostics-only incremental
/// re-parse engine (#237), gated behind `unstable-internals` and hidden from
/// docs; the in-workspace LSP enables the feature and drives it.
#[cfg(feature = "unstable-internals")]
#[doc(hidden)]
pub use incremental::{DiagBaseRef, DiagSplice, PieceSeq, SanitizedSrc, SanitizedSrcSealed};

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
#[cfg(feature = "unstable-internals")]
#[doc(hidden)]
#[must_use]
#[expect(
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
#[cfg(feature = "unstable-internals")]
#[doc(hidden)]
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
/// bulk of the cost (~150 microseconds).
///
/// ```
/// aozora::prewarm();
/// let doc = aozora::Document::new("｜青梅《おうめ》");
/// let _ = doc.parse().to_html();
/// ```
pub fn prewarm() {
    pipeline::prewarm();
}

/// Source-canonicalising formatter algorithm.
///
/// The `parse ∘ to_source` round trip; its output byte-identity is inherited
/// from [`Document::parse`] + [`Tree::to_source_with`]. The CLI plumbing that
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
/// let cst = aozora::cst::from_tree(&Document::new("｜青梅《おうめ》").parse());
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

    /// A three-paragraph document with ruby in the outer paragraphs and a plain
    /// middle paragraph, plus a purely-local plain-text insertion into that
    /// middle paragraph, and the cached region-find base over it. Such an edit
    /// satisfies every splice precondition, so the diagnostics-only reparse must
    /// accept it (return `Some`) rather than fall back — the input that separates
    /// the live wrappers from a `-> None` mutation of their bodies.
    #[cfg(feature = "unstable-internals")]
    fn local_edit_fixture() -> (syntax::ast::LexOutput, String, usize) {
        let cached = Document::new("｜青梅《おうめ》\n\nかきくけこ\n\n｜山川《やまかわ》\n").lex();
        let san = cached.sanitized.clone();
        let at = san.find("かき").expect("plain middle paragraph") + "かき".len();
        let mut new_san = String::with_capacity(san.len() + 3);
        new_san.push_str(&san[..at]);
        new_san.push('が');
        new_san.push_str(&san[at..]);
        (cached, new_san, at)
    }

    #[cfg(feature = "unstable-internals")]
    #[test]
    fn reparse_incremental_diagnostics_only_splices_a_local_edit() {
        let (cached, new_san, at) = local_edit_fixture();
        let pieces = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        );
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        let cached_nodes = u64::try_from(cached.source_nodes.len()).expect("node count fits u64");
        // The outer ruby paragraphs must supply nodes for the reuse accounting
        // below to be non-degenerate.
        assert!(cached_nodes > 0, "fixture must carry ruby nodes to reuse");

        // The `&str` wrapper takes the base by value and forwards it to the
        // engine; a `-> None` mutation of the wrapper body would drop this
        // successful splice on the floor.
        let splice = reparse_incremental_diagnostics_only(base, new_san.as_str(), at..at)
            .expect("a purely-local plain-text edit must splice, not fall back");
        // The plain middle paragraph re-lexes to zero nodes, so every cached node
        // is carried unchanged (prefix + shifted suffix).
        assert_eq!(splice.relexed_nodes, 0);
        assert_eq!(splice.reused_nodes, cached_nodes);
        assert_eq!(splice.pieces.node_count(), cached_nodes);
    }

    #[cfg(feature = "unstable-internals")]
    #[test]
    fn reparse_incremental_diagnostics_only_in_splices_a_local_edit() {
        let (cached, new_san, at) = local_edit_fixture();
        let pieces = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        );
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        let cached_nodes = u64::try_from(cached.source_nodes.len()).expect("node count fits u64");
        assert!(cached_nodes > 0, "fixture must carry ruby nodes to reuse");

        // The generic `_in` wrapper takes the base by reference (the rope-source
        // entry) but forwards to the same engine; over the `&str` `SanitizedSrc`
        // impl it must accept the identical local edit. A `-> None` mutation of
        // the wrapper body would drop the splice.
        let splice = reparse_incremental_diagnostics_only_in(&base, &new_san.as_str(), at..at)
            .expect("a purely-local plain-text edit must splice, not fall back");
        assert_eq!(splice.relexed_nodes, 0);
        assert_eq!(splice.reused_nodes, cached_nodes);
        assert_eq!(splice.pieces.node_count(), cached_nodes);
    }
}
