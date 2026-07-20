//! Owned AST construction.
//!
//! [`Allocator`] builds [`Node`] and its payload types into an owned
//! [`NodeStore`] (a string interner plus the flat
//! content / segment pools). Byte-equal strings share a single interned
//! [`StrId`](super::ast::StrId).
//!
//! ## Canonicalisation
//!
//! The builders apply these canonicalisations so the produced tree resolves to
//! a single byte-identical form:
//!
//! - `content_plain("")` and `content_segments(&[])` both canonicalise to the
//!   empty-segments form (an empty [`SegRange`](super::ast::SegRange)).
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

use crate::encoding::gaiji::GaijiCanonical;

use crate::syntax::format::{ForwardAttr, ForwardOrigin, LineFormat};
use crate::syntax::{
    BoutenKind, BoutenPosition, DirectiveKind, HeadingKind, HeadingStyle, MarginNoteKind, RubySide,
    SectionKind,
};

use super::ast::{
    AngleQuote, Content, ContentRange, Directive, ForwardFormat, ForwardPayload, Gaiji,
    GaijiCanonicalOwned, Heading, HeadingHint, Illustration, Kaeriten, MarginNote, Node, NodeStore,
    Ruby, Segment,
};

/// `true` for the canonical empty-content form (an empty segment run).
fn is_empty_content(c: Content) -> bool {
    matches!(c, Content::Segments(r) if r.len == 0)
}

/// Summed byte length of the `Text` runs in `segs`, for pre-sizing the
/// all-`Text` collapse buffer in [`Allocator::content_segments`].
///
/// `mutants::skip`: this value feeds only `String::with_capacity`, an advisory
/// reservation. Any mutation here changes the reservation but never the bytes
/// pushed, and the buffer is re-interned by value — so the resulting `StrId` is
/// byte-identical regardless and no test can observe the difference
/// (equivalent mutant).
#[cfg_attr(test, mutants::skip)]
fn text_run_len_hint(store: &NodeStore, segs: &[Segment]) -> usize {
    segs.iter()
        .map(|s| match s {
            Segment::Text(id) => store.resolve_str(*id).len(),
            _ => 0,
        })
        .sum()
}

/// Builder for [`Node`] and its payload types.
///
/// Owns the [`NodeStore`] every produced handle resolves against; call
/// [`Self::into_store`] at the end of a parse to hand it to the lex output.
#[derive(Debug, Default)]
pub(crate) struct Allocator {
    store: NodeStore,
}

#[expect(
    clippy::unused_self,
    reason = "every builder takes &(mut) self even when it is a pure wrapper, so call sites have a uniform shape."
)]
impl Allocator {
    /// New allocator backed by an empty [`NodeStore`].
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Borrow the backing store (e.g. to resolve a handle mid-build).
    #[must_use]
    pub(crate) fn store(&self) -> &NodeStore {
        &self.store
    }

    /// Finish allocation and return the owning [`NodeStore`] so the caller can
    /// move it into the lex output and inspect its interner dedup counters.
    #[must_use]
    pub(crate) fn into_store(self) -> NodeStore {
        self.store
    }

    // ---------------------------------------------------------------------
    // Content / segment builders
    // ---------------------------------------------------------------------

    /// Build a plain-text body content. Empty input canonicalises to the
    /// empty-segments form.
    pub(crate) fn content_plain(&mut self, s: &str) -> Content {
        if s.is_empty() {
            Content::Segments(self.store.push_segments(&[]))
        } else {
            Content::Plain(self.store.intern(s))
        }
    }

    /// Build a body content from a sequence of segments. Empty input → the
    /// empty-segments form; all-`Text` input collapses into a single
    /// concatenated `Plain` (re-interned).
    pub(crate) fn content_segments(&mut self, segs: &[Segment]) -> Content {
        if segs.is_empty() {
            return Content::Segments(self.store.push_segments(&[]));
        }
        if segs.iter().all(|s| matches!(s, Segment::Text(_))) {
            // Resolve each text run, concatenate, and re-intern the all-`Text`
            // collapse. Pre-size the buffer from the resolved lengths to avoid
            // reallocation.
            let mut buf = String::with_capacity(text_run_len_hint(&self.store, segs));
            for s in segs {
                if let Segment::Text(id) = s {
                    buf.push_str(self.store.resolve_str(*id));
                }
            }
            return Content::Plain(self.store.intern(&buf));
        }
        Content::Segments(self.store.push_segments(segs))
    }

