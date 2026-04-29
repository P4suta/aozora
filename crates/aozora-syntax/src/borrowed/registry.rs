//! Sentinel-position → [`AozoraNode`] lookup tables, in `SoA` layout.
//!
//! The registry pairs every PUA sentinel position written into the
//! lexer's normalized text with the [`AozoraNode`] (or
//! [`crate::extension::ContainerKind`]) that originated it.
//! Downstream renderers walk the normalized text, encounter a
//! sentinel, and `get(pos)` to recover the structured node.
//!
//! ## Layout decision
//!
//! Stored as **`SoA`** (struct-of-arrays) with keys laid out via
//! [`aozora_veb::EytzingerMap`] for cache-friendly binary search at
//! sizes ≥ L1 (~16k entries). The Eytzinger key array dwarfs the L1
//! footprint of `std::Vec::binary_search` at all sizes ≥ a few
//! thousand entries; payload arrays are accessed only on dispatch,
//! so they don't need the same layout optimisation.
//!
//! Entries are inserted in monotonically increasing position order
//! during the lex pipeline, so construction can short-circuit the
//! sort step that a general-purpose builder would need.
//!
//! ## Coexistence
//!
//! This is the borrowed-AST registry. The legacy
//! [`crate::PlaceholderRegistry`] is the owned-AST equivalent.

use crate::extension::ContainerKind;

use aozora_veb::EytzingerMap;

use super::types::AozoraNode;

/// Inline [`AozoraNode`] lookup keyed by normalized byte position.
pub type InlineRegistry<'src> = EytzingerMap<u32, AozoraNode<'src>>;

/// Block-leaf [`AozoraNode`] lookup keyed by normalized byte position.
pub type BlockRegistry<'src> = EytzingerMap<u32, AozoraNode<'src>>;

/// Container-kind lookup keyed by normalized byte position. Used by
/// paired-container open / close sentinel positions; the value is the
/// [`ContainerKind`] enum, not a node.
pub type ContainerRegistry = EytzingerMap<u32, ContainerKind>;

/// Unified view over a registry hit, returned by [`Registry::node_at`].
///
/// Hides the four-table structure behind a single enum so editor
/// surfaces (LSP `textDocument/inlayHint`, `hover`, …) can query a
/// single position-keyed entry point without caring which sentinel
/// kind it landed on.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum NodeRef<'src> {
    /// Hit in the inline-sentinel table.
    Inline(AozoraNode<'src>),
    /// Hit in the block-leaf-sentinel table.
    BlockLeaf(AozoraNode<'src>),
    /// Hit in the block-container-open table.
    BlockOpen(ContainerKind),
    /// Hit in the block-container-close table.
    BlockClose(ContainerKind),
}

/// Whole-document registry — four `SoA` tables, one per sentinel kind.
///
/// Mirrors the legacy [`crate::PlaceholderRegistry`]'s shape but
/// substitutes `Vec<(K, V)>` with [`EytzingerMap<K, V>`] for
/// cache-friendly lookup. The `inline` / `block_leaf` tables hold
/// [`AozoraNode`] payloads borrowed from the arena; the `block_open` /
/// `block_close` tables hold container kinds (no arena allocation).
#[derive(Debug, Clone)]
pub struct Registry<'src> {
    pub inline: InlineRegistry<'src>,
    pub block_leaf: BlockRegistry<'src>,
    pub block_open: ContainerRegistry,
    pub block_close: ContainerRegistry,
}

