//! Source-region ownership and minimal-diff source splicing.
//!
//! This is the first slice of the **minimal-diff edit splice** (issue
//! #202, the last deferred pillar of the coremodel-purification epic
//! #189). It builds on the now-complete provenance model — every
//! [`Tree::source_nodes`] entry already
//! tiles the sanitized source contiguously, and each forward leaf
//! carries an irreducible [`ForwardOrigin`] — to answer two questions a
//! source-faithful editor surface needs:
//!
//! 1. **Who owns each source byte?** [`Tree::owned_regions`] projects
//!    the source-node table into a *total, non-overlapping, ordered*
//!    tiling of the sanitized source: one [`OwnedRegion`] per classified
//!    node plus the interstitial plain runs between them. Concatenating
//!    every region's bytes reproduces [`Tree::to_source_verbatim`]
//!    exactly.
//!
//! 2. **Can this region be edited by replacing its bytes alone?** Each
//!    region carries a [`SpliceSafety`]. A [`Safe`](SpliceSafety::Safe)
//!    region fully owns its rendered content, so replacing its bytes is
//!    a complete, coherent edit that cannot desync its neighbours.
//!    [`Tree::splice_source`] performs that replacement and returns the
//!    minimal-diff source — every other byte stays identical, unlike the
//!    whole-document reflow of [`Tree::to_source`].
//!
//! # What is *not* here yet
//!
//! Regions whose ownership is **split** (a forward reference whose
//! displayed literal lives in a separate upstream run —
//! [`ForwardOrigin::Referenced`], a non-promoted heading hint, a margin
//! note) or **paired** (a container open/close marker, paired in
//! normalized coordinates) are reported [`Deferred`](SpliceSafety::Deferred):
//! a coherent edit needs multi-region coordination that lands in a later
//! phase. [`Tree::splice_source`] refuses them with [`SpliceError`]
//! rather than emit a byte-valid but semantically incomplete edit.
//!
//! Incremental *re-parse* (reusing the unaffected tree across an edit) is
//! a separate, larger effort and is not attempted here; this slice is
//! purely additive and leaves every existing output byte-for-byte
//! unchanged.

use core::error::Error;
use core::fmt;

use aozora_spec::{SourceOffset, Span};
use aozora_syntax::borrowed::ForwardOrigin;

use crate::{Node, NodeRef, Tree};

/// What a single source region represents.
///
/// Derived purely from the region's classified [`NodeRef`] (and, for a
/// forward leaf, its [`ForwardOrigin`]).
/// The role is informational — tooling renders it to explain *why* a
/// region is or is not safe to splice — while the actionable bit is the
/// region's [`SpliceSafety`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionRole {
    /// Plain text between classified constructs. Not a node; carried so
    /// the tiling is complete.
    Interstitial,
    /// Ruby (furigana). Self-contained: the base run is included in the
    /// region (the explicit `｜` or the implicit trailing-kanji pull-back).
    Ruby,
    /// Forward emphasis whose target literal the classifier pulled into
    /// the node from the immediately-preceding source
    /// ([`ForwardOrigin::Reclaimed`]).
    /// The literal lives inside the region, so it is self-contained.
    ForwardReclaimed,
    /// Forward emphasis whose target literal stays in a separate upstream
    /// run ([`ForwardOrigin::Referenced`]).
    /// Ownership is split across two regions.
    ForwardReferenced,
    /// Out-of-character-range glyph (外字).
    Gaiji,
    /// Single-line layout directive (字下げ / 地付き / 中央 / 罫囲み).
    Line,
    /// Warichu (割り注, split annotation) — the inline form owns its body.
    Warichu,
    /// Page break (`［＃改ページ］`).
    PageBreak,
    /// Section break (`［＃改丁／改段／改見開き］`).
    SectionBreak,
    /// Heading promoted from a bare line above its directive — the
    /// referent line is reclaimed into the region, so it is self-contained.
    Heading,
    /// Forward heading hint whose referent is *not* the bare line above
    /// it, so the referent lives elsewhere. Ownership is split.
    HeadingHint,
    /// Illustration (`［＃挿絵］`).
    Illustration,
    /// Chinese-reading-order mark (返り点).
    Kaeriten,
    /// Generic annotation (`［＃ママ］`, an unresolved `［＃…］`, …). The
    /// directive bracket is self-contained.
    Directive,
    /// `≪…≫` double-angle quotation.
    AngleQuote,
    /// Left-side note (注記 / 傍記) attached to a preceding base run.
    /// Ownership is split like a forward reference.
    MarginNote,
    /// A leaf container node (rare; containers usually surface as
    /// [`RegionRole::ContainerOpen`] / [`RegionRole::ContainerClose`]).
    Container,
    /// A paired-container open marker (`［＃ここから…］`). Its partner
    /// pairs in normalized coordinates.
    ContainerOpen,
    /// A paired-container close marker (`［＃ここで…終わり］`).
    ContainerClose,
    /// A future [`Node`] variant not yet
    /// classified by this projection.
    Other,
}

