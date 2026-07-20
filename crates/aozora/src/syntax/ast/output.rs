//! The lexer's owned, no-lifetime output.
//!
//! [`SourceNode`] pairs a sanitized-source span with the node it
//! classified there; [`LexOutput`] holds the lexer's output, with a
//! shared [`NodeStore`] that backs the `StrId`/range payloads. The whole struct
//! is `Send + Sync` (static assertion below) — the point of the owned
//! representation for the #237 incremental cache / LSP consumer.

use core::ops::Deref;
use std::sync::Arc;

use crate::spec::{Diagnostic, NormalizedOffset, PairLink, SourceOffset, Span};

use super::registry::{ContainerPair, NodeRef, Registry};
use super::store::NodeStore;

#[derive(Debug, Clone)]
pub(crate) enum SanitizedText {
    Shared(Arc<str>),
    Owned(String),
}

impl SanitizedText {
    pub(crate) fn shared(text: Arc<str>) -> Self {
        Self::Shared(text)
    }

    pub(crate) fn owned(text: String) -> Self {
        Self::Owned(text)
    }
}

impl Deref for SanitizedText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(text) => text,
            Self::Owned(text) => text,
        }
    }
}

impl AsRef<str> for SanitizedText {
    fn as_ref(&self) -> &str {
        self
    }
}

impl PartialEq for SanitizedText {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for SanitizedText {}

impl From<String> for SanitizedText {
    fn from(text: String) -> Self {
        Self::Owned(text)
    }
}

impl From<Arc<str>> for SanitizedText {
    fn from(text: Arc<str>) -> Self {
        Self::Shared(text)
    }
}

impl From<&str> for SanitizedText {
    fn from(text: &str) -> Self {
        Self::Owned(text.to_owned())
    }
}

/// Source-keyed registry entry.
///
/// Pairs a sanitized-source byte span with the classified node landed there.
/// Derives `Debug, Clone, Copy`; deliberately no `PartialEq`/`Eq`. `Copy`
/// requires [`NodeRef`] be `Copy`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceNode {
    /// Half-open byte range, in sanitized-source coordinates, this node was
    /// classified from. Entries are sorted by `start`. (`Span` reused.)
    pub source_span: Span,
    pub(crate) normalized_offset: NormalizedOffset,
    /// The classified node landed at `source_span`, tagged with where it sits
    /// in the normalized stream.
    pub node: NodeRef,
}

/// The lexer's complete owned, no-lifetime output.
///
/// Every field is owned (`String`, [`Registry`], `Vec<_>`), with a
/// shared [`NodeStore`] that backs the `StrId`/range payloads referenced by the
/// owned nodes. `Send + Sync` (see static assertion below). Not `Copy`.
/// Derives `Debug` only.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct LexOutput {
    /// Normalized text with PUA sentinels.
    pub(crate) normalized: String,
    /// Verbatim post-sanitize source text (no sentinels, no padding) — the
    /// coordinate space every `source_span` indexes.
    pub(crate) sanitized: SanitizedText,
    pub(crate) source_unchanged: bool,
    /// Sentinel-position → node lookup table.
    pub(crate) registry: Registry,
    /// Non-fatal observations from every stage.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Resolved (open, close) delimiter pairs in sanitized-source coordinates,
    /// close order.
    pub(crate) pairs: Vec<PairLink>,
    /// Source-keyed node side-table, sorted by `source_span.start`.
    pub(crate) source_nodes: Vec<SourceNode>,
    /// Resolved container open/close pairs in normalized coordinates.
    pub(crate) container_pairs: Vec<ContainerPair>,
    /// Owned backing store (string interner + flat content/segment `Vec`s) the
    /// owned nodes' `StrId`/range payloads resolve against.
    pub(crate) store: Arc<NodeStore>,
}

#[derive(Debug)]
pub(crate) struct RegionOutput {
    pub(crate) normalized: String,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) pairs: Vec<PairLink>,
    pub(crate) source_nodes: Vec<SourceNode>,
    pub(crate) container_pairs: Vec<ContainerPair>,
    pub(crate) store: NodeStore,
}

