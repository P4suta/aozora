//! Owned, no-lifetime mirror of the pipeline's `LexOutput`.
//!
//! [`SourceNodeOwned`] mirrors `aozora_pipeline::borrowed::SourceNode`;
//! [`OwnedLexOutput`] mirrors `aozora_pipeline::borrowed::LexOutput`
//! field-for-field, with every arena-borrowed field owned and an added
//! `store: NodeStore` that backs the `StrId`/range payloads. The whole struct
//! is `Send + Sync` (static assertion below) — the point of the owned mirror
//! for the #237 segment-cache / LSP consumer.

use aozora_spec::{Diagnostic, PairLink, SourceOffset, Span};

use super::registry::{NodeRefOwned, RegistryOwned};
use super::store::NodeStore;
use crate::borrowed::{ContainerPair, InternStats};

/// Source-keyed registry entry — owned mirror of
/// `aozora_pipeline::borrowed::SourceNode`.
///
/// Pairs a sanitized-source byte span with the classified node landed there.
/// Mirrors the borrowed derive set exactly (`Debug, Clone, Copy`; no
/// `PartialEq`/`Eq`). `Copy` requires [`NodeRefOwned`] be `Copy`.
#[derive(Debug, Clone, Copy)]
pub struct SourceNodeOwned {
    /// Half-open byte range, in sanitized-source coordinates, this node was
    /// classified from. Entries are sorted by `start`. (`Span` reused.)
    pub source_span: Span,
    /// The classified node landed at `source_span`, tagged with where it sits
    /// in the normalized stream. Mirrors `SourceNode::node`.
    pub node: NodeRefOwned,
}

/// Owned, no-lifetime mirror of `aozora_pipeline::borrowed::LexOutput`.
///
/// Every arena-borrowed field is owned: `&str` => `String`, `Registry<'a>` =>
/// [`RegistryOwned`], `&'a [T]` => `Vec<T_owned>`. Adds a `store: NodeStore`
/// that backs the `StrId`/range payloads referenced by the owned nodes.
/// `Send + Sync` (see static assertion below). Not `Copy`. Mirrors
/// `LexOutput`'s derive set: `Debug` only.
#[derive(Debug)]
#[non_exhaustive]
pub struct OwnedLexOutput {
    /// Normalized text with PUA sentinels. `LexOutput::normalized` (`&'a str`).
    pub normalized: String,
    /// Verbatim post-sanitize source text (no sentinels, no padding) — the
    /// coordinate space every `source_span` indexes. `LexOutput::sanitized`
    /// (`&'a str`).
    pub sanitized: String,
    /// Sentinel-position → node lookup table. `LexOutput::registry`
    /// (`Registry<'a>`).
    pub registry: RegistryOwned,
    /// Non-fatal observations from every stage. `LexOutput::diagnostics`
    /// (already an owned `Vec<Diagnostic>`; reused verbatim).
    pub diagnostics: Vec<Diagnostic>,
    /// Byte length of the sanitize-stage buffer. `LexOutput::sanitized_len`
    /// (`u32`).
    pub sanitized_len: u32,
    /// Resolved (open, close) delimiter pairs in sanitized-source coordinates,
    /// close order. `LexOutput::pairs` (`&'a [PairLink]`).
    pub pairs: Vec<PairLink>,
    /// Source-keyed node side-table, sorted by `source_span.start`.
    /// `LexOutput::source_nodes` (`&'a [SourceNode<'a>]`).
    pub source_nodes: Vec<SourceNodeOwned>,
    /// Resolved container open/close pairs in normalized coordinates.
    /// `LexOutput::container_pairs` (`&'a [ContainerPair]`).
    pub container_pairs: Vec<ContainerPair>,
    /// Interner dedup/probe counters. `LexOutput::intern_stats`
    /// (`InternStats`; reused verbatim).
    pub intern_stats: InternStats,
    /// Owned backing store (string interner + flat content/segment `Vec`s) the
    /// owned nodes' `StrId`/range payloads resolve against. NEW field with no
    /// `LexOutput` analogue — it owns what the arena owned in the borrowed
    /// pipeline.
    pub store: NodeStore,
}

impl OwnedLexOutput {
    /// Assemble an [`OwnedLexOutput`] from its already-owned field set.
    ///
    /// The only constructor for this `#[non_exhaustive]` struct reachable from
    /// outside `aozora-syntax` — the pipeline-side converter
    /// (`aozora_pipeline::LexOutput::to_owned`) builds the [`RegistryOwned`],
    /// [`SourceNodeOwned`] table, and [`NodeStore`] (via the `from_borrowed`
    /// mappers), then hands the whole field set here. Every argument maps to
    /// the identically-named field; see the struct docs for their borrowed
    /// `LexOutput` analogues.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "constructs the non_exhaustive OwnedLexOutput from its complete already-owned field set; a parameter object would only re-mirror the struct"
    )]
    pub fn new(
        normalized: String,
        sanitized: String,
        registry: RegistryOwned,
        diagnostics: Vec<Diagnostic>,
        sanitized_len: u32,
        pairs: Vec<PairLink>,
        source_nodes: Vec<SourceNodeOwned>,
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

    /// Find the [`SourceNodeOwned`] whose `source_span` covers `src_off`
    /// (a sanitized-source byte offset). O(log n) binary search. Mirror of
    /// `LexOutput::node_at_source`.
    #[must_use]
    pub fn node_at_source(&self, src_off: SourceOffset) -> Option<&SourceNodeOwned> {
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
/// (the whole point of the owned mirror for the #237 LSP consumer).
const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OwnedLexOutput>();
};
