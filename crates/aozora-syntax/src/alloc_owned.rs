//! Owned AST construction.
//!
//! [`OwnedAllocator`] builds [`NodeOwned`] and its payload types into an owned
//! [`NodeStore`] (the [`StrInterner`](super::owned::StrInterner) plus the flat
//! content / segment pools). Byte-equal strings share a single interned
//! [`StrId`](super::owned::StrId).
//!
//! ## Canonicalisation
//!
//! The builders apply these canonicalisations so the produced tree resolves to
//! a single byte-identical form:
//!
//! - `content_plain("")` and `content_segments(&[])` both canonicalise to the
//!   empty-segments form (an empty [`SegRange`](super::owned::SegRange)).
//! - `content_segments` collapses an all-`Text` input into a single
//!   concatenated `Plain` (the concatenation is re-interned).
//! - Each `NonEmpty<Content>` field becomes a length-1 [`ContentRange`]; the
//!   non-empty contract is upheld with an `expect` message (a classifier bug,
//!   never reachable for well-formed emit sites).
//! - `make_gaiji` interns the mencode tail whenever it is present, so the
//!   classification and the retained `Unresolved` id stay in sync.
//!
//! ## Status
//!
//! This is the **sole** AST builder: the lex pipeline's `classify` stage drives
//! it directly.

use aozora_encoding::gaiji::GaijiCanonical;

use crate::format::{ForwardAttr, ForwardOrigin, LineFormat};
use crate::{
    BoutenKind, BoutenPosition, Container, DirectiveKind, HeadingKind, HeadingStyle,
    MarginNoteKind, RubySide, SectionKind,
};

use super::owned::{
    AngleQuoteOwned, ContentOwned, ContentRange, DirectiveOwned, ForwardFormatOwned,
    GaijiCanonicalOwned, GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned,
    KaeritenOwned, MarginNoteOwned, NodeOwned, NodeStore, RubyOwned, SegmentOwned, WarichuOwned,
};

/// `true` for the canonical empty-content form (an empty segment run).
fn is_empty_content(c: ContentOwned) -> bool {
    matches!(c, ContentOwned::Segments(r) if r.len == 0)
}

/// Builder for [`NodeOwned`] and its payload types.
///
/// Owns the [`NodeStore`] every produced handle resolves against; call
/// [`Self::into_store`] at the end of a parse to hand it to the lex output.
#[derive(Debug, Default)]
pub struct OwnedAllocator {
    store: NodeStore,
}

#[allow(
    clippy::unused_self,
    reason = "every builder takes &(mut) self even when it is a pure wrapper, so call sites have a uniform shape."
)]
impl OwnedAllocator {
    /// New allocator backed by an empty [`NodeStore`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the backing store (e.g. to resolve a handle mid-build).
    #[must_use]
    pub fn store(&self) -> &NodeStore {
        &self.store
    }

    /// Finish allocation and return the owning [`NodeStore`] so the caller can
    /// move it into the lex output and inspect its interner dedup counters.
    #[must_use]
    pub fn into_store(self) -> NodeStore {
        self.store
    }

    // ---------------------------------------------------------------------
    // Content / segment builders
    // ---------------------------------------------------------------------

    /// Build a plain-text body content. Empty input canonicalises to the
    /// empty-segments form.
    pub fn content_plain(&mut self, s: &str) -> ContentOwned {
        if s.is_empty() {
            ContentOwned::Segments(self.store.push_segments(&[]))
        } else {
            ContentOwned::Plain(self.store.intern(s))
        }
    }