impl<'src> Registry<'src> {
    /// Empty registry — every table is empty. Useful as a starting
    /// point for incremental construction (the lex driver pushes into
    /// builder vecs that later collapse into Eytzinger tables).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            inline: EytzingerMap::new(),
            block_leaf: EytzingerMap::new(),
            block_open: EytzingerMap::new(),
            block_close: EytzingerMap::new(),
        }
    }

    /// True iff every table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inline.is_empty()
            && self.block_leaf.is_empty()
            && self.block_open.is_empty()
            && self.block_close.is_empty()
    }

    /// Total number of entries across all four tables. O(1).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inline.len() + self.block_leaf.len() + self.block_open.len() + self.block_close.len()
    }

    /// Look up the registry entry at the given *normalized-text* byte
    /// position, querying the four sub-tables in order: inline →
    /// `block_leaf` → `block_open` → `block_close`. Returns `None` if
    /// no table holds that position.
    ///
    /// The four tables address disjoint positions by construction (a
    /// single PUA byte position carries exactly one sentinel kind), so
    /// the order matters only for the empty-table fast paths.
    ///
    /// Coordinates here are **normalized**, not source: editor surfaces
    /// that hold a source byte offset must first translate via
    /// `BorrowedLexOutput::node_at_source` (which walks a source-keyed
    /// side-table built during the lex pipeline).
    #[must_use]
    pub fn node_at(&self, pos: u32) -> Option<NodeRef<'src>> {
        if let Some(node) = self.inline.get(&pos).copied() {
            return Some(NodeRef::Inline(node));
        }
        if let Some(node) = self.block_leaf.get(&pos).copied() {
            return Some(NodeRef::BlockLeaf(node));
        }
        if let Some(kind) = self.block_open.get(&pos).copied() {
            return Some(NodeRef::BlockOpen(kind));
        }
        if let Some(kind) = self.block_close.get(&pos).copied() {
            return Some(NodeRef::BlockClose(kind));
        }
        None
    }
}

impl Default for Registry<'_> {
    fn default() -> Self {
        Self::empty()
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
    fn inline_registry_lookup_returns_node() {
        let inline = EytzingerMap::from_sorted_slice(&[
            (10u32, AozoraNode::Indent(Indent { amount: 1 })),
            (20u32, AozoraNode::PageBreak),
            (30u32, AozoraNode::Indent(Indent { amount: 3 })),
        ]);
        let r: Registry<'static> = Registry {
            inline,
            block_leaf: EytzingerMap::new(),
            block_open: EytzingerMap::new(),
            block_close: EytzingerMap::new(),
        };
        assert!(!r.is_empty());
        assert_eq!(r.len(), 3);
        let got = r.inline.get(&20u32).copied();
        assert!(matches!(got, Some(AozoraNode::PageBreak)));
        assert!(r.inline.get(&15).is_none());
    }

    #[test]
    fn node_at_dispatches_to_correct_table() {
        let inline = EytzingerMap::from_sorted_slice(&[(10u32, AozoraNode::PageBreak)]);
        let block_leaf = EytzingerMap::from_sorted_slice(&[(20u32, AozoraNode::PageBreak)]);
        let block_open = EytzingerMap::from_sorted_slice(&[(30u32, ContainerKind::Keigakomi)]);
        let block_close = EytzingerMap::from_sorted_slice(&[(40u32, ContainerKind::Keigakomi)]);
        let r: Registry<'static> = Registry {
            inline,
            block_leaf,
            block_open,
            block_close,
        };
        assert!(matches!(
            r.node_at(10),
            Some(NodeRef::Inline(AozoraNode::PageBreak))
        ));
        assert!(matches!(
            r.node_at(20),
            Some(NodeRef::BlockLeaf(AozoraNode::PageBreak))
        ));
        assert!(matches!(
            r.node_at(30),
            Some(NodeRef::BlockOpen(ContainerKind::Keigakomi))
        ));
        assert!(matches!(
            r.node_at(40),
            Some(NodeRef::BlockClose(ContainerKind::Keigakomi))
        ));
        assert!(r.node_at(99).is_none());
    }

    #[test]
    fn container_registry_carries_kind() {
        let block_open = EytzingerMap::from_sorted_slice(&[
            (5u32, ContainerKind::Indent { amount: 2 }),
            (10u32, ContainerKind::Keigakomi),
        ]);
        let r = Registry::<'static> {
            inline: EytzingerMap::new(),
            block_leaf: EytzingerMap::new(),
            block_open,
            block_close: EytzingerMap::new(),
        };
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.block_open.get(&5).copied(),
            Some(ContainerKind::Indent { amount: 2 })
        );
        assert_eq!(
            r.block_open.get(&10).copied(),
            Some(ContainerKind::Keigakomi)
        );
    }
}