/// Why a region cannot be spliced by replacing its bytes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeferredReason {
    /// Plain interstitial text — not a node. Edit the bytes directly via
    /// [`Tree::to_source_verbatim`];
    /// the splice API maps *nodes* to regions, so it declines plain runs.
    Interstitial,
    /// The region's displayed literal lives in a separate upstream run
    /// (a [`ForwardOrigin::Referenced`]
    /// forward, a [`RegionRole::HeadingHint`], a [`RegionRole::MarginNote`]).
    /// A coherent edit must change both regions together — the coupled
    /// splice lands in a later phase (#202).
    SplitOwnership,
    /// The region is a container open/close marker whose partner pairs in
    /// normalized coordinates; container splice lands in a later phase
    /// (#202).
    ContainerPairing,
    /// A future node variant this projection does not yet understand.
    Unclassified,
}

/// Whether replacing a region's bytes is a complete, coherent edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpliceSafety {
    /// The region fully owns its rendered content. Replacing its bytes is
    /// a complete edit; every neighbouring region's bytes stay identical
    /// (a consequence of the contiguous tiling), and the result re-parses
    /// to the intended structure for any well-formed replacement.
    Safe,
    /// The region's ownership is split or paired, so a single-region byte
    /// replacement would be incomplete. See [`DeferredReason`].
    Deferred(DeferredReason),
}

/// A contiguous run of source bytes and what it owns.
///
/// Yielded by [`Tree::owned_regions`] / [`Tree::owned_region_at`]. The
/// [`span`](Self::span) indexes the **sanitized** source — the same
/// coordinate space as [`Tree::to_source_verbatim`]
/// and every `source_span` on [`Tree::source_nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedRegion {
    /// Half-open byte range in sanitized-source coordinates.
    pub span: Span,
    /// What the region represents.
    pub role: RegionRole,
    /// Whether the region can be spliced by replacing its bytes alone.
    pub safety: SpliceSafety,
}

/// Error returned by [`Tree::splice_source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpliceError {
    /// The region is not a single-region-safe edit point; its
    /// [`SpliceSafety`] was [`Deferred`](SpliceSafety::Deferred).
    Deferred {
        /// The region's role, for diagnostics.
        role: RegionRole,
        /// Why the region was declined.
        reason: DeferredReason,
    },
}

impl fmt::Display for SpliceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deferred { role, reason } => write!(
                f,
                "region {role:?} cannot be spliced in isolation ({reason:?})"
            ),
        }
    }
}

impl Error for SpliceError {}

