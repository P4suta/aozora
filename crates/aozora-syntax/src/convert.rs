//! Owned → borrowed AST conversion.
//!
//! Bridge from the legacy `Box<str>` / `Box<AozoraNode>` ownership
//! shape to the borrowed `&'src str` / `&'src AozoraNode<'src>` shape
//! parameterised by an [`Arena`]. Used by the new lex / Document API
//! during the multi-step Plan-B migration: the lex layer still
//! produces owned trees natively, then converts into the arena so the
//! public API surface is borrowed-only.
//!
//! Once Plan B finishes (owned types removed from the codebase),
//! this module disappears with them — the lex layer will allocate
//! directly into the arena.
//!
//! # Cost
//!
//! Every string field copies once into the arena (`alloc_str`); every
//! node payload copies once (`alloc`); slices flow through
//! `alloc_slice_copy` which is a single `memcpy`. For a 2 MB document
//! with ~2,700 nodes the conversion overhead is ~100 µs, well under
//! the 20 ms parse cost it bridges. Native arena emission is the
//! follow-up performance win (deferred to Plan B's later steps).

use crate as owned;
use crate::borrowed::{self, Arena};

/// Convert an owned [`AozoraNode`](crate::AozoraNode) into a borrowed
/// `borrowed::AozoraNode<'a>` allocated inside `arena`.
///
/// All string and child-node payloads are copied into the arena; the
/// returned tree is fully self-contained (no references to the input).
/// Drop the arena to free the entire converted tree in one step.
#[must_use]
pub fn to_borrowed<'a>(node: &owned::AozoraNode, arena: &'a Arena) -> borrowed::AozoraNode<'a> {
    use borrowed::AozoraNode as B;
    use owned::AozoraNode as O;
    match node {
        O::Ruby(r) => B::Ruby(arena.alloc(borrowed::Ruby {
            base: convert_content(&r.base, arena),
            reading: convert_content(&r.reading, arena),
            delim_explicit: r.delim_explicit,
        })),
        O::Bouten(b) => B::Bouten(arena.alloc(borrowed::Bouten {
            kind: b.kind,
            target: convert_content(&b.target, arena),
            position: b.position,
        })),
        O::TateChuYoko(t) => B::TateChuYoko(arena.alloc(borrowed::TateChuYoko {
            text: convert_content(&t.text, arena),
        })),
        O::Gaiji(g) => B::Gaiji(arena.alloc(convert_gaiji(g, arena))),
        O::Indent(i) => B::Indent(*i),
        O::AlignEnd(a) => B::AlignEnd(*a),
        O::Warichu(w) => B::Warichu(arena.alloc(borrowed::Warichu {
            upper: convert_content(&w.upper, arena),
            lower: convert_content(&w.lower, arena),
        })),
        O::Keigakomi(k) => B::Keigakomi(*k),
        O::PageBreak => B::PageBreak,
        O::SectionBreak(s) => B::SectionBreak(*s),
        O::AozoraHeading(h) => B::AozoraHeading(arena.alloc(borrowed::AozoraHeading {
            kind: h.kind,
            text: convert_content(&h.text, arena),
        })),
        O::HeadingHint(h) => B::HeadingHint(arena.alloc(borrowed::HeadingHint {
            level: h.level,
            target: arena.alloc_str(&h.target),
        })),
        O::Sashie(s) => B::Sashie(arena.alloc(borrowed::Sashie {
            file: arena.alloc_str(&s.file),
            caption: s.caption.as_ref().map(|c| convert_content(c, arena)),
        })),
        O::Kaeriten(k) => B::Kaeriten(arena.alloc(borrowed::Kaeriten {
            mark: arena.alloc_str(&k.mark),
        })),
        O::Annotation(a) => B::Annotation(arena.alloc(convert_annotation(a, arena))),
        O::DoubleRuby(d) => B::DoubleRuby(arena.alloc(borrowed::DoubleRuby {
            content: convert_content(&d.content, arena),
        })),
        O::Container(c) => B::Container(*c),
    }
}

/// Convert an owned [`Content`](crate::Content) into the borrowed
/// equivalent allocated inside `arena`.
#[must_use]
pub fn convert_content<'a>(c: &owned::Content, arena: &'a Arena) -> borrowed::Content<'a> {
    match c {
        owned::Content::Plain(s) => borrowed::Content::Plain(arena.alloc_str(s)),
        owned::Content::Segments(segs) => {
            // Build the borrowed segment list in a temporary Vec, then
            // bulk-copy via `alloc_slice_copy` so the slice ends up
            // contiguous in the arena. Borrowed segments are `Copy`,
            // satisfying the `T: Copy` bound on `alloc_slice_copy`.
            let new_segs: Vec<borrowed::Segment<'a>> =
                segs.iter().map(|s| convert_segment(s, arena)).collect();
            borrowed::Content::Segments(arena.alloc_slice_copy(&new_segs))
        }
    }
}

