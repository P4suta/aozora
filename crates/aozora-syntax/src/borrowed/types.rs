//! Borrowed AST types parameterised by the source/arena lifetime
//! `'src`.
//!
//! Mirror of the legacy owned types in the parent module, with every
//! `Box<str>` replaced by `&'src str` and every `Box<X>` replaced by
//! `&'src X<'src>`. All types are `Copy` because they carry only
//! `Copy` payloads (references, primitives, `Copy` enums); this lets
//! visitors and renderers walk the tree without `&mut` ceremony.
//!
//! Variants that hold no string content (`Indent`, `AlignEnd`,
//! `Keigakomi`, `SectionKind`, `Container`) are re-exported from the
//! owned module unchanged — they are already `Copy + 'static`.

use core::slice;

use aozora_encoding::gaiji::Resolved;

use crate::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, Container, Indent,
    Keigakomi, SectionKind,
};

// ----------------------------------------------------------------------
// Top-level node enum
// ----------------------------------------------------------------------

/// Every Aozora-specific AST node, in borrowed form.
///
/// `'src` is the lifetime of the arena (and of source-text slices the
/// arena does not own). Mirrors the variant set of the legacy owned
/// [`aozora_syntax owned API (no longer present)`] type 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AozoraNode<'src> {
    /// Ruby (furigana). See [`Ruby`].
    Ruby(&'src Ruby<'src>),
    /// Emphasis dots / sidelines. See [`Bouten`].
    Bouten(&'src Bouten<'src>),
    /// Tate-chu-yoko (horizontal embedding inside vertical text).
    TateChuYoko(&'src TateChuYoko<'src>),
    /// Out-of-character-range glyph reference. See [`Gaiji`].
    Gaiji(&'src Gaiji<'src>),
    /// Indentation marker. Carries no string content; uses the legacy
    /// owned [`Indent`] type unchanged.
    Indent(Indent),
    /// End-aligned text marker.
    AlignEnd(AlignEnd),
    /// Warichu (split annotation). See [`Warichu`].
    Warichu(&'src Warichu<'src>),
    /// Keigakomi (boxed text marker, no fields).
    Keigakomi(Keigakomi),
    /// Page break (`［＃改ページ］`).
    PageBreak,
    /// Section break — `［＃改丁／改段／改見開き］`.
    SectionBreak(SectionKind),
    /// Aozora heading (窓見出し / 副見出し). See [`AozoraHeading`].
    AozoraHeading(&'src AozoraHeading<'src>),
    /// Forward-reference heading hint (`［＃「X」は大見出し］`).
    HeadingHint(&'src HeadingHint<'src>),
    /// Illustration (`［＃挿絵］`).
    Sashie(&'src Sashie<'src>),
    /// Chinese-reading-order mark (`返り点`).
    Kaeriten(&'src Kaeriten<'src>),
    /// Generic annotation when no more specific recogniser matched.
    Annotation(&'src Annotation<'src>),
    /// `《《…》》` double-bracket bouten. See [`DoubleRuby`].
    DoubleRuby(&'src DoubleRuby<'src>),
    /// Paired-container open (`［＃ここから字下げ］` etc.).
    Container(Container),
}

// ----------------------------------------------------------------------
// Content + Segment
// ----------------------------------------------------------------------

/// Body content for nodes whose textual payload may carry nested
/// Aozora constructs.
///
/// See [`crate::Content`] for the design rationale — this is the
/// borrowed mirror, with the same `Plain` fast path and `Segments`
/// general case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Content<'src> {
    /// Plain text without embedded Aozora constructs. Borrows directly
    /// from source or from arena.
    Plain(&'src str),
    /// Mixed text plus nested Aozora constructs. Slice owned by arena.
    Segments(&'src [Segment<'src>]),
}

impl<'src> Content<'src> {
    /// Empty content. By convention `Segments(&[])` rather than
    /// `Plain("")` — same canonicalisation as the owned counterpart.
    pub const EMPTY: Self = Self::Segments(&[]);

