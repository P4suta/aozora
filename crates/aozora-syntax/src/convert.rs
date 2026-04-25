//! Owned → borrowed AST conversion.
//!
//! Bridge from the legacy `Box<str>` / `Box<AozoraNode>` ownership
//! shape to the borrowed `&'src str` / `&'src AozoraNode<'src>` shape
//! parameterised by an [`Arena`]. Used by the new lex / Document API
//! during the multi-step Plan-B migration: the lex layer still
//! produces owned trees natively, then converts into the arena so the
//! public API surface is borrowed-only.
//!
//! # String allocation strategy
//!
//! All string fields flow through a [`StringPool`] trait so callers
//! can choose between two strategies:
//!
//! - [`DirectAlloc`] — every string copies into the arena once
//!   (`arena.alloc_str`). Use when the input has no string-content
//!   repetition or when conversion latency is paramount.
//! - [`Interned`] — strings flow through an [`Interner`] (Innovation
//!   I-7) so byte-equal content shares a single arena allocation.
//!   Use for real-world Aozora corpora, where short ruby readings
//!   (`の`, `に`, `を`, …), kaeriten marks, and container labels
//!   repeat dozens to hundreds of times per document. The interner
//!   has its own diagnostic counters so callers can measure dedup
//!   rates and average probe length.
//!
//! Both strategies preserve byte-identical output (the same final
//! `&str` content); only the underlying memory layout differs.
//!
//! # Cost
//!
//! Direct allocation is O(|s|) per string. The interner adds a
//! constant-factor hash + probe (avg < 2 probes per call at typical
//! load) but eliminates duplicate-byte allocations entirely on
//! repeated content. Empirically Aozora corpora dedup to ~30–50%
//! of the naive size.

use crate as owned;
use crate::borrowed::{self, Arena, Interner};

/// Strategy for placing a `&str` into the arena.
///
/// Implemented by both [`DirectAlloc`] (one fresh `arena.alloc_str`
/// per call) and [`Interned`] (deduplicates byte-equal content via
/// an [`Interner`]). The converter is generic over this trait so the
/// choice of strategy is per-call; the per-node walker code is
/// shared.
pub trait StringPool<'a> {
    /// Place `s` into the underlying arena and return a stable
    /// `&'a str` reference. Implementations differ on whether
    /// repeated calls with byte-equal `s` return the same pointer.
    fn place(&mut self, s: &str) -> &'a str;
}

/// One `arena.alloc_str` per call. No deduplication. Use when the
/// input is known not to repeat strings, or when the per-call
/// constant of an interner is not worth paying.
#[derive(Debug)]
pub struct DirectAlloc<'a> {
    arena: &'a Arena,
}

impl<'a> DirectAlloc<'a> {
    /// Wrap an arena reference as a no-dedup string pool.
    #[must_use]
    pub const fn new(arena: &'a Arena) -> Self {
        Self { arena }
    }
}

impl<'a> StringPool<'a> for DirectAlloc<'a> {
    fn place(&mut self, s: &str) -> &'a str {
        self.arena.alloc_str(s)
    }
}

impl<'a> StringPool<'a> for Interner<'a> {
    fn place(&mut self, s: &str) -> &'a str {
        self.intern(s)
    }
}

/// Convert an owned [`AozoraNode`](crate::AozoraNode) into a borrowed
/// `borrowed::AozoraNode<'a>` allocated inside `arena`.
///
/// Convenience wrapper around [`to_borrowed_with`] that uses
/// [`DirectAlloc`] (no string deduplication). For real-corpus
/// throughput prefer [`to_borrowed_with`] with an [`Interner`].
#[must_use]
pub fn to_borrowed<'a>(
    node: &owned::AozoraNode,
    arena: &'a Arena,
) -> borrowed::AozoraNode<'a> {
    let mut pool = DirectAlloc::new(arena);
    to_borrowed_with(node, arena, &mut pool)
}

