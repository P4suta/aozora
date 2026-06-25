//! Sentinel-position → [`Node`] lookup table.
//!
//! The registry pairs every PUA sentinel position written into the
//! lexer's normalized text with the [`Node`] (or the
//! [`crate::format::RegionFormat`] / [`crate::format::RegionClose`]
//! container marker) that originated it.
//! Downstream renderers walk the normalized text, encounter a
//! sentinel, and `node_at(pos)` to recover the structured node.
//!
//! # Layout decision
//!
//! Stored as **one** [`aozora_veb::EytzingerMap`] keyed by normalized
//! byte position. Every entry's payload is a [`NodeRef`] enum that
//! discriminates inline / block-leaf / block-open / block-close
//! hits — pre-Phase-D the four sentinel kinds lived in four
//! independent tables and `node_at` did a 4-way linear sweep. The
//! single-table layout means one binary search per lookup; renderers
//! pattern-match on the `NodeRef` variant inline.
//!
//! Entries are inserted in monotonically increasing position order
//! during the lex pipeline (the classifier emits spans in source
//! order, every sentinel position is therefore strictly greater than
//! the previous), so construction can short-circuit the sort step
//! that a general-purpose builder would need.
//!
//! Position-keyed map from `NormalizedOffset` to AST node, backed by
//! [`aozora_veb::EytzingerMap`] for cache-friendly lookups during
//! render-time traversal.

use crate::format::{RegionClose, RegionFormat};

use aozora_spec::{NormalizedOffset, Sentinel};
use aozora_veb::EytzingerMap;

use super::types::Node;

/// Unified view over a registry hit, returned by [`Registry::node_at`].
///
/// Each variant tags the sentinel kind that fired; renderers
/// pattern-match the variant once, then handle the inline payload
/// (a borrowed [`Node`]) or the container payload (a
/// [`crate::format::RegionFormat`] enum) accordingly.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum NodeRef<'src> {
    /// Hit on an inline-sentinel position
    /// ([`aozora_spec::Sentinel::Inline`]).
    Inline(Node<'src>),
    /// Hit on a block-leaf-sentinel position
    /// ([`aozora_spec::Sentinel::BlockLeaf`]).
    BlockLeaf(Node<'src>),
    /// Hit on a block-container-open position
    /// ([`aozora_spec::Sentinel::BlockOpen`]). Carries the authoritative
    /// open [`RegionFormat`].
    BlockOpen(RegionFormat),
    /// Hit on a block-container-close position
    /// ([`aozora_spec::Sentinel::BlockClose`]). Carries the [`RegionClose`]
    /// discriminant; the open payload stays authoritative when pairing.
    BlockClose(RegionClose),
}

impl NodeRef<'_> {
    /// Sentinel kind that produced this entry.
    ///
    /// Useful for tests / tooling that want to bucket registry
    /// entries by sentinel kind without depending on the variant
    /// payload shape.
    #[must_use]
    pub const fn sentinel_kind(self) -> Sentinel {
        match self {
            Self::Inline(_) => Sentinel::Inline,
            Self::BlockLeaf(_) => Sentinel::BlockLeaf,
            Self::BlockOpen(_) => Sentinel::BlockOpen,
            Self::BlockClose(_) => Sentinel::BlockClose,
        }
    }

    /// Cross-cutting [`crate::NodeKind`] tag for this entry.
    ///
    /// Inline / block-leaf hits project to the underlying
    /// [`Node::kind`] tag; container open / close hits flatten
    /// into [`NodeKind::ContainerOpen`](crate::NodeKind::ContainerOpen)
    /// / [`ContainerClose`](crate::NodeKind::ContainerClose) because
    /// the wire format places container kind detail in the inline
    /// span rather than on the open/close marker.
    #[must_use]
    pub const fn kind(self) -> crate::NodeKind {
        match self {
            Self::Inline(node) | Self::BlockLeaf(node) => node.kind(),
            Self::BlockOpen(_) => crate::NodeKind::ContainerOpen,
            Self::BlockClose(_) => crate::NodeKind::ContainerClose,
        }
    }
}