    /// `Segment::Text(s)` — interns the string.
    pub(crate) fn seg_text(&mut self, s: &str) -> Segment {
        Segment::Text(self.store.intern(s))
    }

    /// `Segment::Gaiji(g)` — wraps a payload built via [`Self::make_gaiji`].
    #[must_use]
    pub(crate) fn seg_gaiji(&self, g: Gaiji) -> Segment {
        Segment::Gaiji(g)
    }

    /// `Segment::Directive(a)` — wraps a payload built via [`Self::make_directive`].
    #[must_use]
    pub(crate) fn seg_annotation(&self, a: Directive) -> Segment {
        Segment::Directive(a)
    }

    // ---------------------------------------------------------------------
    // Payload builders (used by both Segment and Node constructors)
    // ---------------------------------------------------------------------

    /// Build a [`Gaiji`] payload. `mencode` is classified into its
    /// [`GaijiCanonicalOwned`] form; the verbatim tail is interned for the
    /// `Unresolved` arm. The resolved glyph is derived on demand.
    pub(crate) fn make_gaiji(
        &mut self,
        description: &str,
        mencode: Option<&str>,
        standalone: bool,
    ) -> Gaiji {
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
        Gaiji {
            hint,
            canonical,
            mencode_separator: true,
            standalone,
        }
    }

    /// Build a [`Directive`] payload.
    ///
    /// # Panics
    ///
    /// Panics if `raw` is empty (the `NonEmptyStr` contract the classify stage
    /// upholds before emitting an annotation).
    pub(crate) fn make_directive(&mut self, raw: &str, kind: DirectiveKind) -> Directive {
        assert!(
            !raw.is_empty(),
            "classify stage must emit Directive with non-empty raw bytes"
        );
        Directive {
            raw: self.store.intern(raw),
            kind,
        }
    }

    /// Push a `NonEmpty<Content>` field as a length-1 [`ContentRange`],
    /// upholding the non-empty contract.
    fn push_nonempty(&mut self, c: Content, msg: &'static str) -> ContentRange {
        assert!(!is_empty_content(c), "{msg}");
        self.store.push_contents(&[c])
    }

    // ---------------------------------------------------------------------
    // Node variant constructors (one per Node variant)
    // ---------------------------------------------------------------------

    /// `Node::Ruby(Ruby { base, reading, side: Right })`.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `reading` is empty.
    pub(crate) fn ruby(&mut self, base: Content, reading: Content) -> Node {
        let base = self.push_nonempty(base, "classify stage must emit Ruby with non-empty base");
        let reading = self.push_nonempty(
            reading,
            "classify stage must emit Ruby with non-empty reading",
        );
        Node::Ruby(Ruby {
            base,
            reading,
            side: RubySide::Right,
            base_emphasis: None,
        })
    }

    /// `Node::Ruby(Ruby { side: Left, … })` — a left-side ruby.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `reading` is empty.
    pub(crate) fn left_ruby(&mut self, base: Content, reading: Content) -> Node {
        let base = self.push_nonempty(base, "classify stage must emit Ruby with non-empty base");
        let reading = self.push_nonempty(
            reading,
            "classify stage must emit Ruby with non-empty reading",
        );
        Node::Ruby(Ruby {
            base,
            reading,
            side: RubySide::Left,
            base_emphasis: None,
        })
    }

