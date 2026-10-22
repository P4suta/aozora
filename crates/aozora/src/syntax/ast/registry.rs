use crate::collections::EytzingerMap;
use crate::spec::NormalizedOffset;
#[cfg(test)]
use crate::spec::Sentinel;

use crate::syntax::NodeKind;
use crate::syntax::format::{RegionClose, RegionFormat};

use super::output::SourceNode;
use super::payload::Node;

#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub(crate) enum NodeRef {
    Inline(Node),
    BlockLeaf(Node),
    BlockOpen(RegionFormat),
    BlockClose(RegionClose),
}

impl NodeRef {
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

    #[must_use]
    pub(crate) const fn kind(self) -> NodeKind {
        match self {
            Self::Inline(node) | Self::BlockLeaf(node) => node.kind(),
            Self::BlockOpen(_) => NodeKind::ContainerOpen,
            Self::BlockClose(_) => NodeKind::ContainerClose,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Registry {
    table: EytzingerMap<u32, NodeRef>,
}

impl Registry {
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

    #[must_use]
    pub(crate) const fn empty() -> Self {
        Self {
            table: EytzingerMap::new(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.table.len()
    }

    #[must_use]
    pub(crate) fn node_at(&self, pos: NormalizedOffset) -> Option<NodeRef> {
        self.table.get(&pos.get()).copied()
    }

    #[cfg(test)]
    pub(crate) fn iter_sorted(&self) -> impl Iterator<Item = (u32, NodeRef)> + '_ {
        self.table.iter_sorted().map(|(&p, &nr)| (p, nr))
    }

    #[cfg(test)]
    pub(crate) fn iter_kind(&self, kind: Sentinel) -> impl Iterator<Item = (u32, NodeRef)> + '_ {
        self.iter_sorted()
            .filter(move |(_, nr)| nr.sentinel_kind() == kind)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContainerPair {
    pub kind: RegionFormat,
    pub open: NormalizedOffset,
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
