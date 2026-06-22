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
//! `Framed`, `SectionKind`, `Container`) are re-exported from the
//! owned module unchanged — they are already `Copy + 'static`.

use core::slice;

use aozora_encoding::gaiji::Resolved;

use crate::{
    AlignEnd, BoutenKind, BoutenPosition, Center, Container, DirectiveKind, EmphasisKind, Framed,
    HeadingKind, HeadingStyle, Indent, MarginNoteKind, RubySide, SectionKind,
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
pub enum Node<'src> {
    /// Ruby (furigana). See [`Ruby`].
    Ruby(&'src Ruby<'src>),
    /// Emphasis dots / sidelines. See [`Bouten`].
    Bouten(&'src Bouten<'src>),
    /// Tate-chu-yoko (horizontal embedding inside vertical text).
    CombineUpright(&'src CombineUpright<'src>),
    /// Out-of-character-range glyph reference. See [`Gaiji`].
    Gaiji(&'src Gaiji<'src>),
    /// Indentation marker. Carries no string content; uses the legacy
    /// owned [`Indent`] type unchanged.
    Indent(Indent),
    /// End-aligned text marker.
    AlignEnd(AlignEnd),
    /// Centring marker (`ページの左右中央` / `中央揃え`). See [`Center`].
    Center(Center),
    /// Warichu (split annotation). See [`Warichu`].
    Warichu(&'src Warichu<'src>),
    /// Framed (boxed text marker, no fields).
    Framed(Framed),
    /// Page break (`［＃改ページ］`).
    PageBreak,
    /// Section break — `［＃改丁／改段／改見開き］`.
    SectionBreak(SectionKind),
    /// Aozora heading (窓見出し / 副見出し). See [`Heading`].
    Heading(&'src Heading<'src>),
    /// Forward-reference heading hint (`［＃「X」は大見出し］`).
    HeadingHint(&'src HeadingHint<'src>),
    /// Illustration (`［＃挿絵］`).
    Illustration(&'src Illustration<'src>),
    /// Chinese-reading-order mark (`返り点`).
    Kaeriten(&'src Kaeriten<'src>),
    /// Generic annotation when no more specific recogniser matched.
    Directive(&'src Directive<'src>),
    /// `≪…≫` double-angle quotation (displays as `《…》`). See [`AngleQuote`].
    AngleQuote(&'src AngleQuote<'src>),
    /// Bold / italic emphasis (`X［＃「X」は太字／斜体］`). See [`Emphasis`].
    Emphasis(&'src Emphasis<'src>),
    /// Left-side annotation (注記). See [`MarginNote`].
    MarginNote(&'src MarginNote<'src>),
    /// Paired-container open (`［＃ここから字下げ］` etc.).
    Container(Container),
}

// ----------------------------------------------------------------------
// Content + Segment
// ----------------------------------------------------------------------

/// Body content for nodes whose textual payload may carry nested
/// Aozora constructs.
///
/// Two-tier representation: `Plain` is the fast path for plain
/// strings; `Segments` is the general case for content carrying
/// nested aozora nodes.
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
    /// A plain-text run between nested constructs.
    Text(&'src str),
    /// A nested 外字 reference. See [`Gaiji`].
    Gaiji(&'src Gaiji<'src>),
    /// A nested generic annotation. See [`Directive`].
    Directive(&'src Directive<'src>),
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

/// Ruby (furigana).
///
/// `base` and `reading` are [`super::NonEmpty`] — the classify stage
/// only emits a Ruby node once both have content. The wrapper makes the
/// invariant a build-time fact so renderers never see an empty
/// payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ruby<'src> {
    /// The base text the reading annotates.
    pub base: super::NonEmpty<Content<'src>>,
    /// The furigana reading shown over (or beside) the base.
    pub reading: super::NonEmpty<Content<'src>>,
    /// `true` when the base was delimited by an explicit `｜` (`｜base《…》`);
    /// `false` when the base was inferred by consuming a trailing kanji run
    /// (`漢字《…》`). Preserved so `serialize` reproduces the original
    /// delimiter and the parse∘serialize fixed point holds.
    pub delim_explicit: bool,
    /// Which side the reading sits on. `Right` for the `｜《》` / implicit
    /// forms; `Left` for the `［＃「X」の左に「Y」のルビ］` saidoku building block.
    pub side: RubySide,
}