    /// Build a body content from a sequence of segments. Empty input → the
    /// empty-segments form; all-`Text` input collapses into a single
    /// concatenated `Plain` (re-interned).
    pub fn content_segments(&mut self, segs: &[SegmentOwned]) -> ContentOwned {
        if segs.is_empty() {
            return ContentOwned::Segments(self.store.push_segments(&[]));
        }
        if segs.iter().all(|s| matches!(s, SegmentOwned::Text(_))) {
            // Resolve each text run, concatenate, and re-intern the all-`Text`
            // collapse. Pre-size the buffer from the resolved lengths to avoid
            // reallocation.
            let total: usize = segs
                .iter()
                .map(|s| match s {
                    SegmentOwned::Text(id) => self.store.resolve_str(*id).len(),
                    _ => 0,
                })
                .sum();
            let mut buf = String::with_capacity(total);
            for s in segs {
                if let SegmentOwned::Text(id) = s {
                    buf.push_str(self.store.resolve_str(*id));
                }
            }
            return ContentOwned::Plain(self.store.intern(&buf));
        }
        ContentOwned::Segments(self.store.push_segments(segs))
    }

    /// `SegmentOwned::Text(s)` — interns the string.
    pub fn seg_text(&mut self, s: &str) -> SegmentOwned {
        SegmentOwned::Text(self.store.intern(s))
    }

    /// `SegmentOwned::Gaiji(g)` — wraps a payload built via [`Self::make_gaiji`].
    #[must_use]
    pub fn seg_gaiji(&self, g: GaijiOwned) -> SegmentOwned {
        SegmentOwned::Gaiji(g)
    }

    /// `SegmentOwned::Directive(a)` — wraps a payload built via [`Self::make_directive`].
    #[must_use]
    pub fn seg_annotation(&self, a: DirectiveOwned) -> SegmentOwned {
        SegmentOwned::Directive(a)
    }

    // ---------------------------------------------------------------------
    // Payload builders (used by both Segment and Node constructors)
    // ---------------------------------------------------------------------

    /// Build a [`GaijiOwned`] payload. `mencode` is classified into its
    /// [`GaijiCanonicalOwned`] form; the verbatim tail is interned for the
    /// `Unresolved` arm. The resolved glyph is derived on demand.
    pub fn make_gaiji(
        &mut self,
        description: &str,
        mencode: Option<&str>,
        standalone: bool,
    ) -> GaijiOwned {
        let hint = self.store.intern(description);
        let canonical = match mencode {
            Some(m) => {
                // Intern the tail, then classify the byte-identical interned
                // string. Only the `Unresolved` arm keeps the id; the
                // structured arms discard it just like
                // `GaijiCanonical::from_mencode`.
                let id = self.store.intern(m);
                match GaijiCanonical::from_mencode(Some(self.store.resolve_str(id))) {
                    GaijiCanonical::MenKuTen(mkt) => GaijiCanonicalOwned::MenKuTen(mkt),
                    GaijiCanonical::Unicode(c) => GaijiCanonicalOwned::Unicode(c),
                    GaijiCanonical::Unresolved { .. } => {
                        GaijiCanonicalOwned::Unresolved { mencode: Some(id) }
                    }
                }
            }
            None => GaijiCanonicalOwned::Unresolved { mencode: None },
        };
        GaijiOwned {
            hint,
            canonical,
            standalone,
        }
    }

    /// Build a [`DirectiveOwned`] payload.
    ///
    /// # Panics
    ///
    /// Panics if `raw` is empty (the `NonEmptyStr` contract the classify stage
    /// upholds before emitting an annotation).
    pub fn make_directive(&mut self, raw: &str, kind: DirectiveKind) -> DirectiveOwned {
        assert!(
            !raw.is_empty(),
            "classify stage must emit Directive with non-empty raw bytes"
        );
        DirectiveOwned {
            raw: self.store.intern(raw),
            kind,
        }
    }

    /// Push a `NonEmpty<Content>` field as a length-1 [`ContentRange`],
    /// upholding the non-empty contract.
    fn push_nonempty(&mut self, c: ContentOwned, msg: &'static str) -> ContentRange {
        assert!(!is_empty_content(c), "{msg}");
        self.store.push_contents(&[c])
    }

