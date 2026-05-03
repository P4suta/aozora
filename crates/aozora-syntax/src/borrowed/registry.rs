//! Sentinel-position → [`AozoraNode`] lookup table.
//!
//! The registry pairs every PUA sentinel position written into the
//! lexer's normalized text with the [`AozoraNode`] (or
//! [`crate::extension::ContainerKind`]) that originated it.
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

use crate::extension::ContainerKind;

use aozora_spec::{NormalizedOffset, Sentinel};
use aozora_veb::EytzingerMap;

use super::types::AozoraNode;

/// Unified view over a registry hit, returned by [`Registry::node_at`].
///
/// Each variant tags the sentinel kind that fired; renderers
/// pattern-match the variant once, then handle the inline payload
/// (a borrowed [`AozoraNode`]) or the container payload (a
/// [`ContainerKind`] enum) accordingly.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum NodeRef<'src> {
    /// Hit on an inline-sentinel position
    /// ([`aozora_spec::Sentinel::Inline`]).
    Inline(AozoraNode<'src>),
    /// Hit on a block-leaf-sentinel position
    /// ([`aozora_spec::Sentinel::BlockLeaf`]).
    BlockLeaf(AozoraNode<'src>),
    /// Hit on a block-container-open position
    /// ([`aozora_spec::Sentinel::BlockOpen`]).
    BlockOpen(ContainerKind),
    /// Hit on a block-container-close position
    /// ([`aozora_spec::Sentinel::BlockClose`]).
    BlockClose(ContainerKind),
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
    /// [`AozoraNode::kind`] tag; container open / close hits flatten
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
    /// `BorrowedLexOutput::node_at_source` (which walks a
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
/// `BorrowedLexOutput::normalized` text, the same coordinate space
/// the [`Registry`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerPair {
    /// The container kind. The builder constructs the pair from the
    /// open-stack pop, so `kind` reflects the open marker
    /// authoritatively (rather than the close-side payload).
    pub kind: ContainerKind,
    /// Normalized byte offset of the open sentinel (`U+E003`).
    pub open: NormalizedOffset,
    /// Normalized byte offset of the close sentinel (`U+E004`).
    pub close: NormalizedOffset,
}

impl ContainerPair {
    /// Construct a pair. Helper for builder tests; in production the
    /// pipeline emits these directly.
    #[must_use]
    pub const fn new(kind: ContainerKind, open: NormalizedOffset, close: NormalizedOffset) -> Self {
        Self { kind, open, close }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Indent;

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
                NodeRef::Inline(AozoraNode::Indent(Indent { amount: 1 })),
            ),
            (20u32, NodeRef::Inline(AozoraNode::PageBreak)),
            (
                30u32,
                NodeRef::Inline(AozoraNode::Indent(Indent { amount: 3 })),
            ),
        ]);
        assert!(!r.is_empty());
        assert_eq!(r.len(), 3);
        let got = r.node_at(NormalizedOffset::new(20));
        assert!(matches!(got, Some(NodeRef::Inline(AozoraNode::PageBreak))));
        assert!(r.node_at(NormalizedOffset::new(15)).is_none());
    }

    #[test]
    fn node_at_dispatches_to_correct_variant() {
        let r: Registry<'static> = Registry::from_sorted_slice(&[
            (10u32, NodeRef::Inline(AozoraNode::PageBreak)),
            (20u32, NodeRef::BlockLeaf(AozoraNode::PageBreak)),
            (30u32, NodeRef::BlockOpen(ContainerKind::Keigakomi)),
            (40u32, NodeRef::BlockClose(ContainerKind::Keigakomi)),
        ]);
        assert!(matches!(
            r.node_at(NormalizedOffset::new(10)),
            Some(NodeRef::Inline(AozoraNode::PageBreak))
        ));
        assert!(matches!(
            r.node_at(NormalizedOffset::new(20)),
            Some(NodeRef::BlockLeaf(AozoraNode::PageBreak))
        ));
        assert!(matches!(
            r.node_at(NormalizedOffset::new(30)),
            Some(NodeRef::BlockOpen(ContainerKind::Keigakomi))
        ));
        assert!(matches!(
            r.node_at(NormalizedOffset::new(40)),
            Some(NodeRef::BlockClose(ContainerKind::Keigakomi))
        ));
        assert!(r.node_at(NormalizedOffset::new(99)).is_none());
    }

    #[test]
    fn count_kind_buckets_entries_by_sentinel() {
        let r: Registry<'static> = Registry::from_sorted_slice(&[
            (
                5u32,
                NodeRef::BlockOpen(ContainerKind::Indent { amount: 2 }),
            ),
            (10u32, NodeRef::BlockOpen(ContainerKind::Keigakomi)),
            (15u32, NodeRef::Inline(AozoraNode::PageBreak)),
            (20u32, NodeRef::BlockClose(ContainerKind::Keigakomi)),
        ]);
        assert_eq!(r.count_kind(Sentinel::BlockOpen), 2);
        assert_eq!(r.count_kind(Sentinel::Inline), 1);
        assert_eq!(r.count_kind(Sentinel::BlockClose), 1);
        assert_eq!(r.count_kind(Sentinel::BlockLeaf), 0);
    }

    #[test]
    fn node_ref_sentinel_kind_round_trips() {
        let inline = NodeRef::Inline(AozoraNode::PageBreak);
        let block_leaf = NodeRef::BlockLeaf(AozoraNode::PageBreak);
        let block_open = NodeRef::BlockOpen(ContainerKind::Keigakomi);
        let block_close = NodeRef::BlockClose(ContainerKind::Keigakomi);
        assert_eq!(inline.sentinel_kind(), Sentinel::Inline);
        assert_eq!(block_leaf.sentinel_kind(), Sentinel::BlockLeaf);
        assert_eq!(block_open.sentinel_kind(), Sentinel::BlockOpen);
        assert_eq!(block_close.sentinel_kind(), Sentinel::BlockClose);
    }
}