/// Side annotation — 注記 or 傍記 (selected by [`kind`](Self::kind)).
///
/// A `note` attached to a preceding `base` run via a forward reference.
/// Like a left-side ruby in placement, but a *note* rather than a phonetic
/// reading, so it is a distinct node that round-trips to its own keyword
/// (not `のルビ`):
/// - [`MarginNoteKind::Gloss`] — `［＃「base」の左に「note」の注記］`, an
///   editorial gloss.
/// - [`MarginNoteKind::Marginal`] — `［＃「base」に「note」の傍記］`, a redaction
///   marker (典型的に ×) written beside `base`.
///
/// Both `base` and `note` are [`super::NonEmpty`]: the classify stage emits
/// the node only once both have content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarginNote<'src> {
    /// 注記 vs 傍記 — preserved for faithful round-trip; both render alike.
    pub kind: MarginNoteKind,
    /// The preceding run the note is attached to.
    pub base: super::NonEmpty<Content<'src>>,
    /// The gloss / redaction text shown beside `base`.
    pub note: super::NonEmpty<Content<'src>>,
}

/// Emphasis dots / sidelines.
///
/// `target` is [`super::NonEmpty`] — the classify stage resolves the forward
/// reference (`［＃「対象」に傍点］`) before emitting the node.
///
/// `consumed_predecessor` records whether the classifier
/// pulled this node's source span back over an immediately-preceding
/// literal of `target` (the canonical `target［＃「target」に傍点］`
/// shape). When true, the renderer's `<em class="bouten">target</em>`
/// is the *sole* visible copy of the literal — the surrounding plain
/// run was truncated to make room. The serializer reads this flag to
/// re-emit the literal before `［＃「target」に傍点］`, preserving
/// the parse∘serialize fixed-point invariant. When false (target
/// appears earlier in the paragraph but not immediately before the
/// bracket), the literal stays in the preceding plain run and the
/// serializer emits only the bracket form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bouten<'src> {
    /// Which 傍点 / 傍線 mark decorates the run.
    pub kind: BoutenKind,
    /// The run the marks are applied to.
    pub target: super::NonEmpty<Content<'src>>,
    /// Which side of the base text the marks sit on (`左に` ⇒ `Left`).
    pub position: BoutenPosition,
    /// Whether the classifier pulled this node's span back over an
    /// immediately-preceding literal of `target`; see the struct docs.
    pub consumed_predecessor: bool,
}

/// Tate-chu-yoko (horizontal embedding).
///
/// `text` is [`super::NonEmpty`] — empty TCY is a parse bug, not a
/// valid state.
///
/// `consumed_predecessor` mirrors [`Bouten::consumed_predecessor`] —
/// see that docstring. Forward-reference TCY (`text［＃「text」は縦
/// 中横］`) follows the same back-ref consume model and the same
/// round-trip contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombineUpright<'src> {
    /// The run set horizontally within the vertical line.
    pub text: super::NonEmpty<Content<'src>>,
    /// Whether the classifier pulled this node's span back over an
    /// immediately-preceding literal of `text`; see the struct docs.
    pub consumed_predecessor: bool,
}

/// Bold / italic emphasis.
///
/// The forward-reference leaf form of 太字 / 斜体
/// (`X［＃「X」は太字］` / `X［＃「X」は斜体］`). `kind` selects 太字
/// (`<b>`) or 斜体 (`<i>`); `text` is the emphasised run. The range /
/// block forms (`［＃太字］…［＃太字終わり］`, `［＃ここから太字］…`)
/// are paired containers ([`crate::ContainerKind::Bold`] /
/// [`crate::ContainerKind::Italic`]), not this node.
///
/// `text` is [`super::NonEmpty`] — empty emphasis is a parse bug.
///
/// `consumed_predecessor` mirrors [`Bouten::consumed_predecessor`] and
/// [`CombineUpright::consumed_predecessor`]: when the classify stage pulled the node's
/// source span back over the immediately-preceding literal of `text`,
/// the serializer re-emits that literal before `［＃「text」は太字］` to
/// hold the parse∘serialize fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emphasis<'src> {
    /// Which typographic treatment (太字 / 斜体 / 上付き …) applies.
    pub kind: EmphasisKind,
    /// The emphasised run.
    pub text: super::NonEmpty<Content<'src>>,
    /// Whether the classifier pulled this node's span back over an
    /// immediately-preceding literal of `text`; see the struct docs.
    pub consumed_predecessor: bool,
}

/// Gaiji (out-of-character-range glyph).
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
    /// `true` when the source had no leading `※` — the no-refmark
    /// `［＃…］` external-character form (#122). Drives serialize to omit
    /// the `※` so `parse ∘ serialize` stays a fixed point. Not a wire
    /// field.
    pub standalone: bool,
}

/// Warichu (split annotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Warichu<'src> {
    /// First (upper / right) of the two stacked half-size lines.
    pub upper: Content<'src>,
    /// Second (lower / left) of the two stacked half-size lines.
    pub lower: Content<'src>,
}

