//! Owned, no-lifetime mirrors of the borrowed AST payload structs and the
//! `Content` / `Segment`
//! two-tier content model.
//!
//! Every borrowed `&'src str` becomes a [`StrId`]; every borrowed
//! `NonEmpty<Content>` becomes a [`ContentRange`]; every borrowed
//! `&'src [Segment]` becomes a [`SegRange`]; bare `Content` fields become an
//! inline [`ContentOwned`]. Each `&'src X<'src>` scalar payload becomes its
//! owned `XOwned` mirror held inline (no `Box`/`Id`), so the whole cluster
//! stays `Copy` exactly like the borrowed tree.

use core::fmt;

use aozora_encoding::gaiji::{GaijiCanonical, MenKuTen, Resolved};

use crate::format::{ForwardAttr, ForwardOrigin, LineFormat};
use crate::{
    Container, DirectiveKind, HeadingKind, HeadingStyle, MarginNoteKind, RubySide, SectionKind,
};

use super::intern::StrId;
use super::store::{ContentRange, NodeStore, SegRange};

/// Owned mirror of `crate::borrowed::Content`: body content that may
/// carry nested Aozora constructs. Two-tier like the borrowed form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContentOwned {
    /// Plain text. Borrowed `Plain(&'src str)`.
    Plain(StrId),
    /// Mixed text + nested constructs. Borrowed `Segments(&'src [Segment])`.
    Segments(SegRange),
}

/// Owned mirror of `crate::borrowed::Segment`: one element of a
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

impl GaijiCanonicalOwned {
    /// Reconstruct the lifetime-free [`GaijiCanonical`] this mirrors, resolving
    /// the `Unresolved` mencode `StrId` against `store`. The single bridge to
    /// the `aozora-encoding` canonical authority (`resolve` / `write_mencode`),
    /// kept private so the owned API surface stays owned. `GaijiCanonical`
    /// lives in `aozora-encoding`, not the borrowed AST module, so it outlives
    /// the borrowed tree's deletion.
    ///
    /// # Panics
    ///
    /// Panics if an `Unresolved` mencode `StrId` was not produced by `store`.
    fn to_canonical(self, store: &NodeStore) -> GaijiCanonical<'_> {
        match self {
            Self::MenKuTen(m) => GaijiCanonical::MenKuTen(m),
            Self::Unicode(c) => GaijiCanonical::Unicode(c),
            Self::Unresolved { mencode } => GaijiCanonical::Unresolved {
                mencode: mencode.map(|id| store.resolve_str(id)),
            },
        }
    }

    /// `true` when the source carried a mencode tail. Owned mirror of
    /// [`GaijiCanonical::has_mencode`] — store-free (only the variant matters).
    #[must_use]
    pub fn has_mencode(self) -> bool {
        !matches!(self, Self::Unresolved { mencode: None })
    }

    /// Write the canonical mencode token (without the leading `、`). Owned
    /// mirror of [`GaijiCanonical::write_mencode`]; delegates to the single
    /// encoding authority via the private `to_canonical` bridge.
    ///
    /// # Errors
    ///
    /// Propagates the writer's own errors.
    ///
    /// # Panics
    ///
    /// Panics if an `Unresolved` mencode `StrId` was not produced by `store`.
    pub fn write_mencode<W: fmt::Write>(self, store: &NodeStore, w: &mut W) -> fmt::Result {
        self.to_canonical(store).write_mencode(w)
    }
}

impl GaijiOwned {
    /// Resolve to a concrete glyph via the canonical value. Owned mirror of
    /// `borrowed::Gaiji::resolve`: rebuilds the
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
        self.canonical
            .to_canonical(store)
            .resolve(store.resolve_str(self.hint))
    }
}

/// Owned mirror of `crate::borrowed::Ruby` (furigana).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RubyOwned {
    /// Base text the reading annotates. Borrowed `NonEmpty<Content>`.
    pub base: ContentRange,
    /// Furigana reading. Borrowed `NonEmpty<Content>`.
    pub reading: ContentRange,
    /// Which side the reading sits on. Reused `RubySide`.
    pub side: RubySide,
}