/// Classify a node region's role and splice safety. Pure: the result is
/// a function of the [`NodeRef`] variant and (for a forward leaf) its
/// [`ForwardOrigin`] alone.
fn classify_node_ref(node: NodeRef<'_>) -> (RegionRole, SpliceSafety) {
    use SpliceSafety::{Deferred, Safe};

    match node {
        NodeRef::BlockOpen(_) => (
            RegionRole::ContainerOpen,
            Deferred(DeferredReason::ContainerPairing),
        ),
        NodeRef::BlockClose(_) => (
            RegionRole::ContainerClose,
            Deferred(DeferredReason::ContainerPairing),
        ),
        NodeRef::Inline(n) | NodeRef::BlockLeaf(n) => match n {
            Node::Format(f) => match f.origin {
                ForwardOrigin::Reclaimed => (RegionRole::ForwardReclaimed, Safe),
                ForwardOrigin::Referenced => (
                    RegionRole::ForwardReferenced,
                    Deferred(DeferredReason::SplitOwnership),
                ),
            },
            Node::HeadingHint(_) => (
                RegionRole::HeadingHint,
                Deferred(DeferredReason::SplitOwnership),
            ),
            Node::MarginNote(_) => (
                RegionRole::MarginNote,
                Deferred(DeferredReason::SplitOwnership),
            ),
            Node::Container(_) => (
                RegionRole::Container,
                Deferred(DeferredReason::ContainerPairing),
            ),
            Node::Ruby(_) => (RegionRole::Ruby, Safe),
            Node::Heading(_) => (RegionRole::Heading, Safe),
            Node::Gaiji(_) => (RegionRole::Gaiji, Safe),
            Node::Warichu(_) => (RegionRole::Warichu, Safe),
            Node::AngleQuote(_) => (RegionRole::AngleQuote, Safe),
            Node::Kaeriten(_) => (RegionRole::Kaeriten, Safe),
            Node::Illustration(_) => (RegionRole::Illustration, Safe),
            Node::Line(_) => (RegionRole::Line, Safe),
            Node::PageBreak => (RegionRole::PageBreak, Safe),
            Node::SectionBreak(_) => (RegionRole::SectionBreak, Safe),
            Node::Directive(_) => (RegionRole::Directive, Safe),
            // `Node` is `#[non_exhaustive]`; an unknown future variant is
            // declined rather than assumed splice-safe.
            _ => (RegionRole::Other, Deferred(DeferredReason::Unclassified)),
        },
        // `NodeRef` is `#[non_exhaustive]`; decline an unknown future variant.
        _ => (RegionRole::Other, Deferred(DeferredReason::Unclassified)),
    }
}

const INTERSTITIAL: (RegionRole, SpliceSafety) = (
    RegionRole::Interstitial,
    SpliceSafety::Deferred(DeferredReason::Interstitial),
);