/// Aozora heading — a 大 / 中 / 小 `kind` (level) and a `style`
/// (standard / 同行 / 窓).
///
/// `text` is [`super::NonEmpty`] — every heading carries a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heading<'src> {
    /// The 大 / 中 / 小 outline level.
    pub kind: HeadingKind,
    /// Standard / 同行 / 窓 style.
    pub style: HeadingStyle,
    /// The heading label.
    pub text: super::NonEmpty<Content<'src>>,
}

/// Forward-reference heading hint, carrying the intended outline `level`
/// (大 / 中 / 小) and `style` (standard / 同行 / 窓).
///
/// `target` is [`super::NonEmptyStr`] — the classify stage only emits the hint
/// after a `「対象」` quoted target landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingHint<'src> {
    /// Intended outline level — 大 / 中 / 小. Typed as [`HeadingKind`] (not a
    /// raw `u8`) so an out-of-range level is unrepresentable; the numeric
    /// `data-level` is derived via [`HeadingKind::outline_level`].
    pub level: HeadingKind,
    /// Standard / 同行 / 窓 style.
    pub style: HeadingStyle,
    /// The quoted target run the hint promotes to a heading.
    pub target: super::NonEmptyStr<'src>,
}

/// Illustration metadata.
///
/// `file` is [`super::NonEmptyStr`] — `［＃挿絵（）入る］` with empty
/// path is a parse bug, not a valid state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Illustration<'src> {
    /// Image path / filename (becomes the `<img src>`).
    pub file: super::NonEmptyStr<'src>,
    /// Optional figure number from the `［＃挿絵{N}（…）入る］` numbered form,
    /// kept as the raw digit string (so `０１` / `10` round-trip verbatim).
    /// `None` for the plain `［＃挿絵（…）入る］`.
    pub number: Option<super::NonEmptyStr<'src>>,
    /// Optional verbatim pixel-size note (`横W×縦H`) from the
    /// `［＃挿絵（file、横W×縦H）入る］` form, kept out of `file` so the
    /// rendered `<img src>` stays a clean path. `None` when absent.
    pub dimensions: Option<&'src str>,
    /// Optional figure caption (`「caption」入る` / キャプション付き forms),
    /// rendered as a `<figcaption>`. `None` when the figure has no caption.
    pub caption: Option<Content<'src>>,
    /// The free-text description that precedes `（…）入る` in the *general*
    /// image form `［＃<説明>（file）入る］` — 図 / 地図 / 口絵 / コンドル博士の図
    /// … — per <https://www.aozora.gr.jp/annotation/graphics.html>, where
    /// the leading text is the image's alt. Becomes the `<img alt>`.
    /// `None` for the keyword `挿絵` form, which carries no leading
    /// description (and renders an empty `alt`).
    pub description: Option<&'src str>,
}

/// Generic annotation.
///
/// `raw` is [`super::NonEmptyStr`] — annotation always carries the
/// raw bytes between `［＃` and `］`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Directive<'src> {
    /// The raw bytes between `［＃` and `］` (kept verbatim for round-trip).
    pub raw: super::NonEmptyStr<'src>,
    /// How this annotation is classified. See [`DirectiveKind`].
    pub kind: DirectiveKind,
}

/// Chinese-reading-order mark (`返り点`).
///
/// `mark` is [`super::NonEmptyStr`] — empty kaeriten is a parse bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kaeriten<'src> {
    /// The kanbun reading-order mark (e.g. `レ`, `一`, `上`).
    pub mark: super::NonEmptyStr<'src>,
}

/// Double-angle quotation payload (input `≪…≫`, display `《…》`).
///
/// `content` is [`super::NonEmpty`] — the classify stage pre-filters empty
/// `≪≫` to plain text before allocation, so a `AngleQuote`
/// node is never emitted with empty content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AngleQuote<'src> {
    /// The quoted run (input `≪…≫`, displayed as `《…》`).
    pub content: super::NonEmpty<Content<'src>>,
}

