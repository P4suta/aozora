//! Cross-store **graft** visitor: deep-copy an owned node's payloads from a
//! source [`NodeStore`] into a destination store, minting fresh handles.
//!
//! The owned-table splice re-lexes a document region in isolation, producing a
//! small [`OwnedLexOutput`](super::OwnedLexOutput) with its OWN [`NodeStore`] —
//! its own [`StrInterner`](super::StrInterner) (so [`StrId`]s count from `0`)
//! and its own flat `contents` / `segments` pools (indices from `0`). To splice
//! the re-lexed region's nodes into the cached store, every handle they carry
//! ([`StrId`], [`ContentRange`], [`SegRange`]) must be **re-homed** into the
//! cached store.
//!
//! This module is that re-homing pass. [`NodeStore::graft_node_ref`] /
//! [`NodeStore::graft_node`] walk a node, deep-copy each payload from `src` into
//! `self`, and return a node whose handles address `self`. The returned node
//! resolves (via `self`) to byte-identical strings and structure as the input
//! resolves via `src`. The destination interner deduplicates transparently, so
//! grafting into a non-empty store appends without disturbing prior handles.
//!
//! Every recursion arm matches its (same-crate `#[non_exhaustive]`) enum
//! exhaustively with no wildcard, so a future variant fails to compile here
//! until its handles are explicitly grafted.

use super::intern::StrId;
use super::payload::{
    AngleQuoteOwned, ContentOwned, DirectiveOwned, ForwardFormatOwned, GaijiCanonicalOwned,
    GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned, KaeritenOwned, MarginNoteOwned,
    NodeOwned, RubyOwned, SegmentOwned, WarichuOwned,
};
use super::registry::NodeRefOwned;
use super::store::{ContentRange, NodeStore, SegRange};

impl NodeStore {
    /// Deep-copy `node`'s payloads from `src` into `self`, returning a node
    /// whose [`StrId`] / [`ContentRange`] / [`SegRange`] handles address
    /// `self`. The returned node resolves (via this store) to byte-identical
    /// strings and structure as `node` resolves via `src`. Used by the
    /// owned-table splice to graft a re-lexed region's nodes onto the cached
    /// store.
    ///
    /// `src` and `self` are distinct stores (the splice's region output and the
    /// cached store); no aliasing.
    ///
    /// # Panics
    ///
    /// Panics if any handle carried by `node` was not produced by `src`'s
    /// interner / pools (via the underlying `resolve_*`).
    #[must_use]
    pub fn graft_node_ref(&mut self, src: &Self, node: NodeRefOwned) -> NodeRefOwned {
        match node {
            NodeRefOwned::Inline(n) => NodeRefOwned::Inline(self.graft_node(src, n)),
            NodeRefOwned::BlockLeaf(n) => NodeRefOwned::BlockLeaf(self.graft_node(src, n)),
            // `RegionFormat` / `RegionClose` are scalar-only (bool / enum / u8 /
            // NonZero / IndentBlock / BlockStyles) — no StrId / &str — so the
            // container discriminants carry no handles to re-home.
            NodeRefOwned::BlockOpen(rf) => NodeRefOwned::BlockOpen(rf),
            NodeRefOwned::BlockClose(rc) => NodeRefOwned::BlockClose(rc),
        }
    }