/// Whole-document registry — single Eytzinger-keyed table.
///
/// `node_at` is one binary search, and every entry's sentinel kind is
/// encoded by the [`NodeRef`] variant — renderers pattern-match the
/// hit inline rather than dispatching across per-kind tables.
#[derive(Debug, Clone)]
pub struct Registry<'src> {
    /// Single `SoA` lookup table keyed by normalized byte position.
    /// Built once at pipeline-build time from the classifier's emit
    /// stream; entries arrive in strictly increasing position order
    /// because the classifier tiles spans contiguously.
    table: EytzingerMap<u32, NodeRef<'src>>,
}

impl<'src> Registry<'src> {
    /// Construct a registry from a position-sorted slice of
    /// `(position, NodeRef)` entries.
    ///
    /// # Panics
    ///
    /// Inherits [`EytzingerMap::from_sorted_slice`]'s precondition:
    /// the slice must be sorted by key. The lex pipeline always emits
    /// in source order, so this is satisfied by construction.
    #[must_use]
    pub fn from_sorted_slice(entries: &[(u32, NodeRef<'src>)]) -> Self {
        Self {
            table: EytzingerMap::from_sorted_slice(entries),
        }
    }

    /// Empty registry. Useful as a starting point for incremental
    /// construction (the lex driver pushes into a builder vec that
    /// later collapses into the Eytzinger table).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            table: EytzingerMap::new(),
        }
    }

    /// True iff the registry holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Total number of entries across all sentinel kinds. O(1).
    #[must_use]
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Look up the registry entry at the given *normalized-text* byte
    /// position. Returns `None` if no sentinel landed at that
    /// position.
    ///
    /// The argument is a [`NormalizedOffset`] newtype rather than a
    /// raw `u32` — editor surfaces that hold a source-coordinate byte
    /// offset must first translate via
    /// `LexOutput::node_at_source` (which walks a
    /// source-keyed side-table built during the lex pipeline) instead
    /// of casting between the two coordinate spaces.
    #[must_use]
    pub fn node_at(&self, pos: NormalizedOffset) -> Option<NodeRef<'src>> {
        self.table.get(&pos.get()).copied()
    }

    /// Iterate over `(position, NodeRef)` entries in ascending
    /// position order. Useful for tests and tooling that want to
    /// enumerate everything the registry holds.
    pub fn iter_sorted(&self) -> impl Iterator<Item = (u32, NodeRef<'src>)> + '_ {
        self.table.iter_sorted().map(|(&p, &nr)| (p, nr))
    }

    /// Iterate over entries whose [`NodeRef::sentinel_kind`] matches
    /// `kind`. O(n) but the filter is a constant-time variant
    /// discriminant compare.
    pub fn iter_kind(&self, kind: Sentinel) -> impl Iterator<Item = (u32, NodeRef<'src>)> + '_ {
        self.iter_sorted()
            .filter(move |(_, nr)| nr.sentinel_kind() == kind)
    }

    /// Count entries whose [`NodeRef::sentinel_kind`] matches `kind`.
    ///
    /// O(n) over the table. Cardinality assertions in unit tests
    /// drive this; production lookups go through [`Self::node_at`].
    #[must_use]
    pub fn count_kind(&self, kind: Sentinel) -> usize {
        self.iter_kind(kind).count()
    }
}

impl Default for Registry<'_> {
    fn default() -> Self {
        Self::empty()
    }
}

/// Resolved (open, close) container-marker pair, in normalized
/// coordinates.
///
/// The pipeline tracks an open-stack while it walks the classifier
/// output; `ContainerPair` surfaces that pairing explicitly so editor
/// surfaces and renderers asking "where is the close marker for this
/// open?" can index this slice directly instead of re-running the
/// matching logic over the registry's
/// [`NodeRef::BlockOpen`] / [`NodeRef::BlockClose`] entries.
///
/// Coordinates are [`NormalizedOffset`] — they index the
/// `LexOutput::normalized` text, the same coordinate space
/// the [`Registry`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerPair {
    /// The open container format. The builder constructs the pair from the
    /// open-stack pop, so `kind` reflects the open marker authoritatively
    /// (the close side is a discriminant; see [`RegionClose`]).
    pub kind: RegionFormat,
    /// Normalized byte offset of the open sentinel (`U+E003`).
    pub open: NormalizedOffset,
    /// Normalized byte offset of the close sentinel (`U+E004`).
    pub close: NormalizedOffset,
}