/// [`Node`] classifier methods. Mirror the inherent methods on
/// the legacy owned [`aozora_syntax owned API (no longer present)`] 1:1 so a renderer compiled
/// against either AST takes the same code path and the same XML
/// snapshot string.
impl Node<'_> {
    /// True iff this node occupies a paragraph position in the
    /// document tree (and therefore shouldn't be wrapped inside a
    /// surrounding text run by the renderer).
    #[must_use]
    pub const fn is_block(&self) -> bool {
        matches!(
            self,
            Self::Indent(_)
                | Self::AlignEnd(_)
                | Self::Center(_)
                | Self::Warichu(_)
                | Self::Framed(_)
                | Self::PageBreak
                | Self::SectionBreak(_)
                | Self::Heading(_)
                | Self::Illustration(_)
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
            Self::Heading(_)
                | Self::AlignEnd(_)
                | Self::Center(_)
                | Self::Warichu(_)
                | Self::Framed(_)
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
            Self::CombineUpright(_) => "aozora_tcy",
            Self::Gaiji(_) => "aozora_gaiji",
            Self::Indent(_) => "aozora_indent",
            Self::AlignEnd(_) => "aozora_align_end",
            Self::Center(_) => "aozora_center",
            Self::Warichu(_) => "aozora_warichu",
            Self::Framed(_) => "aozora_keigakomi",
            Self::PageBreak => "aozora_page_break",
            Self::SectionBreak(_) => "aozora_section_break",
            Self::Heading(_) => "aozora_heading",
            Self::HeadingHint(_) => "aozora_heading_hint",
            Self::Illustration(_) => "aozora_sashie",
            Self::Kaeriten(_) => "aozora_kaeriten",
            Self::Directive(_) => "aozora_annotation",
            Self::AngleQuote(_) => "aozora_angle_quote",
            Self::Emphasis(_) => "aozora_emphasis",
            Self::MarginNote(_) => "aozora_side_note",
            Self::Container(_) => "aozora_container",
        }
    }

    /// Cross-cutting [`crate::NodeKind`] tag for this node.
    ///
    /// Driver wire formats (`aozora-ffi` / `aozora-wasm` / `aozora-py`)
    /// project to the camelCase string via
    /// [`NodeKind::as_json_tag`](crate::NodeKind::as_json_tag);
    /// internal consumers can `match` on the typed enum directly.
    #[must_use]
    pub const fn kind(&self) -> crate::NodeKind {
        use crate::NodeKind;
        match self {
            Self::Ruby(_) => NodeKind::Ruby,
            Self::Bouten(_) => NodeKind::Bouten,
            Self::CombineUpright(_) => NodeKind::CombineUpright,
            Self::Gaiji(_) => NodeKind::Gaiji,
            Self::Indent(_) => NodeKind::Indent,
            Self::AlignEnd(_) => NodeKind::AlignEnd,
            Self::Center(_) => NodeKind::Center,
            Self::Warichu(_) => NodeKind::Warichu,
            Self::Framed(_) => NodeKind::Framed,
            Self::PageBreak => NodeKind::PageBreak,
            Self::SectionBreak(_) => NodeKind::SectionBreak,
            Self::Heading(_) => NodeKind::Heading,
            Self::HeadingHint(_) => NodeKind::HeadingHint,
            Self::Illustration(_) => NodeKind::Illustration,
            Self::Kaeriten(_) => NodeKind::Kaeriten,
            Self::Directive(_) => NodeKind::Directive,
            Self::AngleQuote(_) => NodeKind::AngleQuote,
            Self::Emphasis(_) => NodeKind::Emphasis,
            Self::MarginNote(_) => NodeKind::MarginNote,
            Self::Container(_) => NodeKind::Container,
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
        assert_copy::<Node<'static>>();
        assert_copy::<Content<'static>>();
        assert_copy::<Ruby<'static>>();
        assert_copy::<Bouten<'static>>();
        assert_copy::<Emphasis<'static>>();
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
        // in the legacy Node test suite. Our concern here is
        // that the borrowed mirror returns the SAME strings.
        let kaeriten = Kaeriten {
            mark: super::super::NonEmptyStr::new("x").unwrap(),
        };
        let n = Node::Kaeriten(&kaeriten);
        assert_eq!(n.xml_node_name(), "aozora_kaeriten");
        assert!(!n.contains_inlines());

        assert!(Node::PageBreak.is_block());
        assert_eq!(Node::PageBreak.xml_node_name(), "aozora_page_break");
    }

    #[test]
    fn block_variants_report_block() {
        assert!(Node::Indent(Indent { amount: 2 }).is_block());
        assert!(Node::SectionBreak(SectionKind::Kaicho).is_block());
    }

    #[test]
    fn inline_variants_are_not_block() {
        let ruby = Ruby {
            base: super::super::NonEmpty::new(Content::Plain("x")).unwrap(),
            reading: super::super::NonEmpty::new(Content::Plain("x")).unwrap(),
            delim_explicit: false,
            side: RubySide::Right,
        };
        assert!(!Node::Ruby(&ruby).is_block());

        let kaeriten = Kaeriten {
            mark: super::super::NonEmptyStr::new("x").unwrap(),
        };
        assert!(!Node::Kaeriten(&kaeriten).is_block());
    }

    #[test]
    fn ruby_carries_both_base_and_reading() {
        let r = Ruby {
            base: super::super::NonEmpty::new(Content::Plain("青梅")).unwrap(),
            reading: super::super::NonEmpty::new(Content::Plain("おうめ")).unwrap(),
            delim_explicit: true,
            side: RubySide::Right,
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
            standalone: false,
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
            standalone: false,
        };
        assert_eq!(g.ucs, Some(Resolved::Multi("か゚")));
    }
}
