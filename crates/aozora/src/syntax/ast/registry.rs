//! Whole-document sentinel registry — single Eytzinger-keyed table.
//!
//! [`NodeRef`] is the unified registry-hit view: inline payloads carry an
//! owned [`Node`]; container discriminants carry `RegionFormat` /
//! `RegionClose`. [`Registry`] wraps an [`EytzingerMap`] keyed by
//! normalized byte position; `node_at` is one binary search.

use crate::collections::EytzingerMap;
use crate::spec::NormalizedOffset;
#[cfg(test)]
use crate::spec::Sentinel;

use crate::syntax::NodeKind;
use crate::syntax::format::{RegionClose, RegionFormat};

use super::output::SourceNode;
use super::payload::Node;

/// Unified view over a registry hit.
///
/// Each variant tags the sentinel kind that fired; consumers pattern-match the
/// variant once, then handle the inline payload (an owned [`Node`]) or the
/// container payload (a `Copy` [`RegionFormat`] / [`RegionClose`]
/// discriminant) accordingly.
///
/// `Copy` because every inlined payload is `Copy` ([`Node`] flattens its
/// `&str`/list payloads to `StrId`/ranges). No `Eq`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub(crate) enum NodeRef {
    /// Hit on an inline-sentinel position.
    Inline(Node),
    /// Hit on a block-leaf-sentinel position.
    BlockLeaf(Node),
    /// Hit on a block-container-open position.
    /// Carries the authoritative open [`RegionFormat`].
    BlockOpen(RegionFormat),
    /// Hit on a block-container-close position.
    /// Carries the [`RegionClose`] discriminant.
    BlockClose(RegionClose),
}

impl NodeRef {
    /// Sentinel kind that produced this entry.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn sentinel_kind(self) -> Sentinel {
        match self {
            Self::Inline(_) => Sentinel::Inline,
            Self::BlockLeaf(_) => Sentinel::BlockLeaf,
            Self::BlockOpen(_) => Sentinel::BlockOpen,
            Self::BlockClose(_) => Sentinel::BlockClose,
        }
    }

    /// Cross-cutting [`crate::syntax::NodeKind`] tag for this entry.
    #[must_use]
    pub(crate) const fn kind(self) -> NodeKind {
        match self {
            Self::Inline(node) | Self::BlockLeaf(node) => node.kind(),
            Self::BlockOpen(_) => NodeKind::ContainerOpen,
            Self::BlockClose(_) => NodeKind::ContainerClose,
        }
    }
}

/// Whole-document owned registry — single Eytzinger-keyed table.
///
/// `node_at` is one binary search; every entry's sentinel kind is encoded by
/// the [`NodeRef`] variant. Not `Copy` (the map owns a `Vec`).
#[derive(Debug, Clone)]
pub(crate) struct Registry {
    /// Single `SoA` lookup table keyed by normalized byte position. Entries
    /// arrive in strictly increasing position order.
    table: EytzingerMap<u32, NodeRef>,
}

impl Registry {
    /// Construct from a position-sorted slice of `(position, NodeRef)`.
    ///
    /// # Panics
    ///
    /// Inherits `EytzingerMap::from_sorted_slice`'s debug-only sorted-key
    /// precondition.
    #[must_use]
    pub(crate) fn from_sorted_slice(entries: &[(u32, NodeRef)]) -> Self {
        Self {
            table: EytzingerMap::from_sorted_slice(entries),
        }
    }

    pub(crate) fn from_source_nodes(entries: &[SourceNode]) -> Self {
        Self {
            table: EytzingerMap::from_sorted_by(entries, |entry| {
                (entry.normalized_offset.get(), entry.node)
            }),
        }
    }

    /// Empty registry.
    #[must_use]
    pub(crate) const fn empty() -> Self {
        Self {
            table: EytzingerMap::new(),
        }
    }