    // ---------------------------------------------------------------------
    // Node variant constructors (one per NodeOwned variant)
    // ---------------------------------------------------------------------

    /// `NodeOwned::Ruby(Ruby { base, reading, side: Right })`.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `reading` is empty.
    pub fn ruby(&mut self, base: ContentOwned, reading: ContentOwned) -> NodeOwned {
        let base = self.push_nonempty(base, "classify stage must emit Ruby with non-empty base");
        let reading = self.push_nonempty(
            reading,
            "classify stage must emit Ruby with non-empty reading",
        );
        NodeOwned::Ruby(RubyOwned {
            base,
            reading,
            side: RubySide::Right,
        })
    }

    /// `NodeOwned::Ruby(Ruby { side: Left, … })` — a left-side ruby.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `reading` is empty.
    pub fn left_ruby(&mut self, base: ContentOwned, reading: ContentOwned) -> NodeOwned {
        let base = self.push_nonempty(base, "classify stage must emit Ruby with non-empty base");
        let reading = self.push_nonempty(
            reading,
            "classify stage must emit Ruby with non-empty reading",
        );
        NodeOwned::Ruby(RubyOwned {
            base,
            reading,
            side: RubySide::Left,
        })
    }

    /// `NodeOwned::MarginNote(MarginNote { kind, base, note })`.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `note` is empty.
    pub fn side_note(
        &mut self,
        kind: MarginNoteKind,
        base: ContentOwned,
        note: ContentOwned,
    ) -> NodeOwned {
        let base = self.push_nonempty(
            base,
            "classify stage must emit MarginNote with non-empty base",
        );
        let note = self.push_nonempty(
            note,
            "classify stage must emit MarginNote with non-empty note",
        );
        NodeOwned::MarginNote(MarginNoteOwned { kind, base, note })
    }