impl Tree<'_> {
    /// Project the source-node table into a complete tiling of the
    /// sanitized source: one [`OwnedRegion`] per classified node plus the
    /// interstitial plain runs between (and around) them.
    ///
    /// The regions are contiguous, non-overlapping, and ordered by start
    /// offset; the first starts at `0`, the last ends at the sanitized
    /// length, and concatenating each region's bytes reproduces
    /// [`Tree::to_source_verbatim`]
    /// exactly. An empty document yields a single empty interstitial
    /// region only when the source is non-empty; a truly empty source
    /// yields no regions.
    #[must_use]
    pub fn owned_regions(&self) -> Vec<OwnedRegion> {
        let nodes = self.source_nodes();
        // The sanitized length fits u32 — every offset in the tree is a
        // u32 `Span` — so the saturating fallback is never taken.
        let src_len = u32::try_from(self.sanitized().len()).unwrap_or(u32::MAX);
        let mut out: Vec<OwnedRegion> = Vec::with_capacity(nodes.len() * 2 + 1);
        let mut cursor: u32 = 0;
        for sn in nodes {
            let start = sn.source_span.start;
            if start > cursor {
                out.push(OwnedRegion {
                    span: Span::new(cursor, start),
                    role: INTERSTITIAL.0,
                    safety: INTERSTITIAL.1,
                });
            }
            let (role, safety) = classify_node_ref(sn.node);
            out.push(OwnedRegion {
                span: sn.source_span,
                role,
                safety,
            });
            cursor = sn.source_span.end;
        }
        if cursor < src_len {
            out.push(OwnedRegion {
                span: Span::new(cursor, src_len),
                role: INTERSTITIAL.0,
                safety: INTERSTITIAL.1,
            });
        }
        out
    }

    /// The [`OwnedRegion`] covering `off`, a sanitized-source byte offset.
    ///
    /// Returns the classified node region when `off` lands on a construct
    /// ([`O(log n)`](Tree::node_at_source)), or the surrounding
    /// interstitial run otherwise. Returns `None` only when `off` is past
    /// the end of the sanitized source.
    #[must_use]
    pub fn owned_region_at(&self, off: SourceOffset) -> Option<OwnedRegion> {
        // The sanitized length fits u32 — every offset in the tree is a
        // u32 `Span` — so the saturating fallback is never taken.
        let src_len = u32::try_from(self.sanitized().len()).unwrap_or(u32::MAX);
        if off.get() >= src_len {
            return None;
        }
        if let Some(sn) = self.node_at_source(off) {
            let (role, safety) = classify_node_ref(sn.node);
            return Some(OwnedRegion {
                span: sn.source_span,
                role,
                safety,
            });
        }
        // `off` falls in an interstitial gap. The nodes tile the source
        // contiguously and sorted by start, so the gap is bounded by the
        // end of the last node starting at/before `off` and the start of
        // the next node (or the source end).
        let nodes = self.source_nodes();
        let raw = off.get();
        let next_idx = nodes.partition_point(|n| n.source_span.start <= raw);
        let gap_start = if next_idx == 0 {
            0
        } else {
            nodes[next_idx - 1].source_span.end
        };
        let gap_end = nodes.get(next_idx).map_or(src_len, |n| n.source_span.start);
        Some(OwnedRegion {
            span: Span::new(gap_start, gap_end),
            role: INTERSTITIAL.0,
            safety: INTERSTITIAL.1,
        })
    }

    /// Produce minimal-diff source by replacing one [`Safe`](SpliceSafety::Safe)
    /// region's bytes with `replacement`.
    ///
    /// The result equals `verbatim[..region.span.start] + replacement +
    /// verbatim[region.span.end..]` where `verbatim` is
    /// [`Tree::to_source_verbatim`]:
    /// every byte outside the region is preserved exactly, unlike the
    /// whole-document canonicalisation of
    /// [`Tree::to_source`]. The caller typically
    /// re-parses the result (`Document::new(spliced)`) to obtain an
    /// updated tree.
    ///
    /// # Errors
    ///
    /// Returns [`SpliceError::Deferred`] for a region whose
    /// [`SpliceSafety`] is [`Deferred`](SpliceSafety::Deferred) (split or
    /// paired ownership) — these need a coupled splice not yet
    /// implemented (#202).
    ///
    /// # Panics
    ///
    /// Panics if `region` did not come from this tree (its span is out of
    /// bounds for the sanitized source, or not on a UTF-8 codepoint
    /// boundary). Regions from this tree's [`Tree::owned_regions`] /
    /// [`Tree::owned_region_at`] always satisfy the precondition.
    pub fn splice_source(
        &self,
        region: OwnedRegion,
        replacement: &str,
    ) -> Result<String, SpliceError> {
        match region.safety {
            SpliceSafety::Safe => {
                let src = self.sanitized();
                let start = region.span.start as usize;
                let end = region.span.end as usize;
                // Slicing validates the bounds and codepoint boundaries,
                // panicking on a region that did not come from this tree.
                let prefix = &src[..start];
                let suffix = &src[end..];
                let mut out = String::with_capacity(
                    prefix
                        .len()
                        .saturating_add(replacement.len())
                        .saturating_add(suffix.len()),
                );
                out.push_str(prefix);
                out.push_str(replacement);
                out.push_str(suffix);
                Ok(out)
            }
            SpliceSafety::Deferred(reason) => Err(SpliceError::Deferred {
                role: region.role,
                reason,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// Concatenating every owned region's bytes reproduces the verbatim
    /// (sanitized) source, and the regions form a gap-free, ordered,
    /// non-overlapping cover.
    fn assert_tiling(src: &str) {
        let doc = Document::new(src);
        let tree = doc.parse();
        let verbatim = tree.to_source_verbatim();
        let regions = tree.owned_regions();

        if verbatim.is_empty() {
            assert!(regions.is_empty(), "empty source must yield no regions");
            return;
        }

        assert_eq!(regions[0].span.start, 0, "tiling must start at 0");
        assert_eq!(
            regions.last().unwrap().span.end as usize,
            verbatim.len(),
            "tiling must end at the source length",
        );
        for pair in regions.windows(2) {
            assert_eq!(
                pair[0].span.end, pair[1].span.start,
                "regions must be contiguous with no gap/overlap",
            );
            assert!(
                pair[0].span.start < pair[0].span.end,
                "regions must be non-empty",
            );
        }
        let rebuilt: String = regions
            .iter()
            .map(|r| &verbatim[r.span.start as usize..r.span.end as usize])
            .collect();
        assert_eq!(
            rebuilt, verbatim,
            "region concatenation must equal verbatim"
        );

        // Identity splice of every Safe region is the verbatim source.
        for r in &regions {
            if r.safety == SpliceSafety::Safe {
                let same = &verbatim[r.span.start as usize..r.span.end as usize];
                assert_eq!(
                    tree.splice_source(*r, same).unwrap(),
                    verbatim,
                    "identity splice of a Safe region must be the verbatim source",
                );
            } else {
                assert!(
                    tree.splice_source(*r, "x").is_err(),
                    "a Deferred region must decline splice",
                );
            }
        }
    }

    /// Find the first region with the given role.
    fn role_of(src: &str, role: RegionRole) -> OwnedRegion {
        let doc = Document::new(src);
        let tree = doc.parse();
        tree.owned_regions()
            .into_iter()
            .find(|r| r.role == role)
            .unwrap_or_else(|| panic!("no {role:?} region in {src:?}"))
    }

    #[test]
    fn empty_source_has_no_regions() {
        assert_tiling("");
    }

    #[test]
    fn plain_text_is_one_interstitial() {
        assert_tiling("ただの本文です。");
        let doc = Document::new("ただの本文です。");
        let tree = doc.parse();
        let regions = tree.owned_regions();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].role, RegionRole::Interstitial);
    }

    #[test]
    fn ruby_is_safe_and_self_contained() {
        assert_tiling("｜青梅《おうめ》の実");
        let r = role_of("｜青梅《おうめ》の実", RegionRole::Ruby);
        assert_eq!(r.safety, SpliceSafety::Safe);
    }

    #[test]
    fn reclaimed_forward_is_safe() {
        // Adjacent forward bouten: the classifier pulls 青空 into the node.
        assert_tiling("青空［＃「青空」に傍点］の下");
        let r = role_of("青空［＃「青空」に傍点］の下", RegionRole::ForwardReclaimed);
        assert_eq!(r.safety, SpliceSafety::Safe);
    }

    #[test]
    fn referenced_forward_is_deferred() {
        // Non-adjacent: 青空 sits upstream, the bracket references it.
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        assert_tiling(src);
        let r = role_of(src, RegionRole::ForwardReferenced);
        assert_eq!(
            r.safety,
            SpliceSafety::Deferred(DeferredReason::SplitOwnership),
        );
    }

    #[test]
    fn container_markers_are_deferred() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        assert_tiling(src);
        let open = role_of(src, RegionRole::ContainerOpen);
        assert_eq!(
            open.safety,
            SpliceSafety::Deferred(DeferredReason::ContainerPairing),
        );
        let close = role_of(src, RegionRole::ContainerClose);
        assert_eq!(
            close.safety,
            SpliceSafety::Deferred(DeferredReason::ContainerPairing),
        );
    }

    #[test]
    fn gaiji_is_safe() {
        assert_tiling("※［＃「さんずい＋垂」、第3水準1-86-69］");
        let r = role_of("※［＃「さんずい＋垂」、第3水準1-86-69］", RegionRole::Gaiji);
        assert_eq!(r.safety, SpliceSafety::Safe);
    }

    #[test]
    fn splice_replaces_only_the_region() {
        // A real non-identity minimal-diff edit on a Safe (Reclaimed) node.
        let src = "青空［＃「青空」に傍点］の下を歩く";
        let doc = Document::new(src);
        let tree = doc.parse();
        let region = role_of(src, RegionRole::ForwardReclaimed);
        let spliced = tree
            .splice_source(region, "海［＃「海」に傍点］")
            .expect("Reclaimed forward is Safe");
        assert_eq!(spliced, "海［＃「海」に傍点］の下を歩く");
        // The neighbouring plain tail is byte-identical; only the region changed.
        assert!(spliced.ends_with("の下を歩く"));
        // And the result re-parses to the intended structure.
        let reparsed = Document::new(spliced.as_str());
        let rtree = reparsed.parse();
        assert!(
            rtree
                .owned_regions()
                .iter()
                .any(|r| r.role == RegionRole::ForwardReclaimed),
            "spliced source re-parses to a forward bouten",
        );
    }

    #[test]
    fn owned_region_at_finds_node_and_gap() {
        let src = "あ｜青梅《おうめ》い";
        let doc = Document::new(src);
        let tree = doc.parse();
        // Offset 0 is the leading plain 'あ'.
        let head = tree.owned_region_at(SourceOffset::new(0)).unwrap();
        assert_eq!(head.role, RegionRole::Interstitial);
        assert_eq!(head.span.start, 0);
        // An offset inside the ruby construct resolves to the Ruby node.
        let ruby_off = SourceOffset::new(tree.owned_regions()[1].span.start);
        let mid = tree.owned_region_at(ruby_off).unwrap();
        assert_eq!(mid.role, RegionRole::Ruby);
        // Past the end is None.
        assert!(
            tree.owned_region_at(SourceOffset::new(
                u32::try_from(tree.sanitized().len()).unwrap()
            ))
            .is_none(),
        );
    }
}
