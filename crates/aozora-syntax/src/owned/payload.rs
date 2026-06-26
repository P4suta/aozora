//! Owned, no-lifetime mirrors of the borrowed AST payload structs and the
//! [`Content`](crate::borrowed::Content) / [`Segment`](crate::borrowed::Segment)
//! two-tier content model.
//!
//! Every borrowed `&'src str` becomes a [`StrId`]; every borrowed
//! `NonEmpty<Content>` becomes a [`ContentRange`]; every borrowed
//! `&'src [Segment]` becomes a [`SegRange`]; bare `Content` fields become an
//! inline [`ContentOwned`]. Each `&'src X<'src>` scalar payload becomes its
//! owned `XOwned` mirror held inline (no `Box`/`Id`), so the whole cluster
//! stays `Copy` exactly like the borrowed tree.

use aozora_encoding::gaiji::{GaijiCanonical, MenKuTen, Resolved};

use crate::borrowed::{self, ForwardOrigin};
use crate::format::{ForwardAttr, LineFormat};
use crate::{
    Container, DirectiveKind, HeadingKind, HeadingStyle, MarginNoteKind, RubySide, SectionKind,
};

use super::intern::StrId;
use super::store::{ContentRange, NodeStore, SegRange};

/// Owned mirror of [`crate::borrowed::Content`]: body content that may
/// carry nested Aozora constructs. Two-tier like the borrowed form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContentOwned {
    /// Plain text. Borrowed `Plain(&'src str)`.
    Plain(StrId),
    /// Mixed text + nested constructs. Borrowed `Segments(&'src [Segment])`.
    Segments(SegRange),
}

/// Owned mirror of [`crate::borrowed::Segment`]: one element of a
/// [`ContentOwned::Segments`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentOwned {
    /// Plain-text run between nested constructs. Borrowed `Text(&'src str)`.
    Text(StrId),
    /// Nested 外字 reference. Borrowed `Gaiji(&'src Gaiji)` -> inline owned.
    Gaiji(GaijiOwned),
    /// Nested generic annotation. Borrowed `Directive(&'src Directive)` ->
    /// inline owned.
    Directive(DirectiveOwned),
}

impl ContentOwned {
    /// Materialise a borrowed [`Content`](borrowed::Content) into the `store`,
    /// interning its text and pushing any segment run into the segment pool.
    ///
    /// A `Plain` run interns to a single [`StrId`]; a `Segments` run converts
    /// each [`Segment`](borrowed::Segment) (assembled into a temporary `Vec`
    /// first so the `&mut store` borrows do not interleave) and pushes the run
    /// as a [`SegRange`]. The non-exhaustive forward-compat arm maps to the
    /// empty-segments analogue, matching the borrowed `Content::EMPTY`
    /// convention.
    #[must_use]
    pub fn from_borrowed(c: borrowed::Content<'_>, store: &mut NodeStore) -> Self {
        match c {
            borrowed::Content::Plain(s) => Self::Plain(store.intern(s)),
            borrowed::Content::Segments(segs) => {
                let owned: Vec<SegmentOwned> = segs
                    .iter()
                    .map(|&s| SegmentOwned::from_borrowed(s, store))
                    .collect();
                Self::Segments(store.push_segments(&owned))
            }
        }
    }
}

impl SegmentOwned {
    /// Materialise a borrowed [`Segment`](borrowed::Segment) into the `store`.
    #[must_use]
    pub fn from_borrowed(s: borrowed::Segment<'_>, store: &mut NodeStore) -> Self {
        match s {
            borrowed::Segment::Text(t) => Self::Text(store.intern(t)),
            borrowed::Segment::Gaiji(g) => Self::Gaiji(GaijiOwned::from_borrowed(g, store)),
            borrowed::Segment::Directive(d) => {
                Self::Directive(DirectiveOwned::from_borrowed(d, store))
            }
        }
    }
}

impl GaijiCanonicalOwned {
    /// Materialise a borrowed [`GaijiCanonical`] into the `store`, interning the
    /// verbatim mencode tail of the `Unresolved` form.
    #[must_use]
    pub fn from_borrowed(c: GaijiCanonical<'_>, store: &mut NodeStore) -> Self {
        match c {
            GaijiCanonical::MenKuTen(m) => Self::MenKuTen(m),
            GaijiCanonical::Unicode(ch) => Self::Unicode(ch),
            GaijiCanonical::Unresolved { mencode } => Self::Unresolved {
                mencode: mencode.map(|m| store.intern(m)),
            },
        }
    }
}