impl LexOutput {
    /// Assemble an [`LexOutput`] from its already-owned field set.
    ///
    /// The only constructor for this `#[non_exhaustive]` struct reachable from
    /// outside the syntax layer — the pipeline's native owned producer
    /// (`crate::pipeline::lex` / `Pipeline::build`) builds the
    /// [`Registry`], [`SourceNode`] table, and [`NodeStore`] (the
    /// classify stage allocates owned nodes directly into the store via
    /// `Allocator`), then hands the whole field set here. Every argument
    /// maps to the identically-named field.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "constructs the non_exhaustive LexOutput from its complete already-owned field set; a parameter object would only restate the field set"
    )]
    pub(crate) fn new(
        normalized: String,
        sanitized: impl Into<SanitizedText>,
        source_unchanged: bool,
        registry: Registry,
        mut diagnostics: Vec<Diagnostic>,
        pairs: Vec<PairLink>,
        source_nodes: Vec<SourceNode>,
        container_pairs: Vec<ContainerPair>,
        store: impl Into<Arc<NodeStore>>,
    ) -> Self {
        diagnostics.sort_unstable_by(|left, right| {
            let left_span = left.span();
            let right_span = right.span();
            left_span
                .start
                .cmp(&right_span.start)
                .then_with(|| left_span.end.cmp(&right_span.end))
                .then_with(|| left.code().cmp(right.code()))
        });
        Self {
            normalized,
            sanitized: sanitized.into(),
            source_unchanged,
            registry,
            diagnostics,
            pairs,
            source_nodes,
            container_pairs,
            store: store.into(),
        }
    }

    /// Find the [`SourceNode`] whose `source_span` covers `src_off`
    /// (a sanitized-source byte offset). O(log n) binary search.
    #[must_use]
    pub(crate) fn node_at_source(&self, src_off: SourceOffset) -> Option<&SourceNode> {
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

#[cfg(test)]
mod tests {
    use super::super::payload::Node;
    use super::*;

    /// Build a `LexOutput` whose only populated field is the source-node
    /// side-table, for exercising `node_at_source`.
    fn output_with(source_nodes: Vec<SourceNode>) -> LexOutput {
        LexOutput::new(
            String::new(),
            String::new(),
            true,
            Registry::empty(),
            Vec::new(),
            Vec::new(),
            source_nodes,
            Vec::new(),
            NodeStore::new(),
        )
    }

    #[test]
    fn node_at_source_covers_ranges_and_gaps() {
        // Two half-open spans with a gap between them: [2,5) → PageBreak and
        // [10,20) → BodyEnd. The table below pins every branch of the binary
        // search — the `idx == 0` (below all starts) guard, the `<= start`
        // partition point, the `idx - 1` predecessor pick, and the half-open
        // `raw < end` containment test (end-exclusive).
        let sn = vec![
            SourceNode {
                source_span: Span::new(2, 5),
                normalized_offset: NormalizedOffset::new(0),
                node: NodeRef::Inline(Node::PageBreak),
            },
            SourceNode {
                source_span: Span::new(10, 20),
                normalized_offset: NormalizedOffset::new(3),
                node: NodeRef::Inline(Node::BodyEnd),
            },
        ];
        let out = output_with(sn);

        let page = Some(NodeRef::Inline(Node::PageBreak));
        let body = Some(NodeRef::Inline(Node::BodyEnd));
        let cases: &[(u32, Option<NodeRef>)] = &[
            (0, None),  // below every span start → idx 0
            (1, None),  // still below the first start (2)
            (2, page),  // first byte of span 0
            (3, page),  // interior
            (4, page),  // last byte inside [2,5)
            (5, None),  // == end is outside (half-open)
            (7, None),  // in the gap between spans
            (10, body), // first byte of span 1
            (15, body), // interior
            (19, body), // last byte inside [10,20)
            (20, None), // == end is outside
            (25, None), // past every span
        ];
        for &(raw, expected) in cases {
            let got = out.node_at_source(SourceOffset::new(raw)).map(|s| s.node);
            assert_eq!(got, expected, "offset {raw} should map to {expected:?}");
        }
    }

    #[test]
    fn node_at_source_on_empty_table_is_none() {
        let out = output_with(Vec::new());
        assert!(out.node_at_source(SourceOffset::new(0)).is_none());
        assert!(out.node_at_source(SourceOffset::new(42)).is_none());
    }
}