    /// `NodeOwned::Format` with a 傍点 attribute.
    ///
    /// # Panics
    ///
    /// Panics if `target` is empty.
    #[allow(
        clippy::too_many_arguments,
        reason = "every parameter is part of the bouten contract — kind / target / position / origin each carry independent semantics."
    )]
    pub fn bouten(
        &mut self,
        kind: BoutenKind,
        target: ContentOwned,
        position: BoutenPosition,
        origin: ForwardOrigin,
    ) -> NodeOwned {
        self.forward_format(ForwardAttr::Bouten { kind, position }, target, origin)
    }

    /// `NodeOwned::Format` with a 縦中横 attribute.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    pub fn tate_chu_yoko(&mut self, text: ContentOwned, origin: ForwardOrigin) -> NodeOwned {
        self.forward_format(ForwardAttr::CombineUpright, text, origin)
    }

    /// `NodeOwned::Format` with the given forward-scope `attr` applied to `text`.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    pub fn forward_format(
        &mut self,
        attr: ForwardAttr,
        text: ContentOwned,
        origin: ForwardOrigin,
    ) -> NodeOwned {
        let target = self.push_nonempty(
            text,
            "classify stage must emit a forward format with a non-empty target",
        );
        NodeOwned::Format(ForwardFormatOwned {
            attr,
            target,
            origin,
        })
    }

    /// `NodeOwned::Gaiji(g)`.
    #[must_use]
    pub fn gaiji(&self, g: GaijiOwned) -> NodeOwned {
        NodeOwned::Gaiji(g)
    }

    /// `NodeOwned::Line(lf)` — a single-line layout directive.
    #[must_use]
    pub fn line(&self, lf: LineFormat) -> NodeOwned {
        NodeOwned::Line(lf)
    }

    /// `NodeOwned::Warichu(Warichu { upper, lower })` — bare-content fields.
    #[must_use]
    pub fn warichu(&self, upper: ContentOwned, lower: ContentOwned) -> NodeOwned {
        NodeOwned::Warichu(WarichuOwned { upper, lower })
    }

    /// `NodeOwned::PageBreak`.
    #[must_use]
    pub fn page_break(&self) -> NodeOwned {
        NodeOwned::PageBreak
    }

    /// `NodeOwned::SectionBreak(k)`.
    #[must_use]
    pub fn section_break(&self, k: SectionKind) -> NodeOwned {
        NodeOwned::SectionBreak(k)
    }

    /// `NodeOwned::BodyEnd`.
    #[must_use]
    pub fn body_end(&self) -> NodeOwned {
        NodeOwned::BodyEnd
    }

    /// `NodeOwned::ForcedBreak`.
    #[must_use]
    pub fn forced_break(&self) -> NodeOwned {
        NodeOwned::ForcedBreak
    }

    /// `NodeOwned::Heading(Heading { kind, style, text })`.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    pub fn aozora_heading(
        &mut self,
        kind: HeadingKind,
        style: HeadingStyle,
        text: ContentOwned,
    ) -> NodeOwned {
        let text = self.push_nonempty(text, "classify stage must emit Heading with non-empty text");
        NodeOwned::Heading(HeadingOwned { kind, style, text })
    }

    /// `NodeOwned::HeadingHint(HeadingHint { level, style, target })`.
    ///
    /// # Panics
    ///
    /// Panics if `target` is empty.
    pub fn heading_hint(
        &mut self,
        level: HeadingKind,
        style: HeadingStyle,
        target: &str,
    ) -> NodeOwned {
        assert!(
            !target.is_empty(),
            "classify stage must emit HeadingHint with non-empty target"
        );
        NodeOwned::HeadingHint(HeadingHintOwned {
            level,
            style,
            target: self.store.intern(target),
        })
    }

    /// `NodeOwned::Illustration` — keyword 挿絵 form.
    ///
    /// # Panics
    ///
    /// Panics if `file` is empty.
    #[allow(
        clippy::too_many_arguments,
        reason = "every parameter is an independent part of the 挿絵 contract — file / number / dimensions / caption."
    )]
    pub fn sashie(
        &mut self,
        file: &str,
        number: Option<&str>,
        dimensions: Option<&str>,
        caption: Option<ContentOwned>,
    ) -> NodeOwned {
        assert!(
            !file.is_empty(),
            "classify stage must emit Illustration with non-empty file path"
        );
        let file = self.store.intern(file);
        // An empty `number` string is treated as absent (the `NonEmptyStr`
        // contract).
        let number = number
            .filter(|n| !n.is_empty())
            .map(|n| self.store.intern(n));
        let dimensions = dimensions.map(|d| self.store.intern(d));
        NodeOwned::Illustration(IllustrationOwned {
            file,
            number,
            dimensions,
            caption,
            description: None,
        })
    }

    /// `NodeOwned::Illustration` — general image form (leading description, no
    /// 挿絵 keyword / number / caption).
    ///
    /// # Panics
    ///
    /// Panics if `file` or `description` is empty.
    pub fn sashie_general(
        &mut self,
        file: &str,
        description: &str,
        dimensions: Option<&str>,
    ) -> NodeOwned {
        assert!(
            !file.is_empty(),
            "classify stage must emit Illustration with non-empty file path"
        );
        debug_assert!(
            !description.is_empty(),
            "classify stage must emit a general Illustration with a non-empty description"
        );
        let file = self.store.intern(file);
        let description = self.store.intern(description);
        let dimensions = dimensions.map(|d| self.store.intern(d));
        NodeOwned::Illustration(IllustrationOwned {
            file,
            number: None,
            dimensions,
            caption: None,
            description: Some(description),
        })
    }

    /// `NodeOwned::Kaeriten(Kaeriten { mark })`.
    ///
    /// # Panics
    ///
    /// Panics if `mark` is empty.
    pub fn kaeriten(&mut self, mark: &str) -> NodeOwned {
        assert!(
            !mark.is_empty(),
            "classify stage must emit Kaeriten with non-empty mark"
        );
        NodeOwned::Kaeriten(KaeritenOwned {
            mark: self.store.intern(mark),
        })
    }

    /// `NodeOwned::Directive(a)`.
    #[must_use]
    pub fn annotation(&self, a: DirectiveOwned) -> NodeOwned {
        NodeOwned::Directive(a)
    }

    /// `NodeOwned::AngleQuote(AngleQuote { content })`.
    ///
    /// # Panics
    ///
    /// Panics if `content` is empty.
    pub fn angle_quote(&mut self, content: ContentOwned) -> NodeOwned {
        let content = self.push_nonempty(
            content,
            "classify stage pre-filters empty AngleQuote into plain",
        );
        NodeOwned::AngleQuote(AngleQuoteOwned { content })
    }

    /// `NodeOwned::Container(c)`.
    #[must_use]
    pub fn container(&self, c: Container) -> NodeOwned {
        NodeOwned::Container(c)
    }
}