impl GaijiOwned {
    /// Materialise a borrowed [`Gaiji`](borrowed::Gaiji) into the `store`.
    #[must_use]
    pub fn from_borrowed(g: &borrowed::Gaiji<'_>, store: &mut NodeStore) -> Self {
        Self {
            hint: store.intern(g.hint),
            canonical: GaijiCanonicalOwned::from_borrowed(g.canonical, store),
            standalone: g.standalone,
        }
    }

    /// Resolve to a concrete glyph via the canonical value. Owned mirror of
    /// [`borrowed::Gaiji::resolve`](borrowed::Gaiji::resolve): rebuilds the
    /// lifetime-free `GaijiCanonical` against `store` and delegates to the
    /// single `GaijiCanonical::resolve` authority, passing [`Self::hint`] as
    /// the resolver's description fallback. The `Resolved` result is owned
    /// (no borrow of `store`).
    ///
    /// # Panics
    ///
    /// Panics if `self.hint` / the `Unresolved` mencode `StrId` were not
    /// produced by `store`'s interner.
    #[must_use]
    pub fn resolve(&self, store: &NodeStore) -> Option<Resolved> {
        let canonical = match self.canonical {
            GaijiCanonicalOwned::MenKuTen(m) => GaijiCanonical::MenKuTen(m),
            GaijiCanonicalOwned::Unicode(c) => GaijiCanonical::Unicode(c),
            GaijiCanonicalOwned::Unresolved { mencode } => GaijiCanonical::Unresolved {
                mencode: mencode.map(|id| store.resolve_str(id)),
            },
        };
        canonical.resolve(store.resolve_str(self.hint))
    }
}

impl DirectiveOwned {
    /// Materialise a borrowed [`Directive`](borrowed::Directive) into the
    /// `store`.
    #[must_use]
    pub fn from_borrowed(d: &borrowed::Directive<'_>, store: &mut NodeStore) -> Self {
        Self {
            raw: store.intern(d.raw.as_str()),
            kind: d.kind,
        }
    }
}

/// Push a borrowed `NonEmpty<Content>` field as a length-1 [`ContentRange`].
///
/// Builds the single owned content (which may itself append a segment run)
/// before the `push_contents` call, so the `&mut store` borrows never overlap.
fn push_one_content(store: &mut NodeStore, c: borrowed::Content<'_>) -> ContentRange {
    let owned = ContentOwned::from_borrowed(c, store);
    store.push_contents(&[owned])
}

/// Owned mirror of [`crate::borrowed::Ruby`] (furigana).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RubyOwned {
    /// Base text the reading annotates. Borrowed `NonEmpty<Content>`.
    pub base: ContentRange,
    /// Furigana reading. Borrowed `NonEmpty<Content>`.
    pub reading: ContentRange,
    /// Which side the reading sits on. Reused `RubySide`.
    pub side: RubySide,
}

/// Owned mirror of [`crate::borrowed::MarginNote`] (注記 / 傍記).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarginNoteOwned {
    /// 注記 vs 傍記. Reused `MarginNoteKind`.
    pub kind: MarginNoteKind,
    /// Preceding run the note attaches to. Borrowed `NonEmpty<Content>`.
    pub base: ContentRange,
    /// Gloss / redaction text. Borrowed `NonEmpty<Content>`.
    pub note: ContentRange,
}

/// Owned mirror of [`crate::borrowed::ForwardFormat`] (forward-reference
/// emphasis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardFormatOwned {
    /// Which forward-scope attribute decorates the run. Reused `ForwardAttr`.
    pub attr: ForwardAttr,
    /// The decorated run. Borrowed `NonEmpty<Content>`.
    pub target: ContentRange,
    /// Target-text provenance. Reused `ForwardOrigin`.
    pub origin: ForwardOrigin,
}

/// Owned mirror of [`aozora_encoding::gaiji::GaijiCanonical`]. Needed because
/// the borrowed enum's `Unresolved` variant carries `&'src str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaijiCanonicalOwned {
    /// Structured `第N水準P-K-T`. Reused lifetime-free `MenKuTen`.
    MenKuTen(MenKuTen),
    /// Explicit `U+XXXX` codepoint.
    Unicode(char),
    /// Verbatim tail. Borrowed `mencode: Option<&'src str>`.
    Unresolved {
        /// The raw mencode tail, interned. `None` when absent.
        mencode: Option<StrId>,
    },
}

/// Owned mirror of [`crate::borrowed::Gaiji`] (out-of-range glyph).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaijiOwned {
    /// Free-form source description / resolver fallback key. Borrowed
    /// `&'src str`.
    pub hint: StrId,
    /// Typed canonical value. Borrowed `GaijiCanonical<'src>`.
    pub canonical: GaijiCanonicalOwned,
    /// `true` for the no-`※` standalone form. Plain `bool`, unchanged.
    pub standalone: bool,
}

