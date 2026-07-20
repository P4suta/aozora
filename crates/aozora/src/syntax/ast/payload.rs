//! No-lifetime AST payload structs and the `Content` / `Segment` two-tier
//! content model.
//!
//! Every text slice is a [`StrId`]; a non-empty content run is a
//! [`ContentRange`]; a segment run is a [`SegRange`]; a bare `Content` field
//! is an inline [`Content`]. Each scalar payload is held inline
//! (no `Box`/`Id`), so the whole cluster stays `Copy`.

#[cfg(any(feature = "pandoc", test))]
use core::fmt;

use crate::encoding::gaiji::{GaijiCanonical, MenKuTen, Resolved};

use crate::syntax::format::{ForwardAttr, ForwardOrigin, LineFormat};
use crate::syntax::{
    DirectiveKind, HeadingKind, HeadingStyle, MarginNoteKind, NodeKind, RubySide, SectionKind,
};

use super::intern::StrId;
use super::store::{ContentRange, NodeStore, SegRange};

/// Body content that may carry nested Aozora constructs. Two-tier: a single
/// plain run or a mixed sequence of segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Content {
    /// Plain text.
    Plain(StrId),
    /// Mixed text + nested constructs.
    Segments(SegRange),
}

/// One element of a [`Content::Segments`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Segment {
    /// Plain-text run between nested constructs.
    Text(StrId),
    /// Nested 外字 reference.
    Gaiji(Gaiji),
    /// Nested generic annotation.
    Directive(Directive),
}

impl GaijiCanonicalOwned {
    /// Reconstruct the [`GaijiCanonical`], resolving the `Unresolved` mencode
    /// `StrId` against `store`. The single bridge to the encoding layer's
    /// canonical authority (`resolve` / `write_mencode`), kept private so the
    /// owned API surface stays owned.
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

    /// `true` when the source carried a mencode tail. Owned counterpart of
    /// [`GaijiCanonical::has_mencode`] — store-free (only the variant matters).
    #[must_use]
    pub(crate) fn has_mencode(self) -> bool {
        !matches!(self, Self::Unresolved { mencode: None })
    }

    /// Write the canonical mencode token (without the leading `、`). Owned
    /// counterpart of [`GaijiCanonical::write_mencode`]; delegates to the single
    /// encoding authority via the private `to_canonical` bridge.
    ///
    /// # Errors
    ///
    /// Propagates the writer's own errors.
    ///
    /// # Panics
    ///
    /// Panics if an `Unresolved` mencode `StrId` was not produced by `store`.
    #[cfg(any(feature = "pandoc", test))]
    pub(crate) fn write_mencode<W: fmt::Write>(self, store: &NodeStore, w: &mut W) -> fmt::Result {
        self.to_canonical(store).write_mencode(w)
    }
}

impl Gaiji {
    /// Resolve to a concrete glyph via the canonical value: rebuilds the
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
    pub(crate) fn resolve(&self, store: &NodeStore) -> Option<Resolved> {
        self.canonical
            .to_canonical(store)
            .resolve(store.resolve_str(self.hint))
    }
}

/// Ruby (furigana) annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ruby {
    /// Base text the reading annotates.
    pub base: ContentRange,
    /// Furigana reading.
    pub reading: ContentRange,
    /// Which side the reading sits on.
    pub side: RubySide,
    /// Render-only forward emphasis applied to the base (#384). Set by the
    /// lowering pass when a declined forward directive `［＃「X」に傍点/罫囲み/
    /// 行右小書き/…］` (a [`ForwardOrigin::Referenced`](crate::syntax::ForwardOrigin)
    /// leaf) names this ruby's base as its *unique* preceding referent — the
    /// classic `｜X《y》…［＃「X」は罫囲み］` where the target is a ruby base and
    /// so cannot be pulled into a plain forward leaf (bouten-over-ruby is not
    /// representable). The renderer wraps the base in the attribute's emphasis
    /// element; the directive leaf stays `Referenced` (serializes the bracket
    /// verbatim, renders nothing), so `base_emphasis` is never read by
    /// `to_source` — it is a render decoration, not a serialized field. As a
    /// `Copy` `Option<ForwardAttr>` it keeps `Ruby` `Copy` and inline.
    pub base_emphasis: Option<ForwardAttr>,
}

