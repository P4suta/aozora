//! The lexer's owned, no-lifetime output.
//!
//! [`SourceNode`] pairs a sanitized-source span with the node it
//! classified there; [`LexOutput`] holds the lexer's output, with a
//! `store: NodeStore` that backs the `StrId`/range payloads. The whole struct
//! is `Send + Sync` (static assertion below) — the point of the owned
//! representation for the #237 incremental cache / LSP consumer.

use aozora_spec::{Diagnostic, PairLink, SourceOffset, Span};

use super::intern::InternStats;
use super::registry::{ContainerPair, NodeRef, Registry};
use super::store::NodeStore;

/// Source-keyed registry entry.
///
/// Pairs a sanitized-source byte span with the classified node landed there.
/// Derives `Debug, Clone, Copy`; deliberately no `PartialEq`/`Eq`. `Copy`
/// requires [`NodeRef`] be `Copy`.
#[derive(Debug, Clone, Copy)]
pub struct SourceNode {
    /// Half-open byte range, in sanitized-source coordinates, this node was
    /// classified from. Entries are sorted by `start`. (`Span` reused.)
    pub source_span: Span,
    /// The classified node landed at `source_span`, tagged with where it sits
    /// in the normalized stream.
    pub node: NodeRef,
}

/// The lexer's complete owned, no-lifetime output.
///
/// Every field is owned (`String`, [`Registry`], `Vec<_>`), with a
/// `store: NodeStore` that backs the `StrId`/range payloads referenced by the
/// owned nodes. `Send + Sync` (see static assertion below). Not `Copy`.
/// Derives `Debug` only.
#[derive(Debug)]
#[non_exhaustive]
pub struct LexOutput {
    /// Normalized text with PUA sentinels.
    pub normalized: String,
    /// Verbatim post-sanitize source text (no sentinels, no padding) — the
    /// coordinate space every `source_span` indexes.
    pub sanitized: String,
    /// Sentinel-position → node lookup table.
    pub registry: Registry,
    /// Non-fatal observations from every stage.
    pub diagnostics: Vec<Diagnostic>,
    /// Byte length of the sanitize-stage buffer.
    pub sanitized_len: u32,
    /// Resolved (open, close) delimiter pairs in sanitized-source coordinates,
    /// close order.
    pub pairs: Vec<PairLink>,
    /// Source-keyed node side-table, sorted by `source_span.start`.
    pub source_nodes: Vec<SourceNode>,
    /// Resolved container open/close pairs in normalized coordinates.
    pub container_pairs: Vec<ContainerPair>,
    /// Interner dedup/probe counters.
    pub intern_stats: InternStats,
    /// Owned backing store (string interner + flat content/segment `Vec`s) the
    /// owned nodes' `StrId`/range payloads resolve against.
    pub store: NodeStore,
}

impl LexOutput {
    /// Assemble an [`LexOutput`] from its already-owned field set.
    ///
    /// The only constructor for this `#[non_exhaustive]` struct reachable from
    /// outside `aozora-syntax` — the pipeline's native owned producer
    /// (`aozora_pipeline::lex` / `Pipeline::build`) builds the
    /// [`Registry`], [`SourceNode`] table, and [`NodeStore`] (the
    /// classify stage allocates owned nodes directly into the store via
    /// `Allocator`), then hands the whole field set here. Every argument
    /// maps to the identically-named field.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "constructs the non_exhaustive LexOutput from its complete already-owned field set; a parameter object would only restate the field set"
    )]
    pub fn new(
        normalized: String,
        sanitized: String,
        registry: Registry,
        diagnostics: Vec<Diagnostic>,
        sanitized_len: u32,
        pairs: Vec<PairLink>,
        source_nodes: Vec<SourceNode>,
        container_pairs: Vec<ContainerPair>,
        intern_stats: InternStats,
        store: NodeStore,
    ) -> Self {
        Self {
            normalized,
            sanitized,
            registry,
            diagnostics,
            sanitized_len,
            pairs,
            source_nodes,
            container_pairs,
            intern_stats,
            store,
        }
    }

    /// Find the [`SourceNode`] whose `source_span` covers `src_off`
    /// (a sanitized-source byte offset). O(log n) binary search.
    #[must_use]
    pub fn node_at_source(&self, src_off: SourceOffset) -> Option<&SourceNode> {
        let raw = src_off.get();
        let idx = self
            .source_nodes
            .partition_point(|entry| entry.source_span.start <= raw);
        if idx == 0 {
            return None;
        }
        let candidate = &self.source_nodes[idx - 1];
        (raw < candidate.source_span.end).then_some(candidate)
    }
}

/// Required static assertion: the owned lex output crosses thread boundaries
/// (the whole point of the owned representation for the #237 LSP consumer).
const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LexOutput>();
};