/// Owned mirror of [`crate::borrowed::Warichu`] (split annotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarichuOwned {
    /// First (upper / right) half-size line. Borrowed bare `Content<'src>`.
    pub upper: ContentOwned,
    /// Second (lower / left) half-size line. Borrowed bare `Content<'src>`.
    pub lower: ContentOwned,
}

/// Owned mirror of [`crate::borrowed::Heading`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingOwned {
    /// 大 / 中 / 小 outline level. Reused `HeadingKind`.
    pub kind: HeadingKind,
    /// Standard / 同行 / 窓 style. Reused `HeadingStyle`.
    pub style: HeadingStyle,
    /// Heading label. Borrowed `NonEmpty<Content>`.
    pub text: ContentRange,
}

/// Owned mirror of [`crate::borrowed::HeadingHint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingHintOwned {
    /// Intended outline level. Reused `HeadingKind`.
    pub level: HeadingKind,
    /// Standard / 同行 / 窓 style. Reused `HeadingStyle`.
    pub style: HeadingStyle,
    /// Quoted target run. Borrowed `NonEmptyStr<'src>`.
    pub target: StrId,
}

/// Owned mirror of [`crate::borrowed::Illustration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllustrationOwned {
    /// Image path / filename. Borrowed `NonEmptyStr<'src>`.
    pub file: StrId,
    /// Optional figure number (raw digits). Borrowed
    /// `Option<NonEmptyStr<'src>>`.
    pub number: Option<StrId>,
    /// Optional verbatim `横W×縦H` size note. Borrowed `Option<&'src str>`.
    pub dimensions: Option<StrId>,
    /// Optional caption. Borrowed bare `Option<Content<'src>>`.
    pub caption: Option<ContentOwned>,
    /// Optional alt description. Borrowed `Option<&'src str>`.
    pub description: Option<StrId>,
}

/// Owned mirror of [`crate::borrowed::Directive`] (generic annotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectiveOwned {
    /// Raw bytes between `［＃` and `］`. Borrowed `NonEmptyStr<'src>`.
    pub raw: StrId,
    /// Classification. Reused `DirectiveKind`.
    pub kind: DirectiveKind,
}

/// Owned mirror of [`crate::borrowed::Kaeriten`] (返り点).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KaeritenOwned {
    /// Kanbun reading-order mark. Borrowed `NonEmptyStr<'src>`.
    pub mark: StrId,
}

/// Owned mirror of [`crate::borrowed::AngleQuote`] (`≪…≫` -> `《…》`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AngleQuoteOwned {
    /// Quoted run. Borrowed `NonEmpty<Content>`.
    pub content: ContentRange,
}

/// Owned, no-lifetime mirror of [`crate::borrowed::Node`]. Every borrowed
/// `&'src X<'src>` payload is held INLINE as its owned `XOwned` (no `Box`/`Id`);
/// `Copy` scalar-enum variants are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeOwned {
    /// Borrowed `Ruby(&'src Ruby)`.
    Ruby(RubyOwned),
    /// Borrowed `Format(&'src ForwardFormat)`.
    Format(ForwardFormatOwned),
    /// Borrowed `Gaiji(&'src Gaiji)`.
    Gaiji(GaijiOwned),
    /// Borrowed `Line(LineFormat)` — reused `Copy` enum unchanged.
    Line(LineFormat),
    /// Borrowed `Warichu(&'src Warichu)`.
    Warichu(WarichuOwned),
    /// Borrowed `PageBreak` — unit.
    PageBreak,
    /// Borrowed `SectionBreak(SectionKind)` — reused `Copy` enum unchanged.
    SectionBreak(SectionKind),
    /// Borrowed `BodyEnd` — unit.
    BodyEnd,
    /// Borrowed `ForcedBreak` — unit.
    ForcedBreak,
    /// Borrowed `Heading(&'src Heading)`.
    Heading(HeadingOwned),
    /// Borrowed `HeadingHint(&'src HeadingHint)`.
    HeadingHint(HeadingHintOwned),
    /// Borrowed `Illustration(&'src Illustration)`.
    Illustration(IllustrationOwned),
    /// Borrowed `Kaeriten(&'src Kaeriten)`.
    Kaeriten(KaeritenOwned),
    /// Borrowed `Directive(&'src Directive)`.
    Directive(DirectiveOwned),
    /// Borrowed `AngleQuote(&'src AngleQuote)`.
    AngleQuote(AngleQuoteOwned),
    /// Borrowed `MarginNote(&'src MarginNote)`.
    MarginNote(MarginNoteOwned),
    /// Borrowed `Container(Container)` — reused `Copy` enum unchanged.
    Container(Container),
}