#[cfg(test)]
mod tests {
    //! Per-variant round-trip tests for `OwnedAllocator`. Each builds one
    //! `NodeOwned` and resolves its payloads against the store to confirm the
    //! fields match.

    use aozora_encoding::gaiji::MenKuTen;

    use super::*;

    /// Resolve a length-1 `ContentRange` to its `Plain` text, or `None` for a
    /// mixed / multi-entry run.
    fn plain(alloc: &OwnedAllocator, range: ContentRange) -> Option<&str> {
        alloc.store().content_range_as_plain(range)
    }

    #[test]
    fn ruby_round_trip() {
        let mut a = OwnedAllocator::new();
        let base = a.content_plain("青梅");
        let reading = a.content_plain("おうめ");
        let n = a.ruby(base, reading);
        let NodeOwned::Ruby(r) = n else {
            panic!("expected Ruby, got {n:?}");
        };
        assert_eq!(plain(&a, r.base), Some("青梅"));
        assert_eq!(plain(&a, r.reading), Some("おうめ"));
        assert_eq!(r.side, RubySide::Right);
    }

    #[test]
    fn left_ruby_is_left_side() {
        let mut a = OwnedAllocator::new();
        let base = a.content_plain("未");
        let reading = a.content_plain("ザル");
        let NodeOwned::Ruby(r) = a.left_ruby(base, reading) else {
            panic!("expected Ruby");
        };
        assert_eq!(r.side, RubySide::Left);
    }

    #[test]
    fn bouten_round_trip() {
        let mut a = OwnedAllocator::new();
        let target = a.content_plain("青空");
        let n = a.bouten(
            BoutenKind::Goma,
            target,
            BoutenPosition::Right,
            ForwardOrigin::Referenced,
        );
        let NodeOwned::Format(b) = n else {
            panic!("expected Format(Bouten), got {n:?}");
        };
        assert_eq!(
            b.attr,
            ForwardAttr::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
        );
        assert_eq!(plain(&a, b.target), Some("青空"));
        assert_eq!(b.origin, ForwardOrigin::Referenced);
    }

    #[test]
    fn tate_chu_yoko_round_trip() {
        let mut a = OwnedAllocator::new();
        let text = a.content_plain("12");
        let NodeOwned::Format(t) = a.tate_chu_yoko(text, ForwardOrigin::Referenced) else {
            panic!("expected Format(CombineUpright)");
        };
        assert_eq!(t.attr, ForwardAttr::CombineUpright);
        assert_eq!(plain(&a, t.target), Some("12"));
    }

    #[test]
    fn emphasis_round_trip() {
        let mut a = OwnedAllocator::new();
        let text = a.content_plain("重要");
        let NodeOwned::Format(e) =
            a.forward_format(ForwardAttr::Bold, text, ForwardOrigin::Reclaimed)
        else {
            panic!("expected Format(Bold)");
        };
        assert_eq!(e.attr, ForwardAttr::Bold);
        assert_eq!(plain(&a, e.target), Some("重要"));
        assert_eq!(e.origin, ForwardOrigin::Reclaimed);
    }