/// Convert an owned `AozoraNode` into a borrowed equivalent, routing
/// every string field through `pool`.
///
/// `pool` is an arena-bound [`StringPool`] strategy; pass
/// [`DirectAlloc`] for one-alloc-per-string, or [`Interner`] for
/// hash-table-based deduplication of byte-equal content.
#[must_use]
pub fn to_borrowed_with<'a, P: StringPool<'a>>(
    node: &owned::AozoraNode,
    arena: &'a Arena,
    pool: &mut P,
) -> borrowed::AozoraNode<'a> {
    use borrowed::AozoraNode as B;
    use owned::AozoraNode as O;
    match node {
        O::Ruby(r) => B::Ruby(arena.alloc(borrowed::Ruby {
            base: convert_content_with(&r.base, arena, pool),
            reading: convert_content_with(&r.reading, arena, pool),
            delim_explicit: r.delim_explicit,
        })),
        O::Bouten(b) => B::Bouten(arena.alloc(borrowed::Bouten {
            kind: b.kind,
            target: convert_content_with(&b.target, arena, pool),
            position: b.position,
        })),
        O::TateChuYoko(t) => B::TateChuYoko(arena.alloc(borrowed::TateChuYoko {
            text: convert_content_with(&t.text, arena, pool),
        })),
        O::Gaiji(g) => B::Gaiji(arena.alloc(convert_gaiji(g, pool))),
        O::Indent(i) => B::Indent(*i),
        O::AlignEnd(a) => B::AlignEnd(*a),
        O::Warichu(w) => B::Warichu(arena.alloc(borrowed::Warichu {
            upper: convert_content_with(&w.upper, arena, pool),
            lower: convert_content_with(&w.lower, arena, pool),
        })),
        O::Keigakomi(k) => B::Keigakomi(*k),
        O::PageBreak => B::PageBreak,
        O::SectionBreak(s) => B::SectionBreak(*s),
        O::AozoraHeading(h) => B::AozoraHeading(arena.alloc(borrowed::AozoraHeading {
            kind: h.kind,
            text: convert_content_with(&h.text, arena, pool),
        })),
        O::HeadingHint(h) => B::HeadingHint(arena.alloc(borrowed::HeadingHint {
            level: h.level,
            target: pool.place(&h.target),
        })),
        O::Sashie(s) => B::Sashie(arena.alloc(borrowed::Sashie {
            file: pool.place(&s.file),
            caption: s.caption.as_ref().map(|c| convert_content_with(c, arena, pool)),
        })),
        O::Kaeriten(k) => B::Kaeriten(arena.alloc(borrowed::Kaeriten {
            mark: pool.place(&k.mark),
        })),
        O::Annotation(a) => B::Annotation(arena.alloc(convert_annotation(a, pool))),
        O::DoubleRuby(d) => B::DoubleRuby(arena.alloc(borrowed::DoubleRuby {
            content: convert_content_with(&d.content, arena, pool),
        })),
        O::Container(c) => B::Container(*c),
    }
}

/// Convert an owned [`Content`](crate::Content) into the borrowed
/// equivalent. Convenience wrapper that uses [`DirectAlloc`].
#[must_use]
pub fn convert_content<'a>(c: &owned::Content, arena: &'a Arena) -> borrowed::Content<'a> {
    let mut pool = DirectAlloc::new(arena);
    convert_content_with(c, arena, &mut pool)
}

/// Convert an owned [`Content`](crate::Content) into the borrowed
/// equivalent, routing strings through `pool`. The segment list
/// itself is copied via `alloc_slice_copy` (single arena memcpy).
#[must_use]
pub fn convert_content_with<'a, P: StringPool<'a>>(
    c: &owned::Content,
    arena: &'a Arena,
    pool: &mut P,
) -> borrowed::Content<'a> {
    match c {
        owned::Content::Plain(s) => borrowed::Content::Plain(pool.place(s)),
        owned::Content::Segments(segs) => {
            let new_segs: Vec<borrowed::Segment<'a>> =
                segs.iter().map(|s| convert_segment(s, arena, pool)).collect();
            borrowed::Content::Segments(arena.alloc_slice_copy(&new_segs))
        }
    }
}

fn convert_segment<'a, P: StringPool<'a>>(
    s: &owned::Segment,
    arena: &'a Arena,
    pool: &mut P,
) -> borrowed::Segment<'a> {
    match s {
        owned::Segment::Text(t) => borrowed::Segment::Text(pool.place(t)),
        owned::Segment::Gaiji(g) => borrowed::Segment::Gaiji(arena.alloc(convert_gaiji(g, pool))),
        owned::Segment::Annotation(a) => {
            borrowed::Segment::Annotation(arena.alloc(convert_annotation(a, pool)))
        }
    }
}

fn convert_gaiji<'a, P: StringPool<'a>>(
    g: &owned::Gaiji,
    pool: &mut P,
) -> borrowed::Gaiji<'a> {
    borrowed::Gaiji {
        description: pool.place(&g.description),
        ucs: g.ucs,
        mencode: g.mencode.as_deref().map(|s| pool.place(s)),
    }
}

fn convert_annotation<'a, P: StringPool<'a>>(
    a: &owned::Annotation,
    pool: &mut P,
) -> borrowed::Annotation<'a> {
    borrowed::Annotation {
        raw: pool.place(&a.raw),
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