impl NodeOwned {
    /// Cross-cutting [`crate::NodeKind`] tag for this node. Owned mirror of
    /// [`crate::borrowed::Node::kind`].
    #[must_use]
    pub const fn kind(self) -> crate::NodeKind {
        use crate::NodeKind;
        match self {
            Self::Ruby(_) => NodeKind::Ruby,
            Self::Format(f) => match f.attr {
                ForwardAttr::Bouten { .. } => NodeKind::Bouten,
                ForwardAttr::CombineUpright => NodeKind::CombineUpright,
                _ => NodeKind::Emphasis,
            },
            Self::Gaiji(_) => NodeKind::Gaiji,
            Self::Line(l) => match l {
                LineFormat::Indent { .. } => NodeKind::Indent,
                LineFormat::AlignEnd { .. } => NodeKind::AlignEnd,
                LineFormat::Center { .. } => NodeKind::Center,
                LineFormat::Framed => NodeKind::Framed,
            },
            Self::Warichu(_) => NodeKind::Warichu,
            Self::PageBreak => NodeKind::PageBreak,
            Self::SectionBreak(_) => NodeKind::SectionBreak,
            Self::BodyEnd => NodeKind::BodyEnd,
            Self::ForcedBreak => NodeKind::ForcedBreak,
            Self::Heading(_) => NodeKind::Heading,
            Self::HeadingHint(_) => NodeKind::HeadingHint,
            Self::Illustration(_) => NodeKind::Illustration,
            Self::Kaeriten(_) => NodeKind::Kaeriten,
            Self::Directive(_) => NodeKind::Directive,
            Self::AngleQuote(_) => NodeKind::AngleQuote,
            Self::MarginNote(_) => NodeKind::MarginNote,
            Self::Container(_) => NodeKind::Container,
        }
    }