    #[test]
    fn side_note_round_trip() {
        let mut a = OwnedAllocator::new();
        let base = a.content_plain("未来");
        let note = a.content_plain("みらい");
        let NodeOwned::MarginNote(s) = a.side_note(MarginNoteKind::Gloss, base, note) else {
            panic!("expected MarginNote");
        };
        assert_eq!(s.kind, MarginNoteKind::Gloss);
        assert_eq!(plain(&a, s.base), Some("未来"));
        assert_eq!(plain(&a, s.note), Some("みらい"));
    }

    #[test]
    fn gaiji_full_metadata() {
        let mut a = OwnedAllocator::new();
        let g = a.make_gaiji("木＋吶のつくり", Some("第3水準1-85-54"), false);
        let NodeOwned::Gaiji(gn) = a.gaiji(g) else {
            panic!("expected Gaiji");
        };
        assert_eq!(a.store().resolve_str(gn.hint), "木＋吶のつくり");
        assert_eq!(
            gn.canonical,
            GaijiCanonicalOwned::MenKuTen(MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54,
            })
        );
        assert!(!gn.standalone);
    }

    #[test]
    fn gaiji_no_mencode_is_unresolved_none() {
        let mut a = OwnedAllocator::new();
        let g = a.make_gaiji("desc", None, true);
        let NodeOwned::Gaiji(gn) = a.gaiji(g) else {
            panic!("expected Gaiji");
        };
        assert_eq!(
            gn.canonical,
            GaijiCanonicalOwned::Unresolved { mencode: None }
        );
        assert!(gn.standalone);
    }

    #[test]
    fn gaiji_unresolved_keeps_mencode_tail() {
        let mut a = OwnedAllocator::new();
        let g = a.make_gaiji("謎", Some("未知の注記"), false);
        let NodeOwned::Gaiji(gn) = a.gaiji(g) else {
            panic!("expected Gaiji");
        };
        let GaijiCanonicalOwned::Unresolved { mencode: Some(id) } = gn.canonical else {
            panic!("expected Unresolved with a tail, got {:?}", gn.canonical);
        };
        assert_eq!(a.store().resolve_str(id), "未知の注記");
    }

    #[test]
    fn directive_round_trip() {
        let mut a = OwnedAllocator::new();
        let d = a.make_directive("［＃ママ］", DirectiveKind::Sic);
        let NodeOwned::Directive(dn) = a.annotation(d) else {
            panic!("expected Directive");
        };
        assert_eq!(a.store().resolve_str(dn.raw), "［＃ママ］");
        assert_eq!(dn.kind, DirectiveKind::Sic);
    }

    #[test]
    fn warichu_round_trip() {
        let mut a = OwnedAllocator::new();
        let upper = a.content_plain("上");
        let lower = a.content_plain("下");
        let NodeOwned::Warichu(w) = a.warichu(upper, lower) else {
            panic!("expected Warichu");
        };
        let ContentOwned::Plain(u) = w.upper else {
            panic!("upper not plain");
        };
        let ContentOwned::Plain(l) = w.lower else {
            panic!("lower not plain");
        };
        assert_eq!(a.store().resolve_str(u), "上");
        assert_eq!(a.store().resolve_str(l), "下");
    }

    #[test]
    fn heading_round_trip() {
        let mut a = OwnedAllocator::new();
        let text = a.content_plain("第一章");
        let NodeOwned::Heading(h) =
            a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, text)
        else {
            panic!("expected Heading");
        };
        assert_eq!(h.kind, HeadingKind::Large);
        assert_eq!(h.style, HeadingStyle::Standard);
        assert_eq!(plain(&a, h.text), Some("第一章"));
    }

    #[test]
    fn heading_hint_round_trip() {
        let mut a = OwnedAllocator::new();
        let NodeOwned::HeadingHint(h) =
            a.heading_hint(HeadingKind::Medium, HeadingStyle::Window, "序章")
        else {
            panic!("expected HeadingHint");
        };
        assert_eq!(h.level, HeadingKind::Medium);
        assert_eq!(h.style, HeadingStyle::Window);
        assert_eq!(a.store().resolve_str(h.target), "序章");
    }

    #[test]
    fn sashie_keyword_form() {
        let mut a = OwnedAllocator::new();
        let caption = a.content_plain("図一");
        let NodeOwned::Illustration(s) =
            a.sashie("cover.png", Some("1"), Some("横100×縦200"), Some(caption))
        else {
            panic!("expected Illustration");
        };
        assert_eq!(a.store().resolve_str(s.file), "cover.png");
        assert_eq!(s.number.map(|id| a.store().resolve_str(id)), Some("1"));
        assert_eq!(
            s.dimensions.map(|id| a.store().resolve_str(id)),
            Some("横100×縦200")
        );
        assert!(s.description.is_none());
        let Some(ContentOwned::Plain(cap)) = s.caption else {
            panic!("expected a plain caption");
        };
        assert_eq!(a.store().resolve_str(cap), "図一");
    }

    #[test]
    fn sashie_general_form() {
        let mut a = OwnedAllocator::new();
        let NodeOwned::Illustration(s) = a.sashie_general("fig.png", "キャラクターの図", None)
        else {
            panic!("expected Illustration");
        };
        assert_eq!(a.store().resolve_str(s.file), "fig.png");
        assert_eq!(
            s.description.map(|id| a.store().resolve_str(id)),
            Some("キャラクターの図")
        );
        assert!(s.number.is_none());
        assert!(s.caption.is_none());
    }

    #[test]
    fn kaeriten_round_trip() {
        let mut a = OwnedAllocator::new();
        let NodeOwned::Kaeriten(k) = a.kaeriten("（レ）") else {
            panic!("expected Kaeriten");
        };
        assert_eq!(a.store().resolve_str(k.mark), "（レ）");
    }

    #[test]
    fn angle_quote_round_trip() {
        let mut a = OwnedAllocator::new();
        let content = a.content_plain("重要");
        let NodeOwned::AngleQuote(d) = a.angle_quote(content) else {
            panic!("expected AngleQuote");
        };
        assert_eq!(plain(&a, d.content), Some("重要"));
    }

    #[test]
    fn unit_leaf_constructors() {
        let a = OwnedAllocator::new();
        assert_eq!(a.page_break(), NodeOwned::PageBreak);
        assert_eq!(a.body_end(), NodeOwned::BodyEnd);
        assert_eq!(a.forced_break(), NodeOwned::ForcedBreak);
        assert_eq!(
            a.section_break(SectionKind::Kaicho),
            NodeOwned::SectionBreak(SectionKind::Kaicho)
        );
        assert_eq!(
            a.line(LineFormat::Indent { amount: 2 }),
            NodeOwned::Line(LineFormat::Indent { amount: 2 })
        );
    }

    #[test]
    fn empty_content_canonicalises_to_empty_segments() {
        let mut a = OwnedAllocator::new();
        assert!(is_empty_content(a.content_plain("")));
        assert!(is_empty_content(a.content_segments(&[])));
    }

    #[test]
    fn content_segments_collapses_all_text() {
        let mut a = OwnedAllocator::new();
        let s1 = a.seg_text("青");
        let s2 = a.seg_text("空");
        // All-Text input collapses to a single concatenated Plain.
        let ContentOwned::Plain(id) = a.content_segments(&[s1, s2]) else {
            panic!("all-text content_segments must collapse to Plain");
        };
        assert_eq!(a.store().resolve_str(id), "青空");
    }

    #[test]
    #[should_panic(expected = "non-empty base")]
    fn ruby_empty_base_panics() {
        let mut a = OwnedAllocator::new();
        let base = a.content_plain("");
        let reading = a.content_plain("おうめ");
        let _ = a.ruby(base, reading);
    }
}