fn convert_segment<'a>(s: &owned::Segment, arena: &'a Arena) -> borrowed::Segment<'a> {
    match s {
        owned::Segment::Text(t) => borrowed::Segment::Text(arena.alloc_str(t)),
        owned::Segment::Gaiji(g) => borrowed::Segment::Gaiji(arena.alloc(convert_gaiji(g, arena))),
        owned::Segment::Annotation(a) => {
            borrowed::Segment::Annotation(arena.alloc(convert_annotation(a, arena)))
        }
    }
}

fn convert_gaiji<'a>(g: &owned::Gaiji, arena: &'a Arena) -> borrowed::Gaiji<'a> {
    borrowed::Gaiji {
        description: arena.alloc_str(&g.description),
        ucs: g.ucs,
        mencode: g.mencode.as_deref().map(|s| arena.alloc_str(s)),
    }
}

fn convert_annotation<'a>(a: &owned::Annotation, arena: &'a Arena) -> borrowed::Annotation<'a> {
    borrowed::Annotation {
        raw: arena.alloc_str(&a.raw),
        kind: a.kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AlignEnd, Annotation, AnnotationKind, AozoraHeading, AozoraHeadingKind, AozoraNode,
        Bouten, BoutenKind, BoutenPosition, Container, ContainerKind, Content, DoubleRuby, Gaiji,
        HeadingHint, Indent, Kaeriten, Keigakomi, Ruby, Sashie, SectionKind, Segment, TateChuYoko,
        Warichu,
    };

    /// Spot-check that every variant survives the round-trip with
    /// matching xml-node-name + structural shape. Pinning xml names
    /// is the cheapest way to verify the variant landed correctly
    /// without a full deep-equal helper.
    #[test]
    fn every_variant_round_trips_xml_node_name() {
        let arena = Arena::new();
        let samples: Vec<AozoraNode> = vec![
            AozoraNode::Ruby(Ruby {
                base: "a".into(),
                reading: "b".into(),
                delim_explicit: true,
            }),
            AozoraNode::Bouten(Bouten {
                kind: BoutenKind::Goma,
                target: "x".into(),
                position: BoutenPosition::Right,
            }),
            AozoraNode::TateChuYoko(TateChuYoko { text: "12".into() }),
            AozoraNode::Gaiji(Gaiji {
                description: "desc".into(),
                ucs: Some('A'),
                mencode: Some("1-2-3".into()),
            }),
            AozoraNode::Indent(Indent { amount: 3 }),
            AozoraNode::AlignEnd(AlignEnd { offset: 2 }),
            AozoraNode::Warichu(Warichu {
                upper: "u".into(),
                lower: "l".into(),
            }),
            AozoraNode::Keigakomi(Keigakomi),
            AozoraNode::PageBreak,
            AozoraNode::SectionBreak(SectionKind::Choho),
            AozoraNode::AozoraHeading(AozoraHeading {
                kind: AozoraHeadingKind::Window,
                text: "h".into(),
            }),
            AozoraNode::HeadingHint(HeadingHint {
                level: 1,
                target: "t".into(),
            }),
            AozoraNode::Sashie(Sashie {
                file: "f.png".into(),
                caption: Some("c".into()),
            }),
            AozoraNode::Kaeriten(Kaeriten { mark: "一".into() }),
            AozoraNode::Annotation(Annotation {
                raw: "r".into(),
                kind: AnnotationKind::Unknown,
            }),
            AozoraNode::DoubleRuby(DoubleRuby {
                content: "d".into(),
            }),
            AozoraNode::Container(Container {
                kind: ContainerKind::Indent { amount: 1 },
            }),
        ];
        for owned in &samples {
            let borrowed = to_borrowed(owned, &arena);
            assert_eq!(
                owned.xml_node_name(),
                borrowed.xml_node_name(),
                "xml node name diverged for variant {owned:?}"
            );
            assert_eq!(owned.is_block(), borrowed.is_block());
            assert_eq!(owned.contains_inlines(), borrowed.contains_inlines());
        }
    }

    #[test]
    fn ruby_fields_copied_into_arena() {
        let arena = Arena::new();
        let owned = AozoraNode::Ruby(Ruby {
            base: "青梅".into(),
            reading: "おうめ".into(),
            delim_explicit: true,
        });
        match to_borrowed(&owned, &arena) {
            borrowed::AozoraNode::Ruby(r) => {
                assert_eq!(r.base.as_plain(), Some("青梅"));
                assert_eq!(r.reading.as_plain(), Some("おうめ"));
                assert!(r.delim_explicit);
            }
            other => panic!("expected Ruby, got {other:?}"),
        }
    }

    #[test]
    fn gaiji_optional_mencode_handled() {
        let arena = Arena::new();
        let with = AozoraNode::Gaiji(Gaiji {
            description: "X".into(),
            ucs: Some('𠀋'),
            mencode: Some("第3水準1-85-54".into()),
        });
        let without = AozoraNode::Gaiji(Gaiji {
            description: "Y".into(),
            ucs: None,
            mencode: None,
        });
        match to_borrowed(&with, &arena) {
            borrowed::AozoraNode::Gaiji(g) => {
                assert_eq!(g.description, "X");
                assert_eq!(g.ucs, Some('𠀋'));
                assert_eq!(g.mencode, Some("第3水準1-85-54"));
            }
            other => panic!("expected Gaiji, got {other:?}"),
        }
        match to_borrowed(&without, &arena) {
            borrowed::AozoraNode::Gaiji(g) => {
                assert_eq!(g.description, "Y");
                assert_eq!(g.ucs, None);
                assert_eq!(g.mencode, None);
            }
            other => panic!("expected Gaiji, got {other:?}"),
        }
    }

    #[test]
    fn content_plain_round_trips() {
        let arena = Arena::new();
        let c = Content::from("hello");
        let b = convert_content(&c, &arena);
        assert_eq!(b.as_plain(), Some("hello"));
    }

    #[test]
    fn content_segments_preserve_order_and_kinds() {
        let arena = Arena::new();
        let c = Content::from_segments(vec![
            Segment::Text("before ".into()),
            Segment::Gaiji(Gaiji {
                description: "X".into(),
                ucs: None,
                mencode: None,
            }),
            Segment::Text(" after".into()),
            Segment::Annotation(Annotation {
                raw: "［＃X］".into(),
                kind: AnnotationKind::Unknown,
            }),
        ]);
        let b = convert_content(&c, &arena);
        let collected: Vec<_> = b.iter().collect();
        assert_eq!(collected.len(), 4);
        match collected[0] {
            borrowed::Segment::Text(t) => assert_eq!(t, "before "),
            other => panic!("first segment kind wrong: {other:?}"),
        }
        match collected[1] {
            borrowed::Segment::Gaiji(g) => assert_eq!(g.description, "X"),
            other => panic!("second segment kind wrong: {other:?}"),
        }
        match collected[2] {
            borrowed::Segment::Text(t) => assert_eq!(t, " after"),
            other => panic!("third segment kind wrong: {other:?}"),
        }
        match collected[3] {
            borrowed::Segment::Annotation(a) => assert_eq!(a.raw, "［＃X］"),
            other => panic!("fourth segment kind wrong: {other:?}"),
        }
    }

    #[test]
    fn empty_segments_preserved() {
        let arena = Arena::new();
        let c = Content::default();
        let b = convert_content(&c, &arena);
        assert_eq!(b.iter().count(), 0);
    }

    #[test]
    fn sashie_optional_caption_handled() {
        let arena = Arena::new();
        let with = AozoraNode::Sashie(Sashie {
            file: "x.png".into(),
            caption: Some("見出し".into()),
        });
        let without = AozoraNode::Sashie(Sashie {
            file: "y.png".into(),
            caption: None,
        });
        match to_borrowed(&with, &arena) {
            borrowed::AozoraNode::Sashie(s) => {
                assert_eq!(s.file, "x.png");
                assert_eq!(s.caption.and_then(borrowed::Content::as_plain), Some("見出し"));
            }
            other => panic!("expected Sashie, got {other:?}"),
        }
        match to_borrowed(&without, &arena) {
            borrowed::AozoraNode::Sashie(s) => {
                assert_eq!(s.file, "y.png");
                assert!(s.caption.is_none());
            }
            other => panic!("expected Sashie, got {other:?}"),
        }
    }

    #[test]
    fn many_conversions_share_one_arena() {
        let arena = Arena::new();
        // Pin that the arena holds independent copies of every input —
        // mutation/drop of the owned source after conversion must not
        // invalidate any borrowed reference.
        let owned: Vec<AozoraNode> = (0..50)
            .map(|i| {
                AozoraNode::Kaeriten(Kaeriten {
                    mark: format!("mark{i}").into(),
                })
            })
            .collect();
        let borrowed: Vec<borrowed::AozoraNode<'_>> =
            owned.iter().map(|n| to_borrowed(n, &arena)).collect();
        // Drop the owned source vector — borrowed must remain valid.
        drop(owned);
        for (i, b) in borrowed.iter().enumerate() {
            match b {
                borrowed::AozoraNode::Kaeriten(k) => assert_eq!(k.mark, format!("mark{i}")),
                _ => panic!("expected Kaeriten"),
            }
        }
    }
}