    /// Stable XML/element-style node name. Owned mirror of
    /// [`crate::borrowed::Node::xml_node_name`], value-for-value identical so
    /// the serializer's fallback placeholder (`<!-- unsupported-aozora: … -->`)
    /// reproduces the borrowed bytes exactly.
    #[must_use]
    pub const fn xml_node_name(self) -> &'static str {
        match self {
            Self::Ruby(_) => "aozora_ruby",
            Self::Format(f) => match f.attr {
                ForwardAttr::Bouten { .. } => "aozora_bouten",
                ForwardAttr::CombineUpright => "aozora_tcy",
                _ => "aozora_emphasis",
            },
            Self::Gaiji(_) => "aozora_gaiji",
            Self::Line(l) => match l {
                LineFormat::Indent { .. } => "aozora_indent",
                LineFormat::AlignEnd { .. } => "aozora_align_end",
                LineFormat::Center { .. } => "aozora_center",
                LineFormat::Framed => "aozora_keigakomi",
            },
            Self::Warichu(_) => "aozora_warichu",
            Self::PageBreak => "aozora_page_break",
            Self::SectionBreak(_) => "aozora_section_break",
            Self::BodyEnd => "aozora_body_end",
            Self::ForcedBreak => "aozora_forced_break",
            Self::Heading(_) => "aozora_heading",
            Self::HeadingHint(_) => "aozora_heading_hint",
            Self::Illustration(_) => "aozora_sashie",
            Self::Kaeriten(_) => "aozora_kaeriten",
            Self::Directive(_) => "aozora_annotation",
            Self::AngleQuote(_) => "aozora_angle_quote",
            Self::MarginNote(_) => "aozora_side_note",
            Self::Container(_) => "aozora_container",
        }
    }

    /// Materialise a borrowed [`Node`](borrowed::Node) into the `store`,
    /// mapping every `&'src` payload to its owned mirror.
    ///
    /// Each `NonEmpty<Content>` field becomes a length-1 [`ContentRange`]
    /// (via `push_one_content`); each bare `Content` field becomes an inline
    /// [`ContentOwned`]; each `&str` / `NonEmptyStr` field interns to a
    /// [`StrId`]. `Copy` scalar payloads (`LineFormat`, `SectionKind`,
    /// `Container`, the scalar enums) are reused verbatim.
    #[must_use]
    pub fn from_borrowed(src_node: borrowed::Node<'_>, store: &mut NodeStore) -> Self {
        match src_node {
            borrowed::Node::Ruby(r) => {
                let base = push_one_content(store, r.base.get());
                let reading = push_one_content(store, r.reading.get());
                Self::Ruby(RubyOwned {
                    base,
                    reading,
                    side: r.side,
                })
            }
            borrowed::Node::Format(f) => {
                let target = push_one_content(store, f.target.get());
                Self::Format(ForwardFormatOwned {
                    attr: f.attr,
                    target,
                    origin: f.origin,
                })
            }
            borrowed::Node::Gaiji(g) => Self::Gaiji(GaijiOwned::from_borrowed(g, store)),
            borrowed::Node::Line(lf) => Self::Line(lf),
            borrowed::Node::Warichu(w) => Self::Warichu(WarichuOwned {
                upper: ContentOwned::from_borrowed(w.upper, store),
                lower: ContentOwned::from_borrowed(w.lower, store),
            }),
            borrowed::Node::PageBreak => Self::PageBreak,
            borrowed::Node::SectionBreak(k) => Self::SectionBreak(k),
            borrowed::Node::BodyEnd => Self::BodyEnd,
            borrowed::Node::ForcedBreak => Self::ForcedBreak,
            borrowed::Node::Heading(h) => {
                let text = push_one_content(store, h.text.get());
                Self::Heading(HeadingOwned {
                    kind: h.kind,
                    style: h.style,
                    text,
                })
            }
            borrowed::Node::HeadingHint(h) => Self::HeadingHint(HeadingHintOwned {
                level: h.level,
                style: h.style,
                target: store.intern(h.target.as_str()),
            }),
            borrowed::Node::Illustration(s) => Self::Illustration(IllustrationOwned {
                file: store.intern(s.file.as_str()),
                number: s.number.map(|n| store.intern(n.as_str())),
                dimensions: s.dimensions.map(|d| store.intern(d)),
                caption: s.caption.map(|c| ContentOwned::from_borrowed(c, store)),
                description: s.description.map(|d| store.intern(d)),
            }),
            borrowed::Node::Kaeriten(k) => Self::Kaeriten(KaeritenOwned {
                mark: store.intern(k.mark.as_str()),
            }),
            borrowed::Node::Directive(a) => {
                Self::Directive(DirectiveOwned::from_borrowed(a, store))
            }
            borrowed::Node::AngleQuote(d) => {
                let content = push_one_content(store, d.content.get());
                Self::AngleQuote(AngleQuoteOwned { content })
            }
            borrowed::Node::MarginNote(s) => {
                let base = push_one_content(store, s.base.get());
                let note = push_one_content(store, s.note.get());
                Self::MarginNote(MarginNoteOwned {
                    kind: s.kind,
                    base,
                    note,
                })
            }
            borrowed::Node::Container(c) => Self::Container(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_payloads_are_copy() {
        // Pin the Copy chain — every owned payload mirrors a Copy borrowed
        // analogue. If a future field breaks Copy, this fails to compile.
        const fn assert_copy<T: Copy>() {}
        assert_copy::<ContentOwned>();
        assert_copy::<SegmentOwned>();
        assert_copy::<NodeOwned>();
        assert_copy::<RubyOwned>();
        assert_copy::<GaijiOwned>();
    }

    #[test]
    fn node_kind_tag_mirrors_borrowed() {
        assert_eq!(NodeOwned::PageBreak.kind(), crate::NodeKind::PageBreak);
        assert_eq!(
            NodeOwned::SectionBreak(SectionKind::Kaicho).kind(),
            crate::NodeKind::SectionBreak
        );
    }

    #[test]
    fn gaiji_owned_resolve_mirrors_borrowed() {
        // Convert a borrowed `Gaiji` to owned and confirm the on-demand glyph
        // resolution is byte-identical across every canonical arm. Uses
        // `GaijiCanonical::from_mencode` so the canonical values are realistic
        // (structured Unicode / 面区点 / verbatim / absent) without hand-built
        // `MenKuTen` internals.
        let mut store = NodeStore::new();
        for (hint, mencode) in [
            ("竜", Some("U+9F8D")),         // → Unicode
            ("熙", Some("第3水準1-14-29")), // → MenKuTen (面区点)
            ("謎の字", Some("未知の注記")), // → Unresolved { Some }
            ("謎", None),                   // → Unresolved { None }
        ] {
            let canonical = GaijiCanonical::from_mencode(mencode);
            let g = borrowed::Gaiji {
                hint,
                canonical,
                standalone: false,
            };
            let owned = GaijiOwned::from_borrowed(&g, &mut store);
            assert_eq!(
                owned.resolve(&store),
                g.resolve(),
                "owned gaiji resolve diverged for hint={hint:?} mencode={mencode:?}",
            );
        }
    }
}