    /// Fast-path accessor: returns the text if this is a `Plain` run,
    /// `None` if mixed. Renderers use this to skip the segment loop on
    /// the 99%+ majority case.
    #[must_use]
    pub fn as_plain(self) -> Option<&'src str> {
        match self {
            Self::Plain(s) => Some(s),
            Self::Segments(_) => None,
        }
    }

    /// Iterate segments left-to-right. `Plain` yields a single text
    /// segment; `Segments` yields each entry in order.
    #[must_use]
    pub fn iter(self) -> ContentIter<'src> {
        match self {
            Self::Plain(s) => ContentIter::Plain(Some(s)),
            Self::Segments(segs) => ContentIter::Segments(segs.iter()),
        }
    }
}

impl Default for Content<'_> {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// One element of a [`Content::Segments`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Segment<'src> {
    Text(&'src str),
    Gaiji(&'src Gaiji<'src>),
    Annotation(&'src Annotation<'src>),
}

/// Iterator over a [`Content`]'s logical segments, returned by
/// [`Content::iter`]. The `Plain` branch yields a single synthesised
/// `Text` segment so renderers can write a uniform loop.
#[derive(Debug, Clone)]
pub enum ContentIter<'src> {
    Plain(Option<&'src str>),
    Segments(slice::Iter<'src, Segment<'src>>),
}

impl<'src> Iterator for ContentIter<'src> {
    type Item = Segment<'src>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Plain(opt) => opt.take().map(Segment::Text),
            Self::Segments(it) => it.next().copied(),
        }
    }
}

impl<'src> IntoIterator for Content<'src> {
    type Item = Segment<'src>;
    type IntoIter = ContentIter<'src>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ----------------------------------------------------------------------
// Per-variant payload structs
// ----------------------------------------------------------------------

/// Ruby (furigana). See [`crate::Ruby`] for field semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ruby<'src> {
    pub base: Content<'src>,
    pub reading: Content<'src>,
    pub delim_explicit: bool,
}

/// Emphasis dots / sidelines. See [`crate::Bouten`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bouten<'src> {
    pub kind: BoutenKind,
    pub target: Content<'src>,
    pub position: BoutenPosition,
}

/// Tate-chu-yoko (horizontal embedding). See [`crate::TateChuYoko`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TateChuYoko<'src> {
    pub text: Content<'src>,
}

/// Gaiji (out-of-character-range glyph). See [`crate::Gaiji`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gaiji<'src> {
    /// Free-form description from the source (e.g. "木＋吶のつくり").
    pub description: &'src str,
    /// Resolved Unicode value — either a single scalar or a static
    /// combining sequence (the 25 plane-1 cells like か゚, IPA tone
    /// marks). `None` when the resolver could not match any path.
    /// `Resolved` is `Copy`, so the surrounding `Content`-tree's
    /// `Copy` chain is preserved.
    pub ucs: Option<Resolved>,
    /// Raw mencode reference (e.g. "第3水準1-85-54", "U+XXXX page-line").
    pub mencode: Option<&'src str>,
}

/// Warichu (split annotation). See [`crate::Warichu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Warichu<'src> {
    pub upper: Content<'src>,
    pub lower: Content<'src>,
}

/// Aozora heading (窓見出し / 副見出し). See [`crate::AozoraHeading`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AozoraHeading<'src> {
    pub kind: AozoraHeadingKind,
    pub text: Content<'src>,
}

/// Forward-reference heading hint. See [`crate::HeadingHint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingHint<'src> {
    pub level: u8,
    pub target: &'src str,
}

/// Illustration metadata. See [`crate::Sashie`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sashie<'src> {
    pub file: &'src str,
    pub caption: Option<Content<'src>>,
}

/// Generic annotation. See [`crate::Annotation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Annotation<'src> {
    pub raw: &'src str,
    pub kind: AnnotationKind,
}

/// Chinese-reading-order mark (`返り点`). See [`crate::Kaeriten`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kaeriten<'src> {
    pub mark: &'src str,
}

/// Double angle-bracket payload. See [`crate::DoubleRuby`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoubleRuby<'src> {
    pub content: Content<'src>,
}