    /// Deep-copy `node`'s payloads from `src` into `self`, returning a node
    /// whose handles address `self`. See [`Self::graft_node_ref`].
    ///
    /// # Panics
    ///
    /// Panics if any handle carried by `node` was not produced by `src`.
    #[must_use]
    pub fn graft_node(&mut self, src: &Self, node: NodeOwned) -> NodeOwned {
        match node {
            NodeOwned::Ruby(r) => NodeOwned::Ruby(RubyOwned {
                base: graft_content_range(src, self, r.base),
                reading: graft_content_range(src, self, r.reading),
                side: r.side,
            }),
            NodeOwned::Format(f) => NodeOwned::Format(ForwardFormatOwned {
                attr: f.attr,
                target: graft_content_range(src, self, f.target),
                origin: f.origin,
            }),
            NodeOwned::Gaiji(g) => NodeOwned::Gaiji(graft_gaiji(src, self, g)),
            // `LineFormat` is a Copy scalar enum with no handles.
            NodeOwned::Line(l) => NodeOwned::Line(l),
            NodeOwned::Warichu(w) => NodeOwned::Warichu(WarichuOwned {
                upper: graft_content(src, self, w.upper),
                lower: graft_content(src, self, w.lower),
            }),
            NodeOwned::PageBreak => NodeOwned::PageBreak,
            NodeOwned::SectionBreak(k) => NodeOwned::SectionBreak(k),
            NodeOwned::BodyEnd => NodeOwned::BodyEnd,
            NodeOwned::ForcedBreak => NodeOwned::ForcedBreak,
            NodeOwned::Heading(h) => NodeOwned::Heading(HeadingOwned {
                kind: h.kind,
                style: h.style,
                text: graft_content_range(src, self, h.text),
            }),
            NodeOwned::HeadingHint(h) => NodeOwned::HeadingHint(HeadingHintOwned {
                level: h.level,
                style: h.style,
                target: graft_str(src, self, h.target),
            }),
            NodeOwned::Illustration(i) => NodeOwned::Illustration(IllustrationOwned {
                file: graft_str(src, self, i.file),
                number: i.number.map(|id| graft_str(src, self, id)),
                dimensions: i.dimensions.map(|id| graft_str(src, self, id)),
                caption: i.caption.map(|c| graft_content(src, self, c)),
                description: i.description.map(|id| graft_str(src, self, id)),
            }),
            NodeOwned::Kaeriten(k) => NodeOwned::Kaeriten(KaeritenOwned {
                mark: graft_str(src, self, k.mark),
            }),
            NodeOwned::Directive(d) => NodeOwned::Directive(graft_directive(src, self, d)),
            NodeOwned::AngleQuote(a) => NodeOwned::AngleQuote(AngleQuoteOwned {
                content: graft_content_range(src, self, a.content),
            }),
            NodeOwned::MarginNote(m) => NodeOwned::MarginNote(MarginNoteOwned {
                kind: m.kind,
                base: graft_content_range(src, self, m.base),
                note: graft_content_range(src, self, m.note),
            }),
            // `Container` is a Copy scalar enum with no handles.
            NodeOwned::Container(c) => NodeOwned::Container(c),
        }
    }
}

/// Re-intern `id`'s bytes from `src` into `dst`, returning the dst handle.
fn graft_str(src: &NodeStore, dst: &mut NodeStore, id: StrId) -> StrId {
    dst.intern(src.resolve_str(id))
}

/// Deep-copy a content run from `src` into `dst`. The resolved slice is copied
/// out first (`ContentOwned` is `Copy`) to drop the `src` borrow before the
/// per-item graft mutates `dst`.
fn graft_content_range(src: &NodeStore, dst: &mut NodeStore, r: ContentRange) -> ContentRange {
    let items: Vec<ContentOwned> = src.resolve_content_range(r).to_vec();
    let mapped: Vec<ContentOwned> = items
        .into_iter()
        .map(|c| graft_content(src, dst, c))
        .collect();
    dst.push_contents(&mapped)
}

/// Deep-copy one body content from `src` into `dst`.
fn graft_content(src: &NodeStore, dst: &mut NodeStore, c: ContentOwned) -> ContentOwned {
    match c {
        ContentOwned::Plain(id) => ContentOwned::Plain(graft_str(src, dst, id)),
        ContentOwned::Segments(sr) => ContentOwned::Segments(graft_seg_range(src, dst, sr)),
    }
}

/// Deep-copy a segment run from `src` into `dst`. The resolved slice is copied
/// out first (`SegmentOwned` is `Copy`) before the per-item graft mutates `dst`.
fn graft_seg_range(src: &NodeStore, dst: &mut NodeStore, sr: SegRange) -> SegRange {
    let items: Vec<SegmentOwned> = src.resolve_seg_range(sr).to_vec();
    let mapped: Vec<SegmentOwned> = items.into_iter().map(|s| graft_seg(src, dst, s)).collect();
    dst.push_segments(&mapped)
}