    /// `Node::MarginNote(MarginNote { kind, base, note })`.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `note` is empty.
    pub(crate) fn side_note(&mut self, kind: MarginNoteKind, base: Content, note: Content) -> Node {
        let base = self.push_nonempty(
            base,
            "classify stage must emit MarginNote with non-empty base",
        );
        let note = self.push_nonempty(
            note,
            "classify stage must emit MarginNote with non-empty note",
        );
        Node::MarginNote(MarginNote { kind, base, note })
    }

    /// `Node::Format` with a 傍点 attribute.
    ///
    /// # Panics
    ///
    /// Panics if `target` is empty.
    #[expect(
        clippy::too_many_arguments,
        reason = "every parameter is part of the bouten contract — kind / target / position / origin each carry independent semantics."
    )]
    pub(crate) fn bouten(
        &mut self,
        kind: BoutenKind,
        target: Content,
        position: BoutenPosition,
        origin: ForwardOrigin,
    ) -> Node {
        self.forward_format(ForwardAttr::Bouten { kind, position }, target, origin)
    }

    pub(crate) fn bouten_range(
        &mut self,
        kind: BoutenKind,
        target: Content,
        position: BoutenPosition,
    ) -> Node {
        self.forward_format_nested(
            ForwardAttr::Bouten { kind, position },
            target,
            ForwardOrigin::Reclaimed,
        )
    }

    /// `Node::Format` with a 縦中横 attribute.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    #[cfg(test)]
    pub(crate) fn tate_chu_yoko(&mut self, text: Content, origin: ForwardOrigin) -> Node {
        self.forward_format(ForwardAttr::CombineUpright, text, origin)
    }

    /// `Node::Format` with the given forward-scope `attr` applied to `text`.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    pub(crate) fn forward_format(
        &mut self,
        attr: ForwardAttr,
        text: Content,
        origin: ForwardOrigin,
    ) -> Node {
        let target = self.push_nonempty(
            text,
            "classify stage must emit a forward format with a non-empty target",
        );
        Node::Format(ForwardFormat {
            attr,
            target,
            origin,
            payload: ForwardPayload::None,
        })
    }

    pub(crate) fn forward_format_nested(
        &mut self,
        attr: ForwardAttr,
        text: Content,
        origin: ForwardOrigin,
    ) -> Node {
        let target = self.push_nonempty(
            text,
            "classify stage must emit a forward format with a non-empty target",
        );
        Node::Format(ForwardFormat {
            attr,
            target,
            origin,
            payload: ForwardPayload::NestedSource,
        })
    }

    /// `Node::Format` for a #331 dotted-letter directive
    /// ([`ForwardAttr::AccentDot`]): decorates the reclaimed `text` run and
    /// interns the raw directive `body` (the selector grammar) so the renderer
    /// can compose the dots and the serializer can re-emit it verbatim.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    pub(crate) fn accent_dot(&mut self, text: Content, body: &str, origin: ForwardOrigin) -> Node {
        let target = self.push_nonempty(
            text,
            "classify stage must emit an accent-dot format with a non-empty target",
        );
        let payload = ForwardPayload::AccentBody(self.store.intern(body));
        Node::Format(ForwardFormat {
            attr: ForwardAttr::AccentDot,
            target,
            origin,
            payload,
        })
    }

    /// `Node::Gaiji(g)`.
    #[must_use]
    pub(crate) fn gaiji(&self, g: Gaiji) -> Node {
        Node::Gaiji(g)
    }

    /// `Node::Line(lf)` — a single-line layout directive.
    #[must_use]
    pub(crate) fn line(&self, lf: LineFormat) -> Node {
        Node::Line(lf)
    }

    /// `Node::PageBreak`.
    #[must_use]
    pub(crate) fn page_break(&self) -> Node {
        Node::PageBreak
    }

    /// `Node::SectionBreak(k)`.
    #[must_use]
    pub(crate) fn section_break(&self, k: SectionKind) -> Node {
        Node::SectionBreak(k)
    }

    /// `Node::BodyEnd`.
    #[must_use]
    pub(crate) fn body_end(&self) -> Node {
        Node::BodyEnd
    }

    /// `Node::ForcedBreak`.
    #[must_use]
    pub(crate) fn forced_break(&self) -> Node {
        Node::ForcedBreak
    }

    /// `Node::Heading(Heading { kind, style, text })`.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    pub(crate) fn aozora_heading(
        &mut self,
        kind: HeadingKind,
        style: HeadingStyle,
        text: Content,
    ) -> Node {
        let text = self.push_nonempty(text, "classify stage must emit Heading with non-empty text");
        Node::Heading(Heading { kind, style, text })
    }

    /// `Node::HeadingHint(HeadingHint { level, style, target, self_contained })`.
    ///
    /// `self_contained` is set when the quoted target has no referent in the
    /// preceding source (a no-referent forward heading), so render shows the
    /// target as the heading text while serialize stays bracket-only.
    ///
    /// # Panics
    ///
    /// Panics if `target` is empty.
    #[expect(
        clippy::too_many_arguments,
        reason = "every parameter is an independent part of the 見出し指定 contract — level / style / target / self_contained."
    )]
    pub(crate) fn heading_hint(
        &mut self,
        level: HeadingKind,
        style: HeadingStyle,
        target: &str,
        self_contained: bool,
    ) -> Node {
        assert!(
            !target.is_empty(),
            "classify stage must emit HeadingHint with non-empty target"
        );
        Node::HeadingHint(HeadingHint {
            level,
            style,
            target: self.store.intern(target),
            self_contained,
        })
    }

    /// `Node::Illustration` — keyword 挿絵 form.
    ///
    /// # Panics
    ///
    /// Panics if `file` is empty.
    #[expect(
        clippy::too_many_arguments,
        reason = "every parameter is an independent part of the 挿絵 contract — file / number / dimensions / caption."
    )]
    pub(crate) fn sashie(
        &mut self,
        file: &str,
        number: Option<&str>,
        dimensions: Option<&str>,
        caption: Option<Content>,
    ) -> Node {
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
        Node::Illustration(Illustration {
            file,
            number,
            dimensions,
            caption,
            description: None,
        })
    }

    /// `Node::Illustration` — general image form (leading description, no
    /// 挿絵 keyword / number / caption).
    ///
    /// # Panics
    ///
    /// Panics if `file` or `description` is empty.
    pub(crate) fn sashie_general(
        &mut self,
        file: &str,
        description: &str,
        dimensions: Option<&str>,
    ) -> Node {
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
        Node::Illustration(Illustration {
            file,
            number: None,
            dimensions,
            caption: None,
            description: Some(description),
        })
    }

    /// `Node::Kaeriten(Kaeriten { mark })`.
    ///
    /// # Panics
    ///
    /// Panics if `mark` is empty.
    pub(crate) fn kaeriten(&mut self, mark: &str) -> Node {
        assert!(
            !mark.is_empty(),
            "classify stage must emit Kaeriten with non-empty mark"
        );
        Node::Kaeriten(Kaeriten {
            mark: self.store.intern(mark),
        })
    }

    /// `Node::Directive(a)`.
    #[must_use]
    pub(crate) fn annotation(&self, a: Directive) -> Node {
        Node::Directive(a)
    }

    /// `Node::AngleQuote(AngleQuote { content })`.
    ///
    /// # Panics
    ///
    /// Panics if `content` is empty.
    pub(crate) fn angle_quote(&mut self, content: Content) -> Node {
        let content = self.push_nonempty(
            content,
            "classify stage pre-filters empty AngleQuote into plain",
        );
        Node::AngleQuote(AngleQuote { content })
    }
}