/// [`AozoraNode`] classifier methods. Mirror the inherent methods on
/// the legacy owned [`aozora_syntax owned API (no longer present)`] 1:1 so a renderer compiled
/// against either AST takes the same code path and the same XML
/// snapshot string.
impl AozoraNode<'_> {
    /// True iff this node occupies a paragraph position in the
    /// document tree (and therefore shouldn't be wrapped inside a
    /// surrounding text run by the renderer).
    #[must_use]
    pub const fn is_block(&self) -> bool {
        matches!(
            self,
            Self::Indent(_)
                | Self::AlignEnd(_)
                | Self::Warichu(_)
                | Self::Keigakomi(_)
                | Self::PageBreak
                | Self::SectionBreak(_)
                | Self::AozoraHeading(_)
                | Self::Sashie(_)
                | Self::Container(_)
        )
    }

    /// Whether children of this node (if any) are inline content.
    /// Block variants that wrap an indented run of paragraphs answer
    /// `true`; leaf blocks answer `false`. `Container` is the
    /// paired-container wrapper — its children are block elements,
    /// so it answers `false` here.
    #[must_use]
    pub const fn contains_inlines(&self) -> bool {
        matches!(
            self,
            Self::AozoraHeading(_)
                | Self::AlignEnd(_)
                | Self::Warichu(_)
                | Self::Keigakomi(_)
                | Self::Indent(_)
        )
    }

    /// Stable XML/element-style node name used by HTML / serialiser /
    /// snapshot tests. Identical to the legacy
    /// [`aozora_syntax owned API (no longer present)::xml_node_name`] return values to keep
    /// snapshot tests cross-compatible.
    #[must_use]
    pub const fn xml_node_name(&self) -> &'static str {
        match self {
            Self::Ruby(_) => "aozora_ruby",
            Self::Bouten(_) => "aozora_bouten",
            Self::TateChuYoko(_) => "aozora_tcy",
            Self::Gaiji(_) => "aozora_gaiji",
            Self::Indent(_) => "aozora_indent",
            Self::AlignEnd(_) => "aozora_align_end",
            Self::Warichu(_) => "aozora_warichu",
            Self::Keigakomi(_) => "aozora_keigakomi",
            Self::PageBreak => "aozora_page_break",
            Self::SectionBreak(_) => "aozora_section_break",
            Self::AozoraHeading(_) => "aozora_heading",
            Self::HeadingHint(_) => "aozora_heading_hint",
            Self::Sashie(_) => "aozora_sashie",
            Self::Kaeriten(_) => "aozora_kaeriten",
            Self::Annotation(_) => "aozora_annotation",
            Self::DoubleRuby(_) => "aozora_double_ruby",
            Self::Container(_) => "aozora_container",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aozora_node_is_copy() {
        // Pin the Copy bound — if a future variant ever holds a
        // non-Copy payload, the visitor pattern falls apart and this
        // test fails to compile.
        fn assert_copy<T: Copy>() {}
        assert_copy::<AozoraNode<'static>>();
        assert_copy::<Content<'static>>();
        assert_copy::<Ruby<'static>>();
        assert_copy::<Bouten<'static>>();
        assert_copy::<Gaiji<'static>>();
    }

    #[test]
    fn content_plain_as_plain_returns_some() {
        let c: Content<'static> = Content::Plain("hello");
        assert_eq!(c.as_plain(), Some("hello"));
    }

    #[test]
    fn content_segments_as_plain_returns_none() {
        let segs: &'static [Segment<'static>] = &[Segment::Text("a"), Segment::Text("b")];
        let c = Content::Segments(segs);
        assert_eq!(c.as_plain(), None);
    }

    #[test]
    fn content_default_is_empty_segments() {
        let c: Content<'static> = Content::default();
        assert!(matches!(c, Content::Segments(s) if s.is_empty()));
    }

    #[test]
    fn content_iter_over_plain_yields_one_text_segment() {
        let c: Content<'static> = Content::Plain("x");
        let collected: Vec<Segment<'static>> = c.iter().collect();
        assert_eq!(collected.len(), 1);
        assert!(matches!(collected[0], Segment::Text("x")));
    }

    #[test]
    fn content_iter_over_empty_segments_yields_nothing() {
        let c: Content<'static> = Content::EMPTY;
        assert_eq!(c.iter().count(), 0);
    }

    #[test]
    fn content_iter_over_segments_preserves_order() {
        let segs: &'static [Segment<'static>] =
            &[Segment::Text("a"), Segment::Text("b"), Segment::Text("c")];
        let collected: Vec<Segment<'static>> = Content::Segments(segs).iter().collect();
        assert_eq!(collected.len(), 3);
        for (i, seg) in collected.iter().enumerate() {
            match seg {
                Segment::Text(t) => {
                    assert_eq!(*t, ["a", "b", "c"][i]);
                }
                _ => panic!("expected Text segment"),
            }
        }
    }

    #[test]
    fn into_iter_works_via_for_loop() {
        let c: Content<'static> = Content::Plain("hi");
        let mut count = 0;
        for seg in c {
            count += 1;
            assert!(matches!(seg, Segment::Text("hi")));
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn aozora_node_xml_names_are_unique_per_variant() {
        // Spot-check a couple of variants — exhaustive coverage lives
        // in the legacy AozoraNode test suite. Our concern here is
        // that the borrowed mirror returns the SAME strings.
        let kaeriten = Kaeriten { mark: "x" };
        let n = AozoraNode::Kaeriten(&kaeriten);
        assert_eq!(n.xml_node_name(), "aozora_kaeriten");
        assert!(!n.contains_inlines());

        assert!(AozoraNode::PageBreak.is_block());
        assert_eq!(AozoraNode::PageBreak.xml_node_name(), "aozora_page_break");
    }

    #[test]
    fn block_variants_report_block() {
        assert!(AozoraNode::Indent(Indent { amount: 2 }).is_block());
        assert!(AozoraNode::SectionBreak(SectionKind::Choho).is_block());
    }

    #[test]
    fn inline_variants_are_not_block() {
        let ruby = Ruby {
            base: Content::Plain("x"),
            reading: Content::Plain("x"),
            delim_explicit: false,
        };
        assert!(!AozoraNode::Ruby(&ruby).is_block());

        let kaeriten = Kaeriten { mark: "x" };
        assert!(!AozoraNode::Kaeriten(&kaeriten).is_block());
    }

    #[test]
    fn ruby_carries_both_base_and_reading() {
        let r = Ruby {
            base: Content::Plain("青梅"),
            reading: Content::Plain("おうめ"),
            delim_explicit: true,
        };
        assert_eq!(r.base.as_plain(), Some("青梅"));
        assert_eq!(r.reading.as_plain(), Some("おうめ"));
        assert!(r.delim_explicit);
    }

    #[test]
    fn gaiji_holds_optional_ucs_and_mencode() {
        use aozora_encoding::gaiji::Resolved;
        let g = Gaiji {
            description: "木＋吶のつくり",
            ucs: Some(Resolved::Char('𠀋')),
            mencode: Some("第3水準1-85-54"),
        };
        assert_eq!(g.description, "木＋吶のつくり");
        assert_eq!(g.ucs, Some(Resolved::Char('𠀋')));
        assert_eq!(g.mencode, Some("第3水準1-85-54"));
    }

    #[test]
    fn gaiji_can_carry_combining_sequence_resolution() {
        // The 25 plane-1 combining-sequence cells (か゚, IPA tone marks,
        // accented Latin) need to round-trip through the Gaiji
        // structure intact. `Resolved::Multi` carries them; without
        // this variant the parser would lose precision on the
        // ~0.6% gaiji corpus that sits on these cells.
        use aozora_encoding::gaiji::Resolved;
        let g = Gaiji {
            description: "か゚",
            ucs: Some(Resolved::Multi("\u{304B}\u{309A}")),
            mencode: Some("第3水準1-4-87"),
        };
        assert_eq!(g.ucs, Some(Resolved::Multi("か゚")));
    }
}
