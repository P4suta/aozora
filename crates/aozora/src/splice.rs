use core::error::Error;
use core::fmt;

use crate::render::spelling::source::container_close_source;
use crate::spec::{SourceOffset, Span};
use crate::syntax::{ForwardOrigin, RegionClose, RegionFormat};

use crate::syntax::ast::{Node, NodeRef};
use crate::{Document, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum RegionRole {
    Interstitial,
    Ruby,
    ForwardReclaimed,
    ForwardReferenced,
    ForwardSelfContained,
    ForwardDetached,
    Gaiji,
    Line,
    PageBreak,
    SectionBreak,
    BodyEnd,
    ForcedBreak,
    Heading,
    HeadingHint,
    HeadingSelfContained,
    Illustration,
    Kaeriten,
    Directive,
    AngleQuote,
    MarginNote,
    ContainerOpen,
    ContainerClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum CoupledKind {
    ForwardReference,
    HeadingHint,
    MarginNote,
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum SpliceSafety {
    Direct,
    Coupled(CoupledKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct Region {
    pub span: Span,
    pub role: RegionRole,
    pub safety: SpliceSafety,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct Coupling {
    pub kind: CoupledKind,
    pub primary: Span,
    pub partner: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum SpliceError {
    Unverifiable { role: RegionRole, kind: CoupledKind },
}

impl fmt::Display for SpliceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unverifiable { role, kind } => write!(
                f,
                "coupled {kind:?} edit of region {role:?} could not be verified \
                 (the candidate did not re-parse to the intended construct)"
            ),
        }
    }
}

impl Error for SpliceError {}

pub(crate) fn classify_node_ref(node: NodeRef) -> (RegionRole, SpliceSafety) {
    use SpliceSafety::{Coupled, Direct};

    match node {
        NodeRef::BlockOpen(_) => (RegionRole::ContainerOpen, Coupled(CoupledKind::Container)),
        NodeRef::BlockClose(_) => (RegionRole::ContainerClose, Coupled(CoupledKind::Container)),
        NodeRef::Inline(n) | NodeRef::BlockLeaf(n) => match n {
            Node::Format(f) => match f.origin {
                ForwardOrigin::Reclaimed => (RegionRole::ForwardReclaimed, Direct),
                ForwardOrigin::Referenced => (
                    RegionRole::ForwardReferenced,
                    Coupled(CoupledKind::ForwardReference),
                ),
                ForwardOrigin::SelfContained => (RegionRole::ForwardSelfContained, Direct),
                ForwardOrigin::Detached => (RegionRole::ForwardDetached, Direct),
            },
            Node::HeadingHint(h) => {
                if h.self_contained {
                    (RegionRole::HeadingSelfContained, Direct)
                } else {
                    (RegionRole::HeadingHint, Coupled(CoupledKind::HeadingHint))
                }
            }
            Node::MarginNote(_) => (RegionRole::MarginNote, Coupled(CoupledKind::MarginNote)),
            Node::Ruby(_) => (RegionRole::Ruby, Direct),
            Node::Heading(_) => (RegionRole::Heading, Direct),
            Node::Gaiji(_) => (RegionRole::Gaiji, Direct),
            Node::AngleQuote(_) => (RegionRole::AngleQuote, Direct),
            Node::Kaeriten(_) => (RegionRole::Kaeriten, Direct),
            Node::Illustration(_) => (RegionRole::Illustration, Direct),
            Node::Line(_) => (RegionRole::Line, Direct),
            Node::PageBreak => (RegionRole::PageBreak, Direct),
            Node::SectionBreak(_) => (RegionRole::SectionBreak, Direct),
            Node::BodyEnd => (RegionRole::BodyEnd, Direct),
            Node::ForcedBreak => (RegionRole::ForcedBreak, Direct),
            Node::Directive(_) => (RegionRole::Directive, Direct),
        },
    }
}

const INTERSTITIAL: (RegionRole, SpliceSafety) = (RegionRole::Interstitial, SpliceSafety::Direct);

// Accepting a self-contained reparse would skip the upstream rewrite required by a target change (ADR-0019).
fn reparsed_in_family(node: NodeRef, kind: CoupledKind) -> bool {
    let leaf = match node {
        NodeRef::Inline(n) | NodeRef::BlockLeaf(n) => Some(n),
        _ => None,
    };
    match kind {
        CoupledKind::ForwardReference => {
            matches!(leaf, Some(Node::Format(f)) if f.origin != ForwardOrigin::SelfContained)
        }
        CoupledKind::HeadingHint => {
            matches!(leaf, Some(Node::HeadingHint(h)) if !h.self_contained)
                || matches!(leaf, Some(Node::Heading(_)))
        }
        CoupledKind::MarginNote => matches!(leaf, Some(Node::MarginNote(_))),
        CoupledKind::Container => {
            matches!(node, NodeRef::BlockOpen(_) | NodeRef::BlockClose(_))
        }
    }
}

impl Snapshot {
    #[must_use]
    #[cfg(test)]
    pub(crate) fn regions(&self) -> Vec<Region> {
        let nodes = self.source_nodes();
        let src_len = u32::try_from(self.normalized_source().len()).unwrap_or(u32::MAX);
        let mut out: Vec<Region> = Vec::with_capacity(nodes.len() * 2 + 1);
        let mut cursor: u32 = 0;
        for sn in nodes {
            let start = sn.source_span.start;
            if start > cursor {
                out.push(Region {
                    span: Span::new(cursor, start),
                    role: INTERSTITIAL.0,
                    safety: INTERSTITIAL.1,
                });
            }
            let (role, safety) = classify_node_ref(sn.node);
            out.push(Region {
                span: sn.source_span,
                role,
                safety,
            });
            cursor = sn.source_span.end;
        }
        if cursor < src_len {
            out.push(Region {
                span: Span::new(cursor, src_len),
                role: INTERSTITIAL.0,
                safety: INTERSTITIAL.1,
            });
        }
        out
    }

    #[must_use]
    pub(crate) fn region_at(&self, off: SourceOffset) -> Option<Region> {
        let src_len = u32::try_from(self.normalized_source().len()).unwrap_or(u32::MAX);
        if off.get() >= src_len {
            return None;
        }
        if let Some(sn) = self.node_at_source(off) {
            let (role, safety) = classify_node_ref(sn.node);
            return Some(Region {
                span: sn.source_span,
                role,
                safety,
            });
        }
        let nodes = self.source_nodes();
        let raw = off.get();
        let next_idx = nodes.partition_point(|n| n.source_span.start <= raw);
        let gap_start = if next_idx == 0 {
            0
        } else {
            nodes[next_idx - 1].source_span.end
        };
        let gap_end = nodes.get(next_idx).map_or(src_len, |n| n.source_span.start);
        Some(Region {
            span: Span::new(gap_start, gap_end),
            role: INTERSTITIAL.0,
            safety: INTERSTITIAL.1,
        })
    }

    #[must_use]
    pub(crate) fn coupling(&self, region: Region) -> Option<Coupling> {
        match region.safety {
            SpliceSafety::Coupled(CoupledKind::Container) => self.container_coupling(region.span),
            SpliceSafety::Coupled(CoupledKind::MarginNote) => {
                self.margin_note_coupling(region.span)
            }
            SpliceSafety::Coupled(kind) => {
                let target = self.coupled_target_text(region.span)?;
                let partner = self.unique_upstream_plain(region.span.start, &target)?;
                Some(Coupling {
                    kind,
                    primary: region.span,
                    partner,
                })
            }
            _ => None,
        }
    }

    fn margin_note_coupling(&self, span: Span) -> Option<Coupling> {
        let source = self.normalized_source();
        let text = source.get(span.start as usize..span.end as usize)?;
        let marker = text.rfind("［＃")?;
        let (NodeRef::Inline(Node::MarginNote(note)) | NodeRef::BlockLeaf(Node::MarginNote(note))) =
            self.node_at_source(SourceOffset::new(span.start))?.node
        else {
            return None;
        };
        let base = self.node_store().content_range_as_plain(note.base)?;
        let base_start = marker.checked_sub(base.len())?;
        if text.get(base_start..marker) != Some(base) {
            return None;
        }
        let span_start = span.start as usize;
        let partner_start = span_start.checked_add(base_start)?;
        let marker_start = span_start.checked_add(marker)?;
        Some(Coupling {
            kind: CoupledKind::MarginNote,
            primary: Span::new(u32::try_from(marker_start).ok()?, span.end),
            partner: Span::new(
                u32::try_from(partner_start).ok()?,
                u32::try_from(marker_start).ok()?,
            ),
        })
    }

    pub(crate) fn splice(&self, region: Region, replacement: &str) -> Result<String, SpliceError> {
        match region.safety {
            SpliceSafety::Direct => Ok(splice_one(
                self.normalized_source(),
                region.span,
                replacement,
            )),
            SpliceSafety::Coupled(CoupledKind::Container) => {
                self.splice_container(region, replacement)
            }
            SpliceSafety::Coupled(CoupledKind::MarginNote) => {
                self.splice_margin_note(region, replacement)
            }
            SpliceSafety::Coupled(kind) => self.splice_split(region, kind, replacement),
        }
    }

    fn splice_margin_note(&self, region: Region, replacement: &str) -> Result<String, SpliceError> {
        let src = self.normalized_source();
        if replacement.is_empty() {
            return Ok(splice_one(src, region.span, replacement));
        }
        let unverifiable = SpliceError::Unverifiable {
            role: region.role,
            kind: CoupledKind::MarginNote,
        };
        let marker = replacement.rfind("［＃").ok_or(unverifiable)?;
        let base = replacement.get(..marker).ok_or(unverifiable)?;
        let directive = replacement.get(marker..).ok_or(unverifiable)?;
        let target = first_quoted(directive).ok_or(unverifiable)?;
        if base != target {
            return Err(unverifiable);
        }
        if !window_reforms_coupled(replacement, CoupledKind::MarginNote, target) {
            return Err(unverifiable);
        }
        Ok(splice_one(src, region.span, replacement))
    }

    fn container_coupling(&self, span: Span) -> Option<Coupling> {
        let (open, close) = self.container_pair_for(span)?;
        let primary = if span == open { open } else { close };
        let partner = if span == open { close } else { open };
        Some(Coupling {
            kind: CoupledKind::Container,
            primary,
            partner,
        })
    }

    fn container_pair_for(&self, span: Span) -> Option<(Span, Span)> {
        let mut stack: Vec<Span> = Vec::new();
        for sn in self.source_nodes() {
            match sn.node {
                NodeRef::BlockOpen(_) => stack.push(sn.source_span),
                NodeRef::BlockClose(_) => {
                    if let Some(open_span) = stack.pop() {
                        let close_span = sn.source_span;
                        if span == open_span || span == close_span {
                            return Some((open_span, close_span));
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn splice_container(&self, region: Region, replacement: &str) -> Result<String, SpliceError> {
        let unverifiable = SpliceError::Unverifiable {
            role: region.role,
            kind: CoupledKind::Container,
        };
        let src = self.normalized_source();
        let pair = self.container_pair_for(region.span);

        if replacement.is_empty() {
            return Ok(match pair {
                Some((open_span, close_span)) => splice_two(src, (open_span, ""), (close_span, "")),
                None => splice_one(src, region.span, ""),
            });
        }

        let Some((open_span, close_span)) = pair.filter(|(open, _)| *open == region.span) else {
            return if marker_in_family(replacement, CoupledKind::Container) {
                Ok(splice_one(src, region.span, replacement))
            } else {
                Err(unverifiable)
            };
        };

        let new_format = lone_open_format(replacement).ok_or(unverifiable)?;
        let old_format = block_open_format_at(self, open_span.start);

        if old_format.is_some_and(|old| RegionClose::of(old) == RegionClose::of(new_format)) {
            return Ok(splice_one(src, open_span, replacement));
        }

        let new_close = container_close_source(new_format);
        Ok(splice_two(
            src,
            (open_span, replacement),
            (close_span, &new_close),
        ))
    }

    // Scoped reparsing preserves minimal edits without making each verification scale with document size (ADR-0019).
    fn splice_split(
        &self,
        region: Region,
        kind: CoupledKind,
        replacement: &str,
    ) -> Result<String, SpliceError> {
        let src = self.normalized_source();
        if replacement.is_empty() {
            return Ok(splice_one(src, region.span, replacement));
        }
        let unverifiable = SpliceError::Unverifiable {
            role: region.role,
            kind,
        };
        let old_target = self.coupled_target_text(region.span).ok_or(unverifiable)?;

        let bracket_at = u32::try_from(old_target.len()).map_err(|_| unverifiable)?;
        let ctx = format!("{old_target}{replacement}");
        if reparsed_family_at(&ctx, bracket_at, kind) {
            return Ok(splice_one(src, region.span, replacement));
        }

        let new_target = first_quoted(replacement).ok_or(unverifiable)?;
        let occ = self
            .unique_upstream_plain(region.span.start, &old_target)
            .ok_or(unverifiable)?;
        let candidate = splice_two(src, (occ, new_target), (region.span, replacement));

        let win_start = occ.start as usize;
        let interstice = region.span.start as usize - occ.end as usize;
        let new_directive_start = win_start + new_target.len() + interstice;
        let win_end = new_directive_start + replacement.len();
        let reformed = candidate
            .get(win_start..win_end)
            .is_some_and(|w| window_reforms_coupled(w, kind, new_target));
        if reformed {
            Ok(candidate)
        } else {
            Err(unverifiable)
        }
    }

    fn coupled_target_text(&self, span: Span) -> Option<String> {
        let store = &self.node_store();
        let (NodeRef::Inline(leaf) | NodeRef::BlockLeaf(leaf)) =
            self.node_at_source(SourceOffset::new(span.start))?.node
        else {
            return None;
        };
        match leaf {
            Node::Format(f) => store.content_range_as_plain(f.target).map(str::to_owned),
            Node::HeadingHint(h) => Some(store.resolve_str(h.target).to_owned()),
            _ => None,
        }
    }

    fn unique_upstream_plain(&self, before: u32, target: &str) -> Option<Span> {
        let prefix = self.normalized_source().get(..before as usize)?;
        let mut hit: Option<usize> = None;
        let mut from = 0usize;
        while let Some(rel) = prefix.get(from..)?.find(target) {
            let at = from + rel;
            if hit.is_some() {
                return None;
            }
            hit = Some(at);
            from = at + target.len();
        }
        let start = u32::try_from(hit?).ok()?;
        let span = Span::new(start, start + u32::try_from(target.len()).ok()?);
        let region = self.region_at(SourceOffset::new(span.start))?;
        let carries_literal = matches!(
            region.role,
            RegionRole::Interstitial | RegionRole::ForwardDetached
        );
        if !carries_literal {
            return None;
        }
        (span.end <= region.span.end).then_some(span)
    }
}

fn block_open_format_at(tree: &Snapshot, start: u32) -> Option<RegionFormat> {
    tree.node_at_source(SourceOffset::new(start))
        .and_then(|sn| match sn.node {
            NodeRef::BlockOpen(f) if sn.source_span.start == start => Some(f),
            _ => None,
        })
}

fn lone_open_format(marker: &str) -> Option<RegionFormat> {
    let doc = Document::new(marker);
    match doc.snapshot().source_nodes().first() {
        Some(sn) if sn.source_span.start == 0 => match sn.node {
            NodeRef::BlockOpen(f) => Some(f),
            _ => None,
        },
        _ => None,
    }
}

fn marker_in_family(marker: &str, kind: CoupledKind) -> bool {
    let doc = Document::new(marker);
    doc.snapshot()
        .source_nodes()
        .first()
        .is_some_and(|sn| sn.source_span.start == 0 && reparsed_in_family(sn.node, kind))
}

fn reparsed_family_at(ctx: &str, off: u32, kind: CoupledKind) -> bool {
    let doc = Document::new(ctx);
    doc.snapshot()
        .node_at_source(SourceOffset::new(off))
        .is_some_and(|sn| reparsed_in_family(sn.node, kind))
}

fn first_quoted(directive: &str) -> Option<&str> {
    let after_open = directive.split_once('「')?.1;
    Some(after_open.split_once('」')?.0)
}

fn window_reforms_coupled(window: &str, kind: CoupledKind, new_target: &str) -> bool {
    let doc = Document::new(window);
    let tree = doc.snapshot();
    let store = &tree.node_store();
    tree.source_nodes().iter().any(|sn| {
        let (NodeRef::Inline(leaf) | NodeRef::BlockLeaf(leaf)) = sn.node else {
            return false;
        };
        let text = match (kind, leaf) {
            (CoupledKind::ForwardReference, Node::Format(f))
                if f.origin != ForwardOrigin::SelfContained =>
            {
                store.content_range_as_plain(f.target)
            }
            (CoupledKind::HeadingHint, Node::HeadingHint(h)) if !h.self_contained => {
                Some(store.resolve_str(h.target))
            }
            (CoupledKind::HeadingHint, Node::Heading(h)) => store.content_range_as_plain(h.text),
            (CoupledKind::MarginNote, Node::MarginNote(m)) => store.content_range_as_plain(m.base),
            _ => None,
        };
        text == Some(new_target)
    })
}

fn splice_one(src: &str, span: Span, replacement: &str) -> String {
    let start = span.start as usize;
    let end = span.end as usize;
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
    out
}

fn splice_two(src: &str, first: (Span, &str), second: (Span, &str)) -> String {
    let (a, repl_a) = first;
    let (b, repl_b) = second;
    debug_assert!(
        a.end <= b.start,
        "splice_two: regions must be ordered and disjoint"
    );
    let (a_start, a_end) = (a.start as usize, a.end as usize);
    let (b_start, b_end) = (b.start as usize, b.end as usize);
    let mut out = String::with_capacity(src.len() + repl_a.len() + repl_b.len());
    out.push_str(&src[..a_start]);
    out.push_str(repl_a);
    out.push_str(&src[a_end..b_start]);
    out.push_str(repl_b);
    out.push_str(&src[b_end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    fn assert_tiling(src: &str) {
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let verbatim = tree.to_source_verbatim();
        let regions = tree.regions();

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

        for r in &regions {
            let same = &verbatim[r.span.start as usize..r.span.end as usize];
            assert_eq!(
                tree.splice(*r, same).unwrap(),
                verbatim,
                "identity splice of {:?} must be the verbatim source",
                r.role,
            );
        }
    }

    fn role_of(src: &str, role: RegionRole) -> Region {
        let doc = Document::new(src);
        let tree = doc.snapshot();
        tree.regions()
            .into_iter()
            .find(|r| r.role == role)
            .unwrap_or_else(|| panic!("no {role:?} region in {src:?}"))
    }

    #[test]
    fn empty_source_has_no_regions() {
        assert_tiling("");
    }

    #[test]
    fn plain_text_is_one_direct_interstitial() {
        assert_tiling("ただの本文です。");
        let doc = Document::new("ただの本文です。");
        let tree = doc.snapshot();
        let regions = tree.regions();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].role, RegionRole::Interstitial);
        assert_eq!(regions[0].safety, SpliceSafety::Direct);
    }

    #[test]
    fn ruby_is_direct_and_self_contained() {
        assert_tiling("｜青梅《おうめ》の実");
        let r = role_of("｜青梅《おうめ》の実", RegionRole::Ruby);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn reclaimed_forward_is_direct() {
        assert_tiling("青空［＃「青空」に傍点］の下");
        let r = role_of("青空［＃「青空」に傍点］の下", RegionRole::ForwardReclaimed);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn referenced_forward_is_coupled() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        assert_tiling(src);
        let r = role_of(src, RegionRole::ForwardReferenced);
        assert_eq!(
            r.safety,
            SpliceSafety::Coupled(CoupledKind::ForwardReference)
        );
    }

    #[test]
    fn self_contained_forward_is_direct() {
        use crate::syntax::ForwardAttr;
        use crate::syntax::alloc::Allocator;

        let mut a = Allocator::new();
        let t = a.content_plain("X");
        let node = a.forward_format(ForwardAttr::Bold, t, ForwardOrigin::SelfContained);
        assert_eq!(
            classify_node_ref(NodeRef::Inline(node)),
            (RegionRole::ForwardSelfContained, SpliceSafety::Direct),
        );
    }

    #[test]
    fn self_contained_heading_is_direct() {
        let src = "本文［＃「序章」は中見出し］";
        assert_tiling(src);
        let r = role_of(src, RegionRole::HeadingSelfContained);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn container_markers_are_coupled() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        assert_tiling(src);
        let open = role_of(src, RegionRole::ContainerOpen);
        assert_eq!(open.safety, SpliceSafety::Coupled(CoupledKind::Container));
        let close = role_of(src, RegionRole::ContainerClose);
        assert_eq!(close.safety, SpliceSafety::Coupled(CoupledKind::Container));
    }

    #[test]
    fn gaiji_is_direct() {
        assert_tiling("※［＃「さんずい＋垂」、第3水準1-86-69］");
        let r = role_of("※［＃「さんずい＋垂」、第3水準1-86-69］", RegionRole::Gaiji);
        assert_eq!(r.safety, SpliceSafety::Direct);
    }

    #[test]
    fn direct_splice_replaces_only_the_region() {
        let src = "青空［＃「青空」に傍点］の下を歩く";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let region = role_of(src, RegionRole::ForwardReclaimed);
        let spliced = tree
            .splice(region, "海［＃「海」に傍点］")
            .expect("Reclaimed forward is Direct");
        assert_eq!(spliced, "海［＃「海」に傍点］の下を歩く");
    }

    #[test]
    fn coupling_pairs_container_markers() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let open = role_of(src, RegionRole::ContainerOpen);
        let c = tree.coupling(open).expect("container open is coupled");
        assert_eq!(c.kind, CoupledKind::Container);
        assert_eq!(c.primary, open.span);
        assert!(c.partner.start > c.primary.start);
        let close = role_of(src, RegionRole::ContainerClose);
        assert_eq!(c.partner, close.span);
    }

    #[test]
    fn container_kind_change_rewrites_both_markers() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let open = role_of(src, RegionRole::ContainerOpen);
        let spliced = tree
            .splice(open, "［＃ここから罫囲み］")
            .expect("kind change is verifiable");
        assert!(spliced.contains("［＃ここから罫囲み］"));
        assert!(spliced.contains("［＃罫囲み終わり］"));
        assert!(!spliced.contains("字下げ"));
        assert!(spliced.contains("本文"));
        let rt = Document::new(spliced.as_str());
        let rtree = rt.snapshot();
        assert!(
            rtree
                .regions()
                .iter()
                .any(|r| r.role == RegionRole::ContainerOpen)
        );
    }

    #[test]
    fn container_amount_change_touches_only_the_open() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let open = role_of(src, RegionRole::ContainerOpen);
        let spliced = tree
            .splice(open, "［＃ここから4字下げ］")
            .expect("amount change is verifiable");
        assert!(spliced.contains("［＃ここから4字下げ］"));
        assert!(spliced.contains("［＃ここで字下げ終わり］"));
    }

    #[test]
    fn mismatched_container_pair_identity_is_verbatim() {
        assert_tiling("前\n［＃ここから2字下げ］\n本文\n［＃ここで地付き終わり］\n後");
    }

    #[test]
    fn container_delete_drops_both_markers() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let open = role_of(src, RegionRole::ContainerOpen);
        let spliced = tree.splice(open, "").expect("delete is coherent");
        assert!(!spliced.contains("字下げ"));
        assert!(spliced.contains("本文"));
        assert!(spliced.contains("前"));
        assert!(spliced.contains("後"));
    }

    #[test]
    fn referenced_forward_attribute_change_is_coherent() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let spliced = tree
            .splice(r, "［＃「青空」に傍線］")
            .expect("attribute-only change keeps the forward");
        assert!(spliced.starts_with("青空がひろがる、その"));
        assert!(spliced.ends_with("［＃「青空」に傍線］"));
    }

    #[test]
    fn referenced_forward_target_change_is_coupled() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let spliced = tree
            .splice(r, "［＃「海」に傍点］")
            .expect("target change is a coupled edit");
        assert_eq!(spliced, "海がひろがる、その［＃「海」に傍点］");
        let rt = Document::new(spliced.as_str());
        assert!(
            rt.snapshot()
                .regions()
                .iter()
                .any(|r| r.role == RegionRole::ForwardReferenced)
        );
    }

    #[test]
    fn referenced_forward_emphasis_target_change_is_coupled() {
        let src = "青空がひろがる、その［＃「青空」は太字］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let spliced = tree
            .splice(r, "［＃「海」は太字］")
            .expect("emphasis target change is a coupled edit");
        assert_eq!(spliced, "海がひろがる、その［＃「海」は太字］");
    }

    #[test]
    fn heading_hint_target_change_is_coupled() {
        let src = "序章、その［＃「序章」は中見出し］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::HeadingHint);
        assert_eq!(r.safety, SpliceSafety::Coupled(CoupledKind::HeadingHint));
        let spliced = tree
            .splice(r, "［＃「海」は中見出し］")
            .expect("heading target change is a coupled edit");
        assert_eq!(spliced, "海、その［＃「海」は中見出し］");
    }

    #[test]
    fn ambiguous_referent_target_change_declines() {
        let src = "青空と青空、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let err = tree
            .splice(r, "［＃「海」に傍点］")
            .expect_err("ambiguous referent declines");
        assert!(matches!(err, SpliceError::Unverifiable { .. }));
    }

    #[test]
    fn unique_upstream_plain_requires_one_literal_region() {
        let plain_source = "青空、その";
        let plain = Document::new(plain_source).snapshot();
        assert_eq!(
            plain.unique_upstream_plain(
                u32::try_from(plain_source.len()).expect("test source length fits u32"),
                "青空"
            ),
            Some(Span::new(
                0,
                u32::try_from("青空".len()).expect("test target length fits u32")
            ))
        );

        let ruby_source = "｜青空《あおぞら》その";
        let ruby = Document::new(ruby_source).snapshot();
        assert_eq!(
            ruby.unique_upstream_plain(
                u32::try_from(ruby_source.len()).expect("test source length fits u32"),
                "青空"
            ),
            None
        );

        let crossing_source = "青｜空《そら》後";
        let crossing = Document::new(crossing_source).snapshot();
        assert_eq!(
            crossing.unique_upstream_plain(
                u32::try_from(crossing_source.len()).expect("test source length fits u32"),
                "青｜"
            ),
            None
        );
    }

    #[test]
    fn multi_target_forward_identity_is_a_noop() {
        assert_tiling("AとB［＃「A」「B」に傍点］");
    }

    #[test]
    fn multi_target_forward_target_change_declines() {
        let src = "AとB［＃「A」「B」に傍点］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::ForwardReferenced);
        assert_eq!(
            r.safety,
            SpliceSafety::Coupled(CoupledKind::ForwardReference)
        );
        let err = tree
            .splice(r, "［＃「海」に傍点］")
            .expect_err("a multi-segment target change is irreducible");
        assert!(matches!(err, SpliceError::Unverifiable { .. }));
    }

    #[test]
    fn region_at_finds_node_and_gap() {
        let src = "あ｜青梅《おうめ》い";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let head = tree.region_at(SourceOffset::new(0)).unwrap();
        assert_eq!(head.role, RegionRole::Interstitial);
        assert_eq!(head.span.start, 0);
        let ruby_off = SourceOffset::new(tree.regions()[1].span.start);
        let mid = tree.region_at(ruby_off).unwrap();
        assert_eq!(mid.role, RegionRole::Ruby);
        assert!(
            tree.region_at(SourceOffset::new(
                u32::try_from(tree.normalized_source().len()).unwrap()
            ))
            .is_none(),
        );
    }

    #[test]
    fn splice_error_display_is_specific_and_nonempty() {
        let unverifiable = SpliceError::Unverifiable {
            role: RegionRole::ForwardReferenced,
            kind: CoupledKind::ForwardReference,
        };
        let s = unverifiable.to_string();
        assert!(s.contains("ForwardReferenced"), "got {s:?}");
        assert!(s.contains("ForwardReference"), "got {s:?}");
        assert!(s.contains("could not be verified"), "got {s:?}");
    }

    #[test]
    fn classify_pins_every_leaf_variant() {
        use crate::syntax::alloc::Allocator;
        use crate::syntax::{DirectiveKind, HeadingKind, HeadingStyle, LineFormat, SectionKind};

        let mut a = Allocator::new();
        let heading_text = a.content_plain("章");
        let heading = a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, heading_text);
        let aq = a.content_plain("重要");
        let angle_quote = a.angle_quote(aq);
        let kaeriten = a.kaeriten("（レ）");
        let illustration = a.sashie_general("fig.png", "図", None);
        let directive_payload = a.make_directive("［＃ママ］", DirectiveKind::Sic);
        let directive = a.annotation(directive_payload);
        let line = a.line(LineFormat::Indent {
            amount: 2,
            end_offset: None,
        });
        let section_break = a.section_break(SectionKind::Kaicho);
        let page_break = a.page_break();
        let body_end = a.body_end();
        let forced_break = a.forced_break();
        let cases: [(Node, RegionRole, SpliceSafety); 10] = [
            (heading, RegionRole::Heading, SpliceSafety::Direct),
            (angle_quote, RegionRole::AngleQuote, SpliceSafety::Direct),
            (kaeriten, RegionRole::Kaeriten, SpliceSafety::Direct),
            (illustration, RegionRole::Illustration, SpliceSafety::Direct),
            (line, RegionRole::Line, SpliceSafety::Direct),
            (page_break, RegionRole::PageBreak, SpliceSafety::Direct),
            (
                section_break,
                RegionRole::SectionBreak,
                SpliceSafety::Direct,
            ),
            (body_end, RegionRole::BodyEnd, SpliceSafety::Direct),
            (forced_break, RegionRole::ForcedBreak, SpliceSafety::Direct),
            (directive, RegionRole::Directive, SpliceSafety::Direct),
        ];
        for (node, role, safety) in cases {
            assert_eq!(
                classify_node_ref(NodeRef::Inline(node)),
                (role, safety),
                "classify mismatch for {role:?}",
            );
        }
    }

    #[test]
    fn reparsed_in_family_heading_hint_accepts_hint_or_promoted_heading() {
        use crate::syntax::alloc::Allocator;
        use crate::syntax::{HeadingKind, HeadingStyle};

        let mut a = Allocator::new();
        let text = a.content_plain("章");
        let promoted = a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, text);
        let hint = a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "序章", false);
        let hint_sc = a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "序章", true);

        assert!(reparsed_in_family(
            NodeRef::Inline(promoted),
            CoupledKind::HeadingHint
        ));
        assert!(reparsed_in_family(
            NodeRef::Inline(hint),
            CoupledKind::HeadingHint
        ));
        assert!(!reparsed_in_family(
            NodeRef::Inline(hint_sc),
            CoupledKind::HeadingHint
        ));
    }

    #[test]
    fn region_at_gap_start_is_previous_node_end() {
        let src = "あ｜青梅《おうめ》い｜里芋《さといも》う｜大豆《だいず》え";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let regions = tree.regions();
        let gap = regions[2];
        assert_eq!(gap.role, RegionRole::Interstitial);
        let got = tree.region_at(SourceOffset::new(gap.span.start)).unwrap();
        assert_eq!(
            got.span, gap.span,
            "gap must span (prev node end .. next node start)",
        );
    }

    #[test]
    fn coupling_of_forward_reference_returns_upstream_partner() {
        let src = "まず青空がひろがる、その［＃「青空」に傍点］";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let r = role_of(src, RegionRole::ForwardReferenced);
        let c = tree.coupling(r).expect("forward reference is coupled");
        assert_eq!(c.kind, CoupledKind::ForwardReference);
        assert_eq!(c.primary, r.span);
        assert_eq!(c.partner.start, 6, "partner starts after まず, not at 0");
        let partner_text =
            &tree.normalized_source()[c.partner.start as usize..c.partner.end as usize];
        assert_eq!(partner_text, "青空");
    }

    #[test]
    fn coupling_of_container_close_returns_open_partner() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let close = role_of(src, RegionRole::ContainerClose);
        let c = tree.coupling(close).expect("close couples to its open");
        assert_eq!(c.kind, CoupledKind::Container);
        assert_eq!(c.primary, close.span);
        let open = role_of(src, RegionRole::ContainerOpen);
        assert_eq!(c.partner, open.span);
        assert!(c.partner.start < c.primary.start, "open precedes close");
    }

    #[test]
    fn block_open_format_at_requires_offset_at_node_start() {
        let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let open = role_of(src, RegionRole::ContainerOpen);
        assert!(block_open_format_at(&tree, open.span.start).is_some());
        assert!(block_open_format_at(&tree, open.span.start + 3).is_none());
    }

    #[test]
    fn lone_open_format_requires_first_node_at_offset_zero() {
        assert!(lone_open_format("［＃ここから2字下げ］").is_some());
        assert!(lone_open_format("前置き\n［＃ここから2字下げ］").is_none());
    }

    #[test]
    fn marker_in_family_needs_container_at_offset_zero() {
        assert!(marker_in_family(
            "［＃ここから2字下げ］",
            CoupledKind::Container
        ));
        assert!(!marker_in_family(
            "前\n［＃ここから2字下げ］",
            CoupledKind::Container
        ));
        assert!(!marker_in_family("ただの本文", CoupledKind::Container));
    }

    #[test]
    fn window_reforms_coupled_pins_families_and_guards() {
        assert!(window_reforms_coupled(
            "海がひろがる、その［＃「海」に傍点］",
            CoupledKind::ForwardReference,
            "海",
        ));
        assert!(!window_reforms_coupled(
            "［＃「海」に傍点］",
            CoupledKind::ForwardReference,
            "海",
        ));
        assert!(window_reforms_coupled(
            "海、その［＃「海」は中見出し］",
            CoupledKind::HeadingHint,
            "海",
        ));
        assert!(!window_reforms_coupled(
            "［＃「海」は中見出し］",
            CoupledKind::HeadingHint,
            "海",
        ));
        assert!(window_reforms_coupled(
            "海\n［＃「海」は中見出し］",
            CoupledKind::HeadingHint,
            "海",
        ));
        assert!(window_reforms_coupled(
            "青空［＃「青空」の左に「あお」の注記］",
            CoupledKind::MarginNote,
            "青空",
        ));
        assert!(!window_reforms_coupled(
            "ただの本文",
            CoupledKind::ForwardReference,
            "海",
        ));
        assert!(!window_reforms_coupled(
            "海がひろがる、その［＃「海」に傍点］",
            CoupledKind::ForwardReference,
            "山",
        ));
    }
}