/// Deep-copy one segment from `src` into `dst`.
fn graft_seg(src: &NodeStore, dst: &mut NodeStore, s: SegmentOwned) -> SegmentOwned {
    match s {
        SegmentOwned::Text(id) => SegmentOwned::Text(graft_str(src, dst, id)),
        SegmentOwned::Gaiji(g) => SegmentOwned::Gaiji(graft_gaiji(src, dst, g)),
        SegmentOwned::Directive(d) => SegmentOwned::Directive(graft_directive(src, dst, d)),
    }
}

/// Deep-copy a 外字 payload from `src` into `dst`. Only `hint` and the
/// `Unresolved { mencode: Some(_) }` tail carry handles; the `MenKuTen` /
/// `Unicode` / `Unresolved { None }` canonical arms are scalar.
fn graft_gaiji(src: &NodeStore, dst: &mut NodeStore, g: GaijiOwned) -> GaijiOwned {
    let canonical = match g.canonical {
        GaijiCanonicalOwned::MenKuTen(m) => GaijiCanonicalOwned::MenKuTen(m),
        GaijiCanonicalOwned::Unicode(c) => GaijiCanonicalOwned::Unicode(c),
        GaijiCanonicalOwned::Unresolved { mencode } => GaijiCanonicalOwned::Unresolved {
            mencode: mencode.map(|id| graft_str(src, dst, id)),
        },
    };
    GaijiOwned {
        hint: graft_str(src, dst, g.hint),
        canonical,
        standalone: g.standalone,
    }
}

/// Deep-copy a generic directive from `src` into `dst`. Only `raw` is a handle;
/// `kind` is a Copy scalar.
fn graft_directive(src: &NodeStore, dst: &mut NodeStore, d: DirectiveOwned) -> DirectiveOwned {
    DirectiveOwned {
        raw: graft_str(src, dst, d.raw),
        kind: d.kind,
    }
}

#[cfg(test)]
mod tests {
    use aozora_encoding::gaiji::MenKuTen;

    use super::*;
    use crate::alloc_owned::OwnedAllocator;
    use crate::format::{ForwardAttr, ForwardOrigin, RegionFormat};
    use crate::{Container, DirectiveKind, HeadingKind, HeadingStyle, MarginNoteKind};

    /// Fully project a node to a store-independent `String`: a variant tag plus
    /// every resolved string, recursively. Equal projections from two stores
    /// prove the graft preserved every resolvable byte and the structure.
    fn project_node(store: &NodeStore, node: NodeOwned) -> String {
        match node {
            NodeOwned::Ruby(r) => format!(
                "Ruby[base={};reading={};side={:?}]",
                project_content_range(store, r.base),
                project_content_range(store, r.reading),
                r.side,
            ),
            NodeOwned::Format(f) => format!(
                "Format[attr={:?};target={};origin={:?}]",
                f.attr,
                project_content_range(store, f.target),
                f.origin,
            ),
            NodeOwned::Gaiji(g) => format!("Gaiji[{}]", project_gaiji(store, g)),
            NodeOwned::Line(l) => format!("Line[{l:?}]"),
            NodeOwned::Warichu(w) => format!(
                "Warichu[upper={};lower={}]",
                project_content(store, w.upper),
                project_content(store, w.lower),
            ),
            NodeOwned::PageBreak => "PageBreak".to_owned(),
            NodeOwned::SectionBreak(k) => format!("SectionBreak[{k:?}]"),
            NodeOwned::BodyEnd => "BodyEnd".to_owned(),
            NodeOwned::ForcedBreak => "ForcedBreak".to_owned(),
            NodeOwned::Heading(h) => format!(
                "Heading[kind={:?};style={:?};text={}]",
                h.kind,
                h.style,
                project_content_range(store, h.text),
            ),
            NodeOwned::HeadingHint(h) => format!(
                "HeadingHint[level={:?};style={:?};target={:?}]",
                h.level,
                h.style,
                store.resolve_str(h.target),
            ),
            NodeOwned::Illustration(i) => format!(
                "Illustration[file={:?};number={:?};dimensions={:?};caption={:?};description={:?}]",
                store.resolve_str(i.file),
                i.number.map(|id| store.resolve_str(id).to_owned()),
                i.dimensions.map(|id| store.resolve_str(id).to_owned()),
                i.caption.map(|c| project_content(store, c)),
                i.description.map(|id| store.resolve_str(id).to_owned()),
            ),
            NodeOwned::Kaeriten(k) => format!("Kaeriten[{:?}]", store.resolve_str(k.mark)),
            NodeOwned::Directive(d) => format!("Directive[{}]", project_directive(store, d)),
            NodeOwned::AngleQuote(a) => {
                format!("AngleQuote[{}]", project_content_range(store, a.content))
            }
            NodeOwned::MarginNote(m) => format!(
                "MarginNote[kind={:?};base={};note={}]",
                m.kind,
                project_content_range(store, m.base),
                project_content_range(store, m.note),
            ),
            NodeOwned::Container(c) => format!("Container[{c:?}]"),
        }
    }

