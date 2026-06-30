//! Whole-document sentinel registry — single Eytzinger-keyed table.
//!
//! [`NodeRefOwned`] is the unified registry-hit view: inline payloads carry an
//! owned [`NodeOwned`]; container discriminants carry `RegionFormat` /
//! `RegionClose`. [`RegistryOwned`] wraps an [`EytzingerMap`] keyed by
//! normalized byte position; `node_at` is one binary search.

use aozora_spec::{NormalizedOffset, Sentinel};
use aozora_veb::EytzingerMap;

use crate::format::{RegionClose, RegionFormat};

use super::payload::NodeOwned;

/// Unified view over a registry hit.
///
/// Each variant tags the sentinel kind that fired; consumers pattern-match the
/// variant once, then handle the inline payload (an owned [`NodeOwned`]) or the
/// container payload (a `Copy` [`RegionFormat`] / [`RegionClose`]
/// discriminant) accordingly.
///
/// `Copy` because every inlined payload is `Copy` ([`NodeOwned`] flattens its
/// `&str`/list payloads to `StrId`/ranges). No `Eq`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum NodeRefOwned {
    /// Hit on an inline-sentinel position ([`Sentinel::Inline`]).
    Inline(NodeOwned),
    /// Hit on a block-leaf-sentinel position ([`Sentinel::BlockLeaf`]).
    BlockLeaf(NodeOwned),
    /// Hit on a block-container-open position ([`Sentinel::BlockOpen`]).
    /// Carries the authoritative open [`RegionFormat`].
    BlockOpen(RegionFormat),
    /// Hit on a block-container-close position ([`Sentinel::BlockClose`]).
    /// Carries the [`RegionClose`] discriminant.
    BlockClose(RegionClose),
}

impl NodeRefOwned {
    /// Sentinel kind that produced this entry.
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
    #[must_use]
    pub const fn kind(self) -> crate::NodeKind {
        match self {
            Self::Inline(node) | Self::BlockLeaf(node) => node.kind(),
            Self::BlockOpen(_) => crate::NodeKind::ContainerOpen,
            Self::BlockClose(_) => crate::NodeKind::ContainerClose,
        }
    }
}

/// Whole-document owned registry — single Eytzinger-keyed table.
///
/// `node_at` is one binary search; every entry's sentinel kind is encoded by
/// the [`NodeRefOwned`] variant. Not `Copy` (the map owns a `Vec`).
#[derive(Debug, Clone)]
pub struct RegistryOwned {
    /// Single `SoA` lookup table keyed by normalized byte position. Entries
    /// arrive in strictly increasing position order.
    table: EytzingerMap<u32, NodeRefOwned>,
}

impl RegistryOwned {
    /// Construct from a position-sorted slice of `(position, NodeRefOwned)`.
    ///
    /// # Panics
    ///
    /// Inherits [`EytzingerMap::from_sorted_slice`]'s debug-only sorted-key
    /// precondition.
    #[must_use]
    pub fn from_sorted_slice(entries: &[(u32, NodeRefOwned)]) -> Self {
        Self {
            table: EytzingerMap::from_sorted_slice(entries),
        }
    }

    /// Empty registry.
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

    /// Look up the entry at the given normalized-text byte position.
    #[must_use]
    pub fn node_at(&self, pos: NormalizedOffset) -> Option<NodeRefOwned> {
        self.table.get(&pos.get()).copied()
    }

    /// Iterate `(position, NodeRefOwned)` in ascending position order.
    pub fn iter_sorted(&self) -> impl Iterator<Item = (u32, NodeRefOwned)> + '_ {
        self.table.iter_sorted().map(|(&p, &nr)| (p, nr))
    }

    /// Iterate entries whose [`NodeRefOwned::sentinel_kind`] matches `kind`.
    pub fn iter_kind(&self, kind: Sentinel) -> impl Iterator<Item = (u32, NodeRefOwned)> + '_ {
        self.iter_sorted()
            .filter(move |(_, nr)| nr.sentinel_kind() == kind)
    }

    /// Count entries whose sentinel kind matches `kind`. O(n).
    #[must_use]
    pub fn count_kind(&self, kind: Sentinel) -> usize {
        self.iter_kind(kind).count()
    }
}

impl Default for RegistryOwned {
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
    /// Construct a pair. Helper for builder tests; in production the pipeline
    /// emits these directly.
    #[must_use]
    pub const fn new(kind: RegionFormat, open: NormalizedOffset, close: NormalizedOffset) -> Self {
        Self { kind, open, close }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_reports_empty() {
        let r = RegistryOwned::empty();
        assert!(r.is_empty(), "empty registry is empty");
        assert_eq!(r.len(), 0, "empty registry has zero entries");
    }

    #[test]
    fn node_at_dispatches_to_variant() {
        let r = RegistryOwned::from_sorted_slice(&[
            (10u32, NodeRefOwned::Inline(NodeOwned::PageBreak)),
            (20u32, NodeRefOwned::BlockLeaf(NodeOwned::PageBreak)),
            (30u32, NodeRefOwned::BlockOpen(RegionFormat::Framed)),
            (40u32, NodeRefOwned::BlockClose(RegionClose::Framed)),
        ]);
        assert!(matches!(
            r.node_at(NormalizedOffset::new(30)),
            Some(NodeRefOwned::BlockOpen(RegionFormat::Framed))
        ));
        assert_eq!(r.count_kind(Sentinel::Inline), 1, "one inline entry");
        assert_eq!(r.count_kind(Sentinel::BlockOpen), 1, "one open entry");
        assert!(
            r.node_at(NormalizedOffset::new(99)).is_none(),
            "miss returns None"
        );
    }
}