/// Margin note (注記 / 傍記).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MarginNote {
    /// 注記 vs 傍記.
    pub kind: MarginNoteKind,
    /// Preceding run the note attaches to.
    pub base: ContentRange,
    /// Gloss / redaction text.
    pub note: ContentRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardPayload {
    None,
    NestedSource,
    AccentBody(StrId),
}

/// Forward-reference emphasis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ForwardFormat {
    /// Which forward-scope attribute decorates the run.
    pub attr: ForwardAttr,
    /// The decorated run.
    pub target: ContentRange,
    /// Target-text provenance.
    pub origin: ForwardOrigin,
    pub payload: ForwardPayload,
}

/// Owned, lifetime-free counterpart of
/// [`crate::encoding::gaiji::GaijiCanonical`], whose `Unresolved` variant
/// carries a `&'src str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GaijiCanonicalOwned {
    /// Structured `第N水準P-K-T`. Reused lifetime-free `MenKuTen`.
    MenKuTen(MenKuTen),
    /// Explicit `U+XXXX` codepoint.
    Unicode(char),
    /// Verbatim tail.
    Unresolved {
        /// The raw mencode tail, interned. `None` when absent.
        mencode: Option<StrId>,
    },
}

/// Out-of-range glyph (外字).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Gaiji {
    /// Free-form source description / resolver fallback key.
    pub hint: StrId,
    /// Typed canonical value.
    pub canonical: GaijiCanonicalOwned,
    /// Whether a present mencode tail was separated from the description by `、`.
    pub mencode_separator: bool,
    /// `true` for the no-`※` standalone form.
    pub standalone: bool,
}

/// Heading (見出し).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Heading {
    /// 大 / 中 / 小 outline level.
    pub kind: HeadingKind,
    /// Standard / 同行 / 窓 style.
    pub style: HeadingStyle,
    /// Heading label.
    pub text: ContentRange,
}

/// Heading hint (見出し指定).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeadingHint {
    /// Intended outline level.
    pub level: HeadingKind,
    /// Standard / 同行 / 窓 style.
    pub style: HeadingStyle,
    /// Quoted target run.
    pub target: StrId,
    /// Whether the quoted target is absent from the preceding source — a
    /// no-referent forward heading whose target run is itself the heading text
    /// (the [`ForwardOrigin::SelfContained`](crate::syntax::ForwardOrigin) analogue for
    /// headings). When set, the hint renders the target visibly instead of as a
    /// hidden marker; it still serializes bracket-only (no fabricated referent
    /// line), keeping the round-trip a fixed point. A hint whose target *is*
    /// preceded leaves this `false`.
    pub self_contained: bool,
}

/// Illustration (挿絵).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Illustration {
    /// Image path / filename.
    pub file: StrId,
    /// Optional figure number (raw digits).
    pub number: Option<StrId>,
    /// Optional verbatim `横W×縦H` size note.
    pub dimensions: Option<StrId>,
    /// Optional caption.
    pub caption: Option<Content>,
    /// Optional alt description.
    pub description: Option<StrId>,
}

/// Generic annotation (注記).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Directive {
    /// Raw bytes between `［＃` and `］`.
    pub raw: StrId,
    /// Classification.
    pub kind: DirectiveKind,
}

/// Kanbun reading-order mark (返り点).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Kaeriten {
    /// Kanbun reading-order mark.
    pub mark: StrId,
}

/// Angle quote (`≪…≫` -> `《…》`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AngleQuote {
    /// Quoted run.
    pub content: ContentRange,
}

/// A single, no-lifetime AST node. Every scalar payload is held INLINE
/// (no `Box`/`Id`); `Copy` scalar-enum variants carry their
/// value directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Node {
    /// Ruby (furigana) annotation.
    Ruby(Ruby),
    /// Forward-reference emphasis.
    Format(ForwardFormat),
    /// Out-of-range glyph (外字).
    Gaiji(Gaiji),
    /// Line-level format — `Copy` enum.
    Line(LineFormat),
    /// Page break — unit.
    PageBreak,
    /// Section break — `Copy` enum.
    SectionBreak(SectionKind),
    /// End of body — unit.
    BodyEnd,
    /// Forced line break — unit.
    ForcedBreak,
    /// Heading (見出し).
    Heading(Heading),
    /// Heading hint (見出し指定).
    HeadingHint(HeadingHint),
    /// Illustration (挿絵).
    Illustration(Illustration),
    /// Kanbun reading-order mark (返り点).
    Kaeriten(Kaeriten),
    /// Generic annotation (注記).
    Directive(Directive),
    /// Angle quote (`≪…≫` -> `《…》`).
    AngleQuote(AngleQuote),
    /// Margin note (注記 / 傍記).
    MarginNote(MarginNote),
}