    fn project_content_range(store: &NodeStore, r: ContentRange) -> String {
        let parts: Vec<String> = store
            .resolve_content_range(r)
            .iter()
            .map(|c| project_content(store, *c))
            .collect();
        format!("<{}>", parts.join(","))
    }

    fn project_content(store: &NodeStore, c: ContentOwned) -> String {
        match c {
            ContentOwned::Plain(id) => format!("P:{:?}", store.resolve_str(id)),
            ContentOwned::Segments(sr) => {
                let parts: Vec<String> = store
                    .resolve_seg_range(sr)
                    .iter()
                    .map(|s| project_seg(store, *s))
                    .collect();
                format!("S[{}]", parts.join(","))
            }
        }
    }

    fn project_seg(store: &NodeStore, s: SegmentOwned) -> String {
        match s {
            SegmentOwned::Text(id) => format!("T:{:?}", store.resolve_str(id)),
            SegmentOwned::Gaiji(g) => format!("G[{}]", project_gaiji(store, g)),
            SegmentOwned::Directive(d) => format!("D[{}]", project_directive(store, d)),
        }
    }

    fn project_gaiji(store: &NodeStore, g: GaijiOwned) -> String {
        let canonical = match g.canonical {
            GaijiCanonicalOwned::MenKuTen(m) => format!("MenKuTen{m:?}"),
            GaijiCanonicalOwned::Unicode(c) => format!("Unicode{c:?}"),
            GaijiCanonicalOwned::Unresolved { mencode } => format!(
                "Unresolved{:?}",
                mencode.map(|id| store.resolve_str(id).to_owned()),
            ),
        };
        format!(
            "hint={:?};canonical={canonical};standalone={}",
            store.resolve_str(g.hint),
            g.standalone,
        )
    }

    fn project_directive(store: &NodeStore, d: DirectiveOwned) -> String {
        format!("raw={:?};kind={:?}", store.resolve_str(d.raw), d.kind)
    }