#[cfg(test)]
mod tests {
    //! Per-variant round-trip tests for `Allocator`. Each builds one
    //! `Node` and resolves its payloads against the store to confirm the
    //! fields match.

    use crate::encoding::gaiji::MenKuTen;

    use super::*;

    /// Resolve a length-1 `ContentRange` to its `Plain` text, or `None` for a
    /// mixed / multi-entry run.
    fn plain(alloc: &Allocator, range: ContentRange) -> Option<&str> {
        alloc.store().content_range_as_plain(range)
    }

    #[test]
    fn ruby_round_trip() {
        let mut a = Allocator::new();
        let base = a.content_plain("青梅");
        let reading = a.content_plain("おうめ");
        let n = a.ruby(base, reading);
        let Node::Ruby(r) = n else {
            panic!("expected Ruby, got {n:?}");
        };
        assert_eq!(plain(&a, r.base), Some("青梅"));
        assert_eq!(plain(&a, r.reading), Some("おうめ"));
        assert_eq!(r.side, RubySide::Right);
    }

    #[test]
    fn left_ruby_is_left_side() {
        let mut a = Allocator::new();
        let base = a.content_plain("未");
        let reading = a.content_plain("ザル");
        let Node::Ruby(r) = a.left_ruby(base, reading) else {
            panic!("expected Ruby");
        };
        assert_eq!(r.side, RubySide::Left);
    }