impl Node {
    /// Cross-cutting [`crate::syntax::NodeKind`] tag for this node.
    #[must_use]
    pub(crate) const fn kind(self) -> NodeKind {
        use crate::syntax::NodeKind;
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
                LineFormat::Gothic => NodeKind::LineGothic,
                LineFormat::FontSizeAbsolute { .. } => NodeKind::LineFontSize,
            },
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_are_copy() {
        // Pin the Copy chain — every owned payload must stay Copy. If a future
        // field breaks Copy, this fails to compile.
        const fn assert_copy<T: Copy>() {}
        assert_copy::<Content>();
        assert_copy::<Segment>();
        assert_copy::<Node>();
        assert_copy::<Ruby>();
        assert_copy::<Gaiji>();
    }

    #[test]
    fn node_kind_tag_matches_variant() {
        assert_eq!(Node::PageBreak.kind(), NodeKind::PageBreak);
        assert_eq!(
            Node::SectionBreak(SectionKind::Kaicho).kind(),
            NodeKind::SectionBreak
        );
    }

    #[test]
    fn gaiji_canonical_owned_has_mencode_tracks_the_tail() {
        let mut store = NodeStore::new();
        let id = store.intern("第3水準1-85-54");
        // A tail is present for every arm except the bare `Unresolved{None}`.
        assert!(
            GaijiCanonicalOwned::MenKuTen(MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            })
            .has_mencode()
        );
        assert!(GaijiCanonicalOwned::Unicode('竜').has_mencode());
        assert!(GaijiCanonicalOwned::Unresolved { mencode: Some(id) }.has_mencode());
        assert!(
            !GaijiCanonicalOwned::Unresolved { mencode: None }.has_mencode(),
            "no tail means has_mencode is false"
        );
    }

    #[test]
    fn gaiji_canonical_owned_write_mencode_emits_the_token() {
        let store = NodeStore::new();
        // The structured arms reproduce their canonical token verbatim (not the
        // empty `Default::default()` a stub would write).
        let mut buf = String::new();
        GaijiCanonicalOwned::MenKuTen(MenKuTen {
            plane: 1,
            ku: 85,
            ten: 54,
        })
        .write_mencode(&store, &mut buf)
        .unwrap();
        assert_eq!(buf, "第3水準1-85-54");

        let mut buf = String::new();
        GaijiCanonicalOwned::Unicode('A')
            .write_mencode(&store, &mut buf)
            .unwrap();
        assert_eq!(buf, "U+0041");
    }

    #[test]
    fn node_kind_distinguishes_forward_attrs() {
        use crate::syntax::alloc::Allocator;
        use crate::syntax::format::ForwardOrigin;
        use crate::syntax::{BoutenKind, BoutenPosition};

        let mut a = Allocator::new();
        let t = a.content_plain("青");
        let bouten = a.bouten(
            BoutenKind::Goma,
            t,
            BoutenPosition::Right,
            ForwardOrigin::Referenced,
        );
        assert_eq!(bouten.kind(), NodeKind::Bouten);

        let t2 = a.content_plain("12");
        let tcy = a.tate_chu_yoko(t2, ForwardOrigin::Referenced);
        assert_eq!(tcy.kind(), NodeKind::CombineUpright);

        // A non-bouten, non-tcy forward attribute falls to the default arm.
        let t3 = a.content_plain("重");
        let bold = a.forward_format(ForwardAttr::Bold, t3, ForwardOrigin::Reclaimed);
        assert_eq!(bold.kind(), NodeKind::Emphasis);
    }

    #[test]
    fn gaiji_resolve_matches_canonical_authority() {
        // Build an owned `Gaiji` per canonical arm and confirm the on-demand
        // glyph resolution is byte-identical to the
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
            let owned = Gaiji {
                hint: hint_id,
                canonical: owned_canonical,
                mencode_separator: true,
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