/// Owned mirror of `crate::borrowed::MarginNote` (注記 / 傍記).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarginNoteOwned {
    /// 注記 vs 傍記. Reused `MarginNoteKind`.
    pub kind: MarginNoteKind,
    /// Preceding run the note attaches to. Borrowed `NonEmpty<Content>`.
    pub base: ContentRange,
    /// Gloss / redaction text. Borrowed `NonEmpty<Content>`.
    pub note: ContentRange,
}

/// Owned mirror of `crate::borrowed::ForwardFormat` (forward-reference
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

/// Owned mirror of `crate::borrowed::Gaiji` (out-of-range glyph).
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

/// Owned mirror of `crate::borrowed::Warichu` (split annotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarichuOwned {
    /// First (upper / right) half-size line. Borrowed bare `Content<'src>`.
    pub upper: ContentOwned,
    /// Second (lower / left) half-size line. Borrowed bare `Content<'src>`.
    pub lower: ContentOwned,
}

/// Owned mirror of `crate::borrowed::Heading`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingOwned {
    /// 大 / 中 / 小 outline level. Reused `HeadingKind`.
    pub kind: HeadingKind,
    /// Standard / 同行 / 窓 style. Reused `HeadingStyle`.
    pub style: HeadingStyle,
    /// Heading label. Borrowed `NonEmpty<Content>`.
    pub text: ContentRange,
}

/// Owned mirror of `crate::borrowed::HeadingHint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingHintOwned {
    /// Intended outline level. Reused `HeadingKind`.
    pub level: HeadingKind,
    /// Standard / 同行 / 窓 style. Reused `HeadingStyle`.
    pub style: HeadingStyle,
    /// Quoted target run. Borrowed `NonEmptyStr<'src>`.
    pub target: StrId,
}

/// Owned mirror of `crate::borrowed::Illustration`.
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

/// Owned mirror of `crate::borrowed::Directive` (generic annotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectiveOwned {
    /// Raw bytes between `［＃` and `］`. Borrowed `NonEmptyStr<'src>`.
    pub raw: StrId,
    /// Classification. Reused `DirectiveKind`.
    pub kind: DirectiveKind,
}

/// Owned mirror of `crate::borrowed::Kaeriten` (返り点).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KaeritenOwned {
    /// Kanbun reading-order mark. Borrowed `NonEmptyStr<'src>`.
    pub mark: StrId,
}

/// Owned mirror of `crate::borrowed::AngleQuote` (`≪…≫` -> `《…》`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AngleQuoteOwned {
    /// Quoted run. Borrowed `NonEmpty<Content>`.
    pub content: ContentRange,
}

/// Owned, no-lifetime mirror of `crate::borrowed::Node`. Every borrowed
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
    /// `crate::borrowed::Node::kind`.
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
    /// `crate::borrowed::Node::xml_node_name`, value-for-value identical so
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
    fn gaiji_owned_resolve_mirrors_canonical_authority() {
        // Build an owned `Gaiji` per canonical arm and confirm the on-demand
        // glyph resolution is byte-identical to the `aozora-encoding`
        // `GaijiCanonical::resolve` authority. Uses `GaijiCanonical::from_mencode`
        // so the canonical values are realistic (structured Unicode / 面区点 /
        // verbatim / absent) without hand-built `MenKuTen` internals.
        let mut store = NodeStore::new();
        for (hint, mencode) in [
            ("竜", Some("U+9F8D")),         // → Unicode
            ("熙", Some("第3水準1-14-29")), // → MenKuTen (面区点)
            ("謎の字", Some("未知の注記")), // → Unresolved { Some }
            ("謎", None),                   // → Unresolved { None }
        ] {
            let canonical = GaijiCanonical::from_mencode(mencode);
            let hint_id = store.intern(hint);
            let owned_canonical = match canonical {
                GaijiCanonical::MenKuTen(m) => GaijiCanonicalOwned::MenKuTen(m),
                GaijiCanonical::Unicode(c) => GaijiCanonicalOwned::Unicode(c),
                GaijiCanonical::Unresolved { mencode } => GaijiCanonicalOwned::Unresolved {
                    mencode: mencode.map(|m| store.intern(m)),
                },
            };
            let owned = GaijiOwned {
                hint: hint_id,
                canonical: owned_canonical,
                standalone: false,
            };
            assert_eq!(
                owned.resolve(&store),
                canonical.resolve(hint),
                "owned gaiji resolve diverged for hint={hint:?} mencode={mencode:?}",
            );
        }
    }
}