    /// True iff the registry holds no entries.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Total number of entries across all sentinel kinds. O(1).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.table.len()
    }

    /// Look up the entry at the given normalized-text byte position.
    #[must_use]
    pub(crate) fn node_at(&self, pos: NormalizedOffset) -> Option<NodeRef> {
        self.table.get(&pos.get()).copied()
    }

    /// Iterate `(position, NodeRef)` in ascending position order.
    #[cfg(test)]
    pub(crate) fn iter_sorted(&self) -> impl Iterator<Item = (u32, NodeRef)> + '_ {
        self.table.iter_sorted().map(|(&p, &nr)| (p, nr))
    }

    /// Iterate entries whose [`NodeRef::sentinel_kind`] matches `kind`.
    #[cfg(test)]
    pub(crate) fn iter_kind(&self, kind: Sentinel) -> impl Iterator<Item = (u32, NodeRef)> + '_ {
        self.iter_sorted()
            .filter(move |(_, nr)| nr.sentinel_kind() == kind)
    }

    /// Count entries whose sentinel kind matches `kind`. O(n).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn count_kind(&self, kind: Sentinel) -> usize {
        self.iter_kind(kind).count()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::empty()
    }
}

/// Resolved container open/close pair in normalized coordinates.
///
/// Lifetime-free `Copy` side-table entry: the pipeline emits one per balanced
/// `［＃ここから…］` / `［＃ここで…終わり］` pair. Editor surfaces (LSP
/// `linkedEditingRange` / `documentHighlight` against container markers) consume
/// this directly instead of re-deriving the pairing from independent open /
/// close registry entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContainerPair {
    /// The open container format. The builder constructs the pair from the
    /// open-stack pop, so `kind` reflects the open marker authoritatively
    /// (the close side is a discriminant; see [`RegionClose`]).
    pub kind: RegionFormat,
    /// Normalized byte offset of the open sentinel (`U+E003`).
    pub open: NormalizedOffset,
    /// Normalized byte offset of the close sentinel (`U+E004`).
    pub close: NormalizedOffset,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::format::EnclosureKind;

    #[test]
    fn empty_registry_reports_empty() {
        let r = Registry::empty();
        assert!(r.is_empty(), "empty registry is empty");
        assert_eq!(r.len(), 0, "empty registry has zero entries");
    }

    #[test]
    fn non_empty_registry_reports_size_and_per_kind_counts() {
        // Two inline entries + one open: len is the total, and `count_kind`
        // returns the real per-kind tally — not a stubbed 0 / constant 1.
        let r = Registry::from_sorted_slice(&[
            (10u32, NodeRef::Inline(Node::PageBreak)),
            (20u32, NodeRef::Inline(Node::BodyEnd)),
            (
                30u32,
                NodeRef::BlockOpen(RegionFormat::Framed(EnclosureKind::Rule)),
            ),
        ]);
        assert!(!r.is_empty(), "a populated registry is not empty");
        assert_eq!(r.len(), 3, "len is the total entry count");
        assert_eq!(r.count_kind(Sentinel::Inline), 2, "two inline entries");
        assert_eq!(r.count_kind(Sentinel::BlockOpen), 1, "one open entry");
        assert_eq!(
            r.count_kind(Sentinel::BlockClose),
            0,
            "no close entries → zero, not a constant"
        );
    }

    #[test]
    fn node_at_dispatches_to_variant() {
        let r = Registry::from_sorted_slice(&[
            (10u32, NodeRef::Inline(Node::PageBreak)),
            (20u32, NodeRef::BlockLeaf(Node::PageBreak)),
            (
                30u32,
                NodeRef::BlockOpen(RegionFormat::Framed(EnclosureKind::Rule)),
            ),
            (
                40u32,
                NodeRef::BlockClose(RegionClose::Framed(EnclosureKind::Rule)),
            ),
        ]);
        assert!(matches!(
            r.node_at(NormalizedOffset::new(30)),
            Some(NodeRef::BlockOpen(RegionFormat::Framed(
                EnclosureKind::Rule
            )))
        ));
        assert_eq!(r.count_kind(Sentinel::Inline), 1, "one inline entry");
        assert_eq!(r.count_kind(Sentinel::BlockOpen), 1, "one open entry");
        assert!(
            r.node_at(NormalizedOffset::new(99)).is_none(),
            "miss returns None"
        );
    }
}