    #[test]
    fn bouten_round_trip() {
        let mut a = Allocator::new();
        let target = a.content_plain("青空");
        let n = a.bouten(
            BoutenKind::Goma,
            target,
            BoutenPosition::Right,
            ForwardOrigin::Referenced,
        );
        let Node::Format(b) = n else {
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
        let mut a = Allocator::new();
        let text = a.content_plain("12");
        let Node::Format(t) = a.tate_chu_yoko(text, ForwardOrigin::Referenced) else {
            panic!("expected Format(CombineUpright)");
        };
        assert_eq!(t.attr, ForwardAttr::CombineUpright);
        assert_eq!(plain(&a, t.target), Some("12"));
    }

    #[test]
    fn emphasis_round_trip() {
        let mut a = Allocator::new();
        let text = a.content_plain("重要");
        let Node::Format(e) = a.forward_format(ForwardAttr::Bold, text, ForwardOrigin::Reclaimed)
        else {
            panic!("expected Format(Bold)");
        };
        assert_eq!(e.attr, ForwardAttr::Bold);
        assert_eq!(plain(&a, e.target), Some("重要"));
        assert_eq!(e.origin, ForwardOrigin::Reclaimed);
    }

    #[test]
    fn side_note_round_trip() {
        let mut a = Allocator::new();
        let base = a.content_plain("未来");
        let note = a.content_plain("みらい");
        let Node::MarginNote(s) = a.side_note(MarginNoteKind::Gloss, base, note) else {
            panic!("expected MarginNote");
        };
        assert_eq!(s.kind, MarginNoteKind::Gloss);
        assert_eq!(plain(&a, s.base), Some("未来"));
        assert_eq!(plain(&a, s.note), Some("みらい"));
    }

    #[test]
    fn gaiji_full_metadata() {
        let mut a = Allocator::new();
        let g = a.make_gaiji("木＋吶のつくり", Some("第3水準1-85-54"), false);
        let Node::Gaiji(gn) = a.gaiji(g) else {
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
        let mut a = Allocator::new();
        let g = a.make_gaiji("desc", None, true);
        let Node::Gaiji(gn) = a.gaiji(g) else {
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
        let mut a = Allocator::new();
        let g = a.make_gaiji("謎", Some("未知の注記"), false);
        let Node::Gaiji(gn) = a.gaiji(g) else {
            panic!("expected Gaiji");
        };
        let GaijiCanonicalOwned::Unresolved { mencode: Some(id) } = gn.canonical else {
            panic!("expected Unresolved with a tail, got {:?}", gn.canonical);
        };
        assert_eq!(a.store().resolve_str(id), "未知の注記");
    }

    #[test]
    fn directive_round_trip() {
        let mut a = Allocator::new();
        let d = a.make_directive("［＃ママ］", DirectiveKind::Sic);
        let Node::Directive(dn) = a.annotation(d) else {
            panic!("expected Directive");
        };
        assert_eq!(a.store().resolve_str(dn.raw), "［＃ママ］");
        assert_eq!(dn.kind, DirectiveKind::Sic);
    }

    #[test]
    fn heading_round_trip() {
        let mut a = Allocator::new();
        let text = a.content_plain("第一章");
        let Node::Heading(h) = a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, text)
        else {
            panic!("expected Heading");
        };
        assert_eq!(h.kind, HeadingKind::Large);
        assert_eq!(h.style, HeadingStyle::Standard);
        assert_eq!(plain(&a, h.text), Some("第一章"));
    }

    #[test]
    fn heading_hint_round_trip() {
        let mut a = Allocator::new();
        let Node::HeadingHint(h) =
            a.heading_hint(HeadingKind::Medium, HeadingStyle::Window, "序章", false)
        else {
            panic!("expected HeadingHint");
        };
        assert_eq!(h.level, HeadingKind::Medium);
        assert_eq!(h.style, HeadingStyle::Window);
        assert_eq!(a.store().resolve_str(h.target), "序章");
        assert!(!h.self_contained);
    }

    #[test]
    fn sashie_keyword_form() {
        let mut a = Allocator::new();
        let caption = a.content_plain("図一");
        let Node::Illustration(s) =
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
        let Some(Content::Plain(cap)) = s.caption else {
            panic!("expected a plain caption");
        };
        assert_eq!(a.store().resolve_str(cap), "図一");
    }

    #[test]
    fn sashie_general_form() {
        let mut a = Allocator::new();
        let Node::Illustration(s) = a.sashie_general("fig.png", "キャラクターの図", None)
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
        let mut a = Allocator::new();
        let Node::Kaeriten(k) = a.kaeriten("（レ）") else {
            panic!("expected Kaeriten");
        };
        assert_eq!(a.store().resolve_str(k.mark), "（レ）");
    }

    #[test]
    fn angle_quote_round_trip() {
        let mut a = Allocator::new();
        let content = a.content_plain("重要");
        let Node::AngleQuote(d) = a.angle_quote(content) else {
            panic!("expected AngleQuote");
        };
        assert_eq!(plain(&a, d.content), Some("重要"));
    }

    #[test]
    fn unit_leaf_constructors() {
        let a = Allocator::new();
        assert_eq!(a.page_break(), Node::PageBreak);
        assert_eq!(a.body_end(), Node::BodyEnd);
        assert_eq!(a.forced_break(), Node::ForcedBreak);
        assert_eq!(
            a.section_break(SectionKind::Kaicho),
            Node::SectionBreak(SectionKind::Kaicho)
        );
        assert_eq!(
            a.line(LineFormat::Indent {
                amount: 2,
                end_offset: None
            }),
            Node::Line(LineFormat::Indent {
                amount: 2,
                end_offset: None
            })
        );
    }

    #[test]
    fn empty_content_canonicalises_to_empty_segments() {
        let mut a = Allocator::new();
        assert!(is_empty_content(a.content_plain("")));
        assert!(is_empty_content(a.content_segments(&[])));
    }

    #[test]
    fn content_segments_collapses_all_text() {
        let mut a = Allocator::new();
        let s1 = a.seg_text("青");
        let s2 = a.seg_text("空");
        // All-Text input collapses to a single concatenated Plain.
        let Content::Plain(id) = a.content_segments(&[s1, s2]) else {
            panic!("all-text content_segments must collapse to Plain");
        };
        assert_eq!(a.store().resolve_str(id), "青空");
    }

    #[test]
    fn into_store_hands_over_the_populated_store() {
        // `into_store` must yield the *built* store (not a fresh default): a
        // handle minted during the build has to still resolve against it.
        let mut a = Allocator::new();
        let Content::Plain(id) = a.content_plain("青梅") else {
            panic!("plain content should intern to a Plain handle");
        };
        let store = a.into_store();
        assert_eq!(
            store.resolve_str(id),
            "青梅",
            "into_store must return the populated store, not Default::default()"
        );
    }

    #[test]
    #[should_panic(expected = "non-empty base")]
    fn ruby_empty_base_panics() {
        let mut a = Allocator::new();
        let base = a.content_plain("");
        let reading = a.content_plain("おうめ");
        let _ = a.ruby(base, reading);
    }
}