    /// Build a corpus-like spread of every handle-bearing variant into one
    /// allocator (so they share a single source store), returning the nodes.
    fn build_source_nodes(a: &mut OwnedAllocator) -> Vec<NodeOwned> {
        let mut nodes = Vec::new();

        // Ruby with a multi-content base (a Segments run) and a plain reading.
        let g = a.make_gaiji("挿絵字", Some("第3水準1-85-54"), false);
        let seg_g = a.seg_gaiji(g);
        let seg_t = a.seg_text("基");
        let base = a.content_segments(&[seg_t, seg_g]);
        let reading = a.content_plain("よみ");
        nodes.push(a.ruby(base, reading));

        // Forward format (bold) over a plain target.
        let target = a.content_plain("強調");
        nodes.push(a.forward_format(ForwardAttr::Bold, target, ForwardOrigin::Reclaimed));

        // Gaiji nodes for every GaijiCanonicalOwned arm.
        let g_unicode = a.make_gaiji("竜", Some("U+9F8D"), false); // Unicode
        nodes.push(a.gaiji(g_unicode));
        let g_menkuten = a.make_gaiji("熙", Some("第3水準1-14-29"), false); // MenKuTen
        nodes.push(a.gaiji(g_menkuten));
        let g_unres_some = a.make_gaiji("謎", Some("未知の注記"), false); // Unresolved{Some}
        nodes.push(a.gaiji(g_unres_some));
        let g_unres_none = a.make_gaiji("謎", None, true); // Unresolved{None}
        nodes.push(a.gaiji(g_unres_none));

        // Warichu with bare-content upper / lower.
        let upper = a.content_plain("上");
        let lower = a.content_plain("下");
        nodes.push(a.warichu(upper, lower));

        // Heading.
        let htext = a.content_plain("第一章");
        nodes.push(a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, htext));

        // HeadingHint.
        nodes.push(a.heading_hint(HeadingKind::Medium, HeadingStyle::Window, "序章"));

        // Illustration with ALL options Some.
        let caption = a.content_plain("図一");
        nodes.push(a.sashie("cover.png", Some("1"), Some("横100×縦200"), Some(caption)));
        // Illustration with caption / number Some via general form options None.
        nodes.push(a.sashie("bare.png", None, None, None));

        // Kaeriten.
        nodes.push(a.kaeriten("（レ）"));

        // Directive.
        let d = a.make_directive("［＃ママ］", DirectiveKind::Sic);
        nodes.push(a.annotation(d));

        // AngleQuote.
        let content = a.content_plain("重要");
        nodes.push(a.angle_quote(content));

        // MarginNote.
        let mbase = a.content_plain("未来");
        let mnote = a.content_plain("みらい");
        nodes.push(a.side_note(MarginNoteKind::Gloss, mbase, mnote));

        // A Segments content run exercising Text + Gaiji + Directive, wrapped in
        // an AngleQuote so it lives inside a ContentRange.
        let s_text = a.seg_text("前");
        let s_g = a.make_gaiji("謎字", Some("第3水準1-90-1"), false);
        let s_gaiji = a.seg_gaiji(s_g);
        let s_d = a.make_directive("［＃割り注］", DirectiveKind::Sic);
        let s_dir = a.seg_annotation(s_d);
        let mixed = a.content_segments(&[s_text, s_gaiji, s_dir]);
        nodes.push(a.angle_quote(mixed));

        // Handle-free scalar variants (graft passthrough).
        nodes.push(a.page_break());
        nodes.push(a.body_end());
        nodes.push(a.forced_break());
        nodes.push(a.container(Container {
            kind: RegionFormat::Framed,
        }));