impl ContainerPair {
    /// Construct a pair. Helper for builder tests; in production the
    /// pipeline emits these directly.
    #[must_use]
    pub const fn new(kind: RegionFormat, open: NormalizedOffset, close: NormalizedOffset) -> Self {
        Self { kind, open, close }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{BlockStyles, IndentBlock, IndentLayout, LineFormat};

    #[test]
    fn empty_registry_reports_empty() {
        let r: Registry<'static> = Registry::empty();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn default_registry_is_empty() {
        let r: Registry<'static> = Registry::default();
        assert!(r.is_empty());
    }

    #[test]
    fn node_at_returns_inline_payload_for_inline_sentinel_position() {
        let r: Registry<'static> = Registry::from_sorted_slice(&[
            (
                10u32,
                NodeRef::Inline(Node::Line(LineFormat::Indent { amount: 1 })),
            ),
            (20u32, NodeRef::Inline(Node::PageBreak)),
            (
                30u32,
                NodeRef::Inline(Node::Line(LineFormat::Indent { amount: 3 })),
            ),
        ]);
        assert!(!r.is_empty());
        assert_eq!(r.len(), 3);
        let got = r.node_at(NormalizedOffset::new(20));
        assert!(matches!(got, Some(NodeRef::Inline(Node::PageBreak))));
        assert!(r.node_at(NormalizedOffset::new(15)).is_none());
    }

    #[test]
    fn node_at_dispatches_to_correct_variant() {
        let r: Registry<'static> = Registry::from_sorted_slice(&[
            (10u32, NodeRef::Inline(Node::PageBreak)),
            (20u32, NodeRef::BlockLeaf(Node::PageBreak)),
            (30u32, NodeRef::BlockOpen(RegionFormat::Framed)),
            (40u32, NodeRef::BlockClose(RegionClose::Framed)),
        ]);
        assert!(matches!(
            r.node_at(NormalizedOffset::new(10)),
            Some(NodeRef::Inline(Node::PageBreak))
        ));
        assert!(matches!(
            r.node_at(NormalizedOffset::new(20)),
            Some(NodeRef::BlockLeaf(Node::PageBreak))
        ));
        assert!(matches!(
            r.node_at(NormalizedOffset::new(30)),
            Some(NodeRef::BlockOpen(RegionFormat::Framed))
        ));
        assert!(matches!(
            r.node_at(NormalizedOffset::new(40)),
            Some(NodeRef::BlockClose(RegionClose::Framed))
        ));
        assert!(r.node_at(NormalizedOffset::new(99)).is_none());
    }

    #[test]
    fn count_kind_buckets_entries_by_sentinel() {
        let r: Registry<'static> = Registry::from_sorted_slice(&[
            (
                5u32,
                NodeRef::BlockOpen(RegionFormat::Indent(IndentBlock {
                    amount: 2,
                    wrap: None,
                    center: false,
                    layout: IndentLayout::None,
                    styles: BlockStyles::EMPTY,
                })),
            ),
            (10u32, NodeRef::BlockOpen(RegionFormat::Framed)),
            (15u32, NodeRef::Inline(Node::PageBreak)),
            (20u32, NodeRef::BlockClose(RegionClose::Framed)),
        ]);
        assert_eq!(r.count_kind(Sentinel::BlockOpen), 2);
        assert_eq!(r.count_kind(Sentinel::Inline), 1);
        assert_eq!(r.count_kind(Sentinel::BlockClose), 1);
        assert_eq!(r.count_kind(Sentinel::BlockLeaf), 0);
    }

    #[test]
    fn node_ref_sentinel_kind_round_trips() {
        let inline = NodeRef::Inline(Node::PageBreak);
        let block_leaf = NodeRef::BlockLeaf(Node::PageBreak);
        let block_open = NodeRef::BlockOpen(RegionFormat::Framed);
        let block_close = NodeRef::BlockClose(RegionClose::Framed);
        assert_eq!(inline.sentinel_kind(), Sentinel::Inline);
        assert_eq!(block_leaf.sentinel_kind(), Sentinel::BlockLeaf);
        assert_eq!(block_open.sentinel_kind(), Sentinel::BlockOpen);
        assert_eq!(block_close.sentinel_kind(), Sentinel::BlockClose);
    }
}