        nodes
    }

    #[test]
    fn graft_round_trips_every_handle_bearing_variant() {
        let mut a = OwnedAllocator::new();
        let nodes = build_source_nodes(&mut a);
        let src = a.into_store();

        for &node in &nodes {
            let mut dst = NodeStore::new();
            let grafted = dst.graft_node(&src, node);
            assert_eq!(
                project_node(&src, node),
                project_node(&dst, grafted),
                "graft must preserve every resolvable byte + structure for {node:?}",
            );
        }
    }

    #[test]
    fn graft_node_ref_wraps_inline_and_passes_containers() {
        use crate::format::RegionClose;

        let mut a = OwnedAllocator::new();
        let base = a.content_plain("日本");
        let reading = a.content_plain("にほん");
        let ruby = a.ruby(base, reading);
        let src = a.into_store();

        let mut dst = NodeStore::new();

        // Inline / BlockLeaf re-home their payload.
        let NodeRefOwned::Inline(gi) = dst.graft_node_ref(&src, NodeRefOwned::Inline(ruby)) else {
            panic!("expected an inline graft result");
        };
        assert_eq!(project_node(&src, ruby), project_node(&dst, gi));

        let NodeRefOwned::BlockLeaf(gl) = dst.graft_node_ref(&src, NodeRefOwned::BlockLeaf(ruby))
        else {
            panic!("expected a block-leaf graft result");
        };
        assert_eq!(project_node(&src, ruby), project_node(&dst, gl));

        // Container open / close discriminants pass through unchanged.
        let open = NodeRefOwned::BlockOpen(RegionFormat::Framed);
        assert_eq!(dst.graft_node_ref(&src, open), open);
        let close = NodeRefOwned::BlockClose(RegionClose::Framed);
        assert_eq!(dst.graft_node_ref(&src, close), close);
    }

    #[test]
    fn graft_into_nonempty_dst_preserves_prior_entries() {
        // Pre-populate dst with unrelated strings + a node, capture their
        // projection, then graft a fresh node and confirm the prior handles
        // still resolve to the same bytes (graft only appends).
        let mut a = OwnedAllocator::new();
        let pre_base = a.content_plain("既存");
        let pre_reading = a.content_plain("きそん");
        let pre_node = a.ruby(pre_base, pre_reading);
        // `dst` keeps the allocator's store (the pre-existing entries).
        let dst_store = a.into_store();
        let pre_projection = project_node(&dst_store, pre_node);

        // Build a separate source store with a different node.
        let mut b = OwnedAllocator::new();
        let nb = b.content_plain("新規");
        let nr = b.content_plain("しんき");
        let new_node = b.ruby(nb, nr);
        let src = b.into_store();

        let mut dst = dst_store;
        let grafted = dst.graft_node(&src, new_node);

        // The pre-existing node's handles still resolve to the original bytes.
        assert_eq!(
            pre_projection,
            project_node(&dst, pre_node),
            "graft must not corrupt pre-existing entries",
        );
        // The grafted node resolves to the source bytes.
        assert_eq!(project_node(&src, new_node), project_node(&dst, grafted));
    }

    #[test]
    fn graft_dedups_shared_strings_across_two_source_nodes() {
        // Two source nodes share a string ("共有"). Grafting both into one dst
        // must resolve both to equal bytes (the dst interner dedups
        // transparently — they may or may not share a StrId).
        let mut a = OwnedAllocator::new();
        let b1 = a.content_plain("共有");
        let r1 = a.content_plain("いち");
        let n1 = a.ruby(b1, r1);
        let b2 = a.content_plain("共有");
        let r2 = a.content_plain("に");
        let n2 = a.ruby(b2, r2);
        let src = a.into_store();

        let mut dst = NodeStore::new();
        let g1 = dst.graft_node(&src, n1);
        let g2 = dst.graft_node(&src, n2);

        let NodeOwned::Ruby(rg1) = g1 else {
            panic!("expected ruby");
        };
        let NodeOwned::Ruby(rg2) = g2 else {
            panic!("expected ruby");
        };
        assert_eq!(
            dst.content_range_as_plain(rg1.base),
            Some("共有"),
            "first grafted base resolves to the shared bytes",
        );
        assert_eq!(
            dst.content_range_as_plain(rg2.base),
            Some("共有"),
            "second grafted base resolves to the shared bytes",
        );
        // Both nodes still project equal to their sources.
        assert_eq!(project_node(&src, n1), project_node(&dst, g1));
        assert_eq!(project_node(&src, n2), project_node(&dst, g2));
    }

    #[test]
    fn graft_menkuten_gaiji_preserves_canonical() {
        // Spot-check the structured MenKuTen arm survives the graft exactly.
        let mut a = OwnedAllocator::new();
        let g = a.make_gaiji("熙", Some("第3水準1-14-29"), false);
        let node = a.gaiji(g);
        let src = a.into_store();

        let mut dst = NodeStore::new();
        let NodeOwned::Gaiji(gg) = dst.graft_node(&src, node) else {
            panic!("expected gaiji");
        };
        assert_eq!(
            gg.canonical,
            GaijiCanonicalOwned::MenKuTen(MenKuTen {
                plane: 1,
                ku: 14,
                ten: 29,
            }),
        );
        assert_eq!(dst.resolve_str(gg.hint), "熙");
    }
}
