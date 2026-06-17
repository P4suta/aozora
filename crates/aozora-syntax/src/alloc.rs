//! Arena-backed AST construction.
//!
//! [`BorrowedAllocator<'a>`] is the sole AST builder for the
//! [`crate::borrowed`] AST. It owns an [`Interner`] so byte-equal
//! strings (ruby readings, container labels, kaeriten marks) share a
//! single arena allocation.
//!
//! ## Naming convention
//!
//! - `make_*` methods build *payload* references (`&'a Gaiji<'a>`,
//!   `&'a Annotation<'a>`) without wrapping them in a node.
//! - Variant-named methods (`ruby`, `bouten`, `gaiji`, …) build the
//!   final [`borrowed::AozoraNode<'a>`]. The `gaiji` and `annotation`
//!   node constructors take the payload reference (built via
//!   `make_gaiji` / `make_annotation`) so a payload can be shared
//!   between a `Segment` and a `Node` without recomputing the string
//!   interns.
//! - `seg_*` methods build segment elements for `content_segments`.
//!
//! ## Canonicalisation
//!
//! Both `content_plain("")` and `content_segments(&[])` return
//! [`borrowed::Content::EMPTY`] (i.e. `Segments(&[])`). `content_segments`
//! collapses an all-`Text` input into a single concatenated `Plain`
//! (the concatenation is interned). The legacy owned `Content::from`
//! / `Content::from_segments` helpers used the same canonicalisation;
//! preserving it keeps the determinism + sentinel-alignment
//! proptests in `aozora-lex/tests/property_borrowed_arena.rs` honest
//! across edits.

use aozora_encoding::gaiji::Resolved;

use crate::borrowed::{self, Arena, Interner};
use crate::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, AozoraHeadingStyle, BoutenKind, BoutenPosition,
    Center, Container, EmphasisKind, Indent, Keigakomi, RubySide, SectionKind,
};

/// Arena-backed builder for [`borrowed::AozoraNode<'a>`] and its
/// payload types.
///
/// Owns an [`Interner`] keyed off the supplied [`Arena`]; both string
/// content and per-variant payloads land in the arena, so dropping the
/// arena tears the entire AST down in one step (no per-node `Drop`
/// runs, no `Box::drop` traffic).
#[derive(Debug)]
pub struct BorrowedAllocator<'a> {
    arena: &'a Arena,
    interner: Interner<'a>,
}

#[allow(
    clippy::unused_self,
    reason = "API consistency: every BorrowedAllocator builder method takes &mut self even when the variant is a pure wrapper, so call sites have a uniform shape (alloc.method(...) for every variant). Switching trivial wrappers to free fns would split the API in half."
)]
impl<'a> BorrowedAllocator<'a> {
    /// New allocator with a fresh interner sized to `interner_capacity`.
    /// Capacity is rounded up to the next power of two by the interner.
    #[must_use]
    pub fn with_capacity(arena: &'a Arena, interner_capacity: usize) -> Self {
        Self {
            arena,
            interner: Interner::with_capacity_in(interner_capacity, arena),
        }
    }

    /// Construct with the interner's default initial capacity (64 → 64
    /// after power-of-two rounding).
    #[must_use]
    pub fn new(arena: &'a Arena) -> Self {
        Self::with_capacity(arena, 64)
    }

    /// Borrow the underlying arena. Useful for callers that need to
    /// emit an arena-allocated normalised text alongside the AST.
    #[must_use]
    pub fn arena(&self) -> &'a Arena {
        self.arena
    }

    /// Finish allocation and return the interner so the caller can
    /// inspect its dedup counters (cache hits, table hits, allocs,
    /// average probe length). The interner's `&'a` arena reference
    /// continues to keep the interned strings alive.
    #[must_use]
    pub fn into_interner(self) -> Interner<'a> {
        self.interner
    }

    // ---------------------------------------------------------------------
    // Content / segment builders
    // ---------------------------------------------------------------------

    /// Build a plain-text body content. Empty input canonicalises to
    /// `Segments(&[])` (the legacy owned shape did the same).
    pub fn content_plain(&mut self, s: &str) -> borrowed::Content<'a> {
        if s.is_empty() {
            borrowed::Content::EMPTY
        } else {
            borrowed::Content::Plain(self.interner.intern(s))
        }
    }

    /// Build a body content from a sequence of segments. Empty input →
    /// `Segments(&[])`; all-`Text` input collapses into a single
    /// concatenated `Plain` (interned).
    pub fn content_segments(&mut self, segs: &[borrowed::Segment<'a>]) -> borrowed::Content<'a> {
        if segs.is_empty() {
            return borrowed::Content::EMPTY;
        }
        if segs.iter().all(|s| matches!(s, borrowed::Segment::Text(_))) {
            // Total length is known (sum of text lengths) so we can
            // pre-size the buffer and avoid reallocation.
            let total: usize = segs
                .iter()
                .map(|s| match s {
                    borrowed::Segment::Text(t) => t.len(),
                    _ => 0,
                })
                .sum();
            let mut buf = String::with_capacity(total);
            for s in segs {
                if let borrowed::Segment::Text(t) = s {
                    buf.push_str(t);
                }
            }
            return borrowed::Content::Plain(self.interner.intern(&buf));
        }
        borrowed::Content::Segments(self.arena.alloc_slice_copy(segs))
    }

    /// `Segment::Text(s)` — interns the string.
    pub fn seg_text(&mut self, s: &str) -> borrowed::Segment<'a> {
        borrowed::Segment::Text(self.interner.intern(s))
    }

    /// `Segment::Gaiji(g)` — wraps a payload built via [`Self::make_gaiji`].
    #[must_use]
    pub fn seg_gaiji(&self, g: &'a borrowed::Gaiji<'a>) -> borrowed::Segment<'a> {
        borrowed::Segment::Gaiji(g)
    }

    /// `Segment::Annotation(a)` — wraps a payload built via [`Self::make_annotation`].
    #[must_use]
    pub fn seg_annotation(&self, a: &'a borrowed::Annotation<'a>) -> borrowed::Segment<'a> {
        borrowed::Segment::Annotation(a)
    }

    // ---------------------------------------------------------------------
    // Payload builders (used by both Segment and Node constructors)
    // ---------------------------------------------------------------------

    /// Build a `Gaiji` payload. Use [`Self::seg_gaiji`] to wrap as a
    /// segment, or [`Self::gaiji`] to wrap as a node.
    pub fn make_gaiji(
        &mut self,
        description: &str,
        ucs: Option<Resolved>,
        mencode: Option<&str>,
    ) -> &'a borrowed::Gaiji<'a> {
        let g = borrowed::Gaiji {
            description: self.interner.intern(description),
            ucs,
            mencode: mencode.map(|s| self.interner.intern(s)),
        };
        self.arena.alloc(g)
    }

    /// Build an `Annotation` payload. Use [`Self::seg_annotation`] to
    /// wrap as a segment, or [`Self::annotation`] to wrap as a node.
    ///
    /// `raw` carries the [`borrowed::NonEmptyStr`] invariant.
    ///
    /// # Panics
    ///
    /// Panics if `raw` is empty. Phase 3 emits annotation only after
    /// at least one byte landed in the bracket body.
    pub fn make_annotation(
        &mut self,
        raw: &str,
        kind: AnnotationKind,
    ) -> &'a borrowed::Annotation<'a> {
        let raw = borrowed::NonEmptyStr::new(self.interner.intern(raw))
            .expect("Phase 3 must emit Annotation with non-empty raw bytes");
        let a = borrowed::Annotation { raw, kind };
        self.arena.alloc(a)
    }

    // ---------------------------------------------------------------------
    // Node variant constructors (18 — matches the AozoraNode enum)
    // ---------------------------------------------------------------------

    /// `AozoraNode::Ruby(Ruby { base, reading, delim_explicit })`.
    ///
    /// `base` and `reading` carry the [`borrowed::NonEmpty`]
    /// invariant. Phase 3 only emits Ruby once both are non-empty,
    /// so this `expect` is a contract-check; an empty payload here
    /// signals a classifier bug.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `reading` is empty. Phase 3 emit-sites
    /// classify only after the body is populated, so the panic
    /// represents a pipeline-internal bug — the
    /// [`borrowed::NonEmpty`] payload encodes this invariant at the
    /// type level.
    #[must_use]
    pub fn ruby(
        &self,
        base: borrowed::Content<'a>,
        reading: borrowed::Content<'a>,
        delim_explicit: bool,
    ) -> borrowed::AozoraNode<'a> {
        let base =
            borrowed::NonEmpty::new(base).expect("Phase 3 must emit Ruby with non-empty base");
        let reading = borrowed::NonEmpty::new(reading)
            .expect("Phase 3 must emit Ruby with non-empty reading");
        borrowed::AozoraNode::Ruby(self.arena.alloc(borrowed::Ruby {
            base,
            reading,
            delim_explicit,
            side: RubySide::Right,
        }))
    }

    /// `AozoraNode::Ruby(Ruby { side: Left, … })` — a left-side ruby from the
    /// `［＃「base」の左に「reading」のルビ］` forward-reference form (the
    /// saidoku-moji 再読文字 building block).
    ///
    /// # Panics
    ///
    /// Panics if `base` or `reading` is empty (same contract as [`Self::ruby`]).
    #[must_use]
    pub fn left_ruby(
        &self,
        base: borrowed::Content<'a>,
        reading: borrowed::Content<'a>,
    ) -> borrowed::AozoraNode<'a> {
        let base =
            borrowed::NonEmpty::new(base).expect("Phase 3 must emit Ruby with non-empty base");
        let reading = borrowed::NonEmpty::new(reading)
            .expect("Phase 3 must emit Ruby with non-empty reading");
        borrowed::AozoraNode::Ruby(self.arena.alloc(borrowed::Ruby {
            base,
            reading,
            delim_explicit: false,
            side: RubySide::Left,
        }))
    }

    /// `AozoraNode::SideNote(SideNote { base, note })` — a left-side
    /// annotation (注記) from the `［＃「base」の左に「note」の注記］`
    /// forward-reference form. Placement parallels a left-side ruby, but a
    /// 注記 is an editorial note rather than a phonetic reading.
    ///
    /// # Panics
    ///
    /// Panics if `base` or `note` is empty (same contract as [`Self::ruby`];
    /// Phase 3 rejects an empty note before emitting).
    #[must_use]
    pub fn side_note(
        &self,
        base: borrowed::Content<'a>,
        note: borrowed::Content<'a>,
    ) -> borrowed::AozoraNode<'a> {
        let base =
            borrowed::NonEmpty::new(base).expect("Phase 3 must emit SideNote with non-empty base");
        let note =
            borrowed::NonEmpty::new(note).expect("Phase 3 must emit SideNote with non-empty note");
        borrowed::AozoraNode::SideNote(self.arena.alloc(borrowed::SideNote { base, note }))
    }

    /// `AozoraNode::Bouten(Bouten { kind, target, position,
    /// consumed_predecessor })`.
    ///
    /// `target` carries the [`borrowed::NonEmpty`] invariant —
    /// Phase 3 resolves the forward reference before emitting.
    ///
    /// `consumed_predecessor` is `true` when the classifier pulled
    /// the node's source span back over the literal occurrence of
    /// `target` that sits immediately before the `［`. See the field
    /// docstring on [`borrowed::Bouten`] for the serializer
    /// round-trip contract that depends on this flag.
    ///
    /// # Panics
    ///
    /// Panics if `target` is empty. The forward-reference resolver
    /// in Phase 3 always lands a non-empty target before emit; an
    /// empty payload here signals a classifier bug.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "every parameter is part of the public bouten contract — kind / target / position / consumed_predecessor each carry independent semantics and grouping them into a builder would add a layer without saving the caller anything"
    )]
    pub fn bouten(
        &self,
        kind: BoutenKind,
        target: borrowed::Content<'a>,
        position: BoutenPosition,
        consumed_predecessor: bool,
    ) -> borrowed::AozoraNode<'a> {
        let target = borrowed::NonEmpty::new(target)
            .expect("Phase 3 must emit Bouten with a resolved non-empty target");
        borrowed::AozoraNode::Bouten(self.arena.alloc(borrowed::Bouten {
            kind,
            target,
            position,
            consumed_predecessor,
        }))
    }

    /// `AozoraNode::TateChuYoko(TateChuYoko { text,
    /// consumed_predecessor })`.
    ///
    /// `text` carries the [`borrowed::NonEmpty`] invariant.
    /// `consumed_predecessor` mirrors [`Self::bouten`]'s flag.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    #[must_use]
    pub fn tate_chu_yoko(
        &self,
        text: borrowed::Content<'a>,
        consumed_predecessor: bool,
    ) -> borrowed::AozoraNode<'a> {
        let text = borrowed::NonEmpty::new(text)
            .expect("Phase 3 must emit TateChuYoko with non-empty text");
        borrowed::AozoraNode::TateChuYoko(self.arena.alloc(borrowed::TateChuYoko {
            text,
            consumed_predecessor,
        }))
    }

    /// `AozoraNode::Emphasis(Emphasis { kind, text, consumed_predecessor })`.
    ///
    /// The forward-reference leaf form of 太字 / 斜体
    /// (`X［＃「X」は太字／斜体］`). `text` carries the
    /// [`borrowed::NonEmpty`] invariant; `consumed_predecessor` mirrors
    /// [`Self::tate_chu_yoko`]'s flag (see [`borrowed::Emphasis`] for the
    /// serializer round-trip contract).
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    #[must_use]
    pub fn emphasis(
        &self,
        kind: EmphasisKind,
        text: borrowed::Content<'a>,
        consumed_predecessor: bool,
    ) -> borrowed::AozoraNode<'a> {
        let text =
            borrowed::NonEmpty::new(text).expect("Phase 3 must emit Emphasis with non-empty text");
        borrowed::AozoraNode::Emphasis(self.arena.alloc(borrowed::Emphasis {
            kind,
            text,
            consumed_predecessor,
        }))
    }

    /// `AozoraNode::Gaiji(g)`.
    #[must_use]
    pub fn gaiji(&self, g: &'a borrowed::Gaiji<'a>) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Gaiji(g)
    }

    /// `AozoraNode::Indent(i)`.
    #[must_use]
    pub fn indent(&self, i: Indent) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Indent(i)
    }

    /// `AozoraNode::AlignEnd(a)`.
    #[must_use]
    pub fn align_end(&self, a: AlignEnd) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::AlignEnd(a)
    }

    /// `AozoraNode::Center(c)`.
    #[must_use]
    pub fn center(&self, c: Center) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Center(c)
    }

    /// `AozoraNode::Warichu(Warichu { upper, lower })`.
    #[must_use]
    pub fn warichu(
        &self,
        upper: borrowed::Content<'a>,
        lower: borrowed::Content<'a>,
    ) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Warichu(self.arena.alloc(borrowed::Warichu { upper, lower }))
    }

    /// `AozoraNode::Keigakomi(k)`.
    #[must_use]
    pub fn keigakomi(&self, k: Keigakomi) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Keigakomi(k)
    }

    /// `AozoraNode::PageBreak`.
    #[must_use]
    pub fn page_break(&self) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::PageBreak
    }

    /// `AozoraNode::SectionBreak(k)`.
    #[must_use]
    pub fn section_break(&self, k: SectionKind) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::SectionBreak(k)
    }

    /// `AozoraNode::AozoraHeading(AozoraHeading { kind, style, text })`.
    ///
    /// `text` carries the [`borrowed::NonEmpty`] invariant.
    ///
    /// # Panics
    ///
    /// Panics if `text` is empty.
    #[must_use]
    pub fn aozora_heading(
        &self,
        kind: AozoraHeadingKind,
        style: AozoraHeadingStyle,
        text: borrowed::Content<'a>,
    ) -> borrowed::AozoraNode<'a> {
        let text = borrowed::NonEmpty::new(text)
            .expect("Phase 3 must emit AozoraHeading with non-empty text");
        borrowed::AozoraNode::AozoraHeading(self.arena.alloc(borrowed::AozoraHeading {
            kind,
            style,
            text,
        }))
    }

    /// `AozoraNode::HeadingHint(HeadingHint { level, style, target })`.
    ///
    /// `target` carries the [`borrowed::NonEmptyStr`] invariant.
    ///
    /// # Panics
    ///
    /// Panics if `target` is empty. Phase 3 emits the hint only
    /// after the forward-reference target lands non-empty; an empty
    /// payload here signals a classifier bug.
    pub fn heading_hint(
        &mut self,
        level: u8,
        style: AozoraHeadingStyle,
        target: &str,
    ) -> borrowed::AozoraNode<'a> {
        let target = borrowed::NonEmptyStr::new(self.interner.intern(target))
            .expect("Phase 3 must emit HeadingHint with non-empty target");
        borrowed::AozoraNode::HeadingHint(self.arena.alloc(borrowed::HeadingHint {
            level,
            style,
            target,
        }))
    }

    /// `AozoraNode::Sashie(Sashie { file, dimensions, caption })`.
    ///
    /// `file` carries the [`borrowed::NonEmptyStr`] invariant.
    /// `dimensions` is the optional verbatim pixel-size note (`横W×縦H`).
    ///
    /// # Panics
    ///
    /// Panics if `file` is empty.
    pub fn sashie(
        &mut self,
        file: &str,
        dimensions: Option<&str>,
        caption: Option<borrowed::Content<'a>>,
    ) -> borrowed::AozoraNode<'a> {
        let file = borrowed::NonEmptyStr::new(self.interner.intern(file))
            .expect("Phase 3 must emit Sashie with non-empty file path");
        let dimensions = dimensions.map(|d| self.interner.intern(d));
        borrowed::AozoraNode::Sashie(self.arena.alloc(borrowed::Sashie {
            file,
            dimensions,
            caption,
        }))
    }

    /// `AozoraNode::Kaeriten(Kaeriten { mark })`.
    ///
    /// `mark` carries the [`borrowed::NonEmptyStr`] invariant.
    ///
    /// # Panics
    ///
    /// Panics if `mark` is empty.
    pub fn kaeriten(&mut self, mark: &str) -> borrowed::AozoraNode<'a> {
        let mark = borrowed::NonEmptyStr::new(self.interner.intern(mark))
            .expect("Phase 3 must emit Kaeriten with non-empty mark");
        borrowed::AozoraNode::Kaeriten(self.arena.alloc(borrowed::Kaeriten { mark }))
    }

    /// `AozoraNode::Annotation(a)`.
    #[must_use]
    pub fn annotation(&self, a: &'a borrowed::Annotation<'a>) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Annotation(a)
    }

    /// `AozoraNode::AngleQuote(AngleQuote { content })`.
    ///
    /// `content` carries the [`borrowed::NonEmpty`] invariant — Phase 3
    /// pre-filters `≪≫` with empty body into plain text so this
    /// path is never reached with an empty payload.
    ///
    /// # Panics
    ///
    /// Panics if `content` is empty. Phase 3's pre-filter is the
    /// gate; an empty payload here signals a classifier bug.
    #[must_use]
    pub fn angle_quote(&self, content: borrowed::Content<'a>) -> borrowed::AozoraNode<'a> {
        let content = borrowed::NonEmpty::new(content)
            .expect("Phase 3 pre-filters empty AngleQuote into plain");
        borrowed::AozoraNode::AngleQuote(self.arena.alloc(borrowed::AngleQuote { content }))
    }

    /// `AozoraNode::Container(c)`.
    #[must_use]
    pub fn container(&self, c: Container) -> borrowed::AozoraNode<'a> {
        borrowed::AozoraNode::Container(c)
    }
}

#[cfg(test)]
mod tests {
    //! Per-variant round-trip tests for `BorrowedAllocator`.
    //!
    //! Each test constructs one `borrowed::AozoraNode<'a>` via the
    //! allocator and asserts the resulting payload fields match what
    //! we asked for. Together they cover all 18 node variants plus
    //! content / segment composition + interner dedup.

    use core::ptr;

    use super::*;
    use crate::borrowed;
    use crate::{
        AlignEnd, AnnotationKind, AozoraHeadingKind, AozoraHeadingStyle, BoutenKind,
        BoutenPosition, Container, ContainerKind, EmphasisKind, Indent, Keigakomi, SectionKind,
    };

    fn fresh_alloc(arena: &Arena) -> BorrowedAllocator<'_> {
        BorrowedAllocator::new(arena)
    }

    #[test]
    fn ruby_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let base = a.content_plain("青梅");
        let reading = a.content_plain("おうめ");
        let n = a.ruby(base, reading, true);
        match n {
            borrowed::AozoraNode::Ruby(r) => {
                assert_eq!(r.base.as_plain(), Some("青梅"));
                assert_eq!(r.reading.as_plain(), Some("おうめ"));
                assert!(r.delim_explicit);
            }
            other => panic!("expected Ruby, got {other:?}"),
        }
    }

    #[test]
    fn bouten_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let target = a.content_plain("青空");
        let n = a.bouten(BoutenKind::Goma, target, BoutenPosition::Right, false);
        match n {
            borrowed::AozoraNode::Bouten(b) => {
                assert_eq!(b.kind, BoutenKind::Goma);
                assert_eq!(b.target.as_plain(), Some("青空"));
                assert_eq!(b.position, BoutenPosition::Right);
                assert!(!b.consumed_predecessor);
            }
            other => panic!("expected Bouten, got {other:?}"),
        }
    }

    #[test]
    fn tate_chu_yoko_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let text = a.content_plain("12");
        let n = a.tate_chu_yoko(text, false);
        match n {
            borrowed::AozoraNode::TateChuYoko(t) => {
                assert_eq!(t.text.as_plain(), Some("12"));
            }
            other => panic!("expected TateChuYoko, got {other:?}"),
        }
    }

    #[test]
    fn emphasis_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let text = a.content_plain("重要");
        let n = a.emphasis(EmphasisKind::Bold, text, true);
        match n {
            borrowed::AozoraNode::Emphasis(e) => {
                assert_eq!(e.kind, EmphasisKind::Bold);
                assert_eq!(e.text.as_plain(), Some("重要"));
                assert!(e.consumed_predecessor);
            }
            other => panic!("expected Emphasis, got {other:?}"),
        }
    }

    #[test]
    fn gaiji_full_metadata() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let g = a.make_gaiji(
            "木＋吶のつくり",
            Some(Resolved::Char('𠀋')),
            Some("第3水準1-85-54"),
        );
        let n = a.gaiji(g);
        match n {
            borrowed::AozoraNode::Gaiji(gn) => {
                assert_eq!(gn.description, "木＋吶のつくり");
                assert_eq!(gn.ucs, Some(Resolved::Char('𠀋')));
                assert_eq!(gn.mencode, Some("第3水準1-85-54"));
            }
            other => panic!("expected Gaiji, got {other:?}"),
        }
    }

    #[test]
    fn gaiji_no_mencode() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let g = a.make_gaiji("desc", None, None);
        let n = a.gaiji(g);
        match n {
            borrowed::AozoraNode::Gaiji(gn) => {
                assert_eq!(gn.description, "desc");
                assert!(gn.ucs.is_none());
                assert!(gn.mencode.is_none());
            }
            other => panic!("expected Gaiji, got {other:?}"),
        }
    }

    #[test]
    fn indent_round_trip() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        let n = a.indent(Indent {
            amount: 3,
            wrap: None,
            head_flush: false,
            from_top: false,
        });
        assert!(matches!(
            n,
            borrowed::AozoraNode::Indent(Indent { amount: 3, .. })
        ));
    }

    #[test]
    fn align_end_round_trip() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        let n = a.align_end(AlignEnd { offset: 2 });
        assert!(matches!(
            n,
            borrowed::AozoraNode::AlignEnd(AlignEnd { offset: 2 })
        ));
    }

    #[test]
    fn warichu_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let upper = a.content_plain("上");
        let lower = a.content_plain("下");
        let n = a.warichu(upper, lower);
        match n {
            borrowed::AozoraNode::Warichu(w) => {
                assert_eq!(w.upper.as_plain(), Some("上"));
                assert_eq!(w.lower.as_plain(), Some("下"));
            }
            other => panic!("expected Warichu, got {other:?}"),
        }
    }

    #[test]
    fn keigakomi_round_trip() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        let n = a.keigakomi(Keigakomi);
        assert!(matches!(n, borrowed::AozoraNode::Keigakomi(Keigakomi)));
    }

    #[test]
    fn page_break_round_trip() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        let n = a.page_break();
        assert!(matches!(n, borrowed::AozoraNode::PageBreak));
    }

    #[test]
    fn section_break_round_trip() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        let n = a.section_break(SectionKind::Kaicho);
        assert!(matches!(
            n,
            borrowed::AozoraNode::SectionBreak(SectionKind::Kaicho)
        ));
    }

    #[test]
    fn aozora_heading_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let text = a.content_plain("見出し");
        let n = a.aozora_heading(AozoraHeadingKind::Medium, AozoraHeadingStyle::Window, text);
        match n {
            borrowed::AozoraNode::AozoraHeading(h) => {
                assert_eq!(h.kind, AozoraHeadingKind::Medium);
                assert_eq!(h.style, AozoraHeadingStyle::Window);
                assert_eq!(h.text.as_plain(), Some("見出し"));
            }
            other => panic!("expected AozoraHeading, got {other:?}"),
        }
    }

    #[test]
    fn heading_hint_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let n = a.heading_hint(2, AozoraHeadingStyle::SameLine, "対象");
        match n {
            borrowed::AozoraNode::HeadingHint(h) => {
                assert_eq!(h.level, 2);
                assert_eq!(h.style, AozoraHeadingStyle::SameLine);
                assert_eq!(h.target.as_str(), "対象");
            }
            other => panic!("expected HeadingHint, got {other:?}"),
        }
    }

    #[test]
    fn sashie_with_caption() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let caption = a.content_plain("挿絵キャプション");
        let n = a.sashie("fig01.png", None, Some(caption));
        match n {
            borrowed::AozoraNode::Sashie(s) => {
                assert_eq!(s.file.as_str(), "fig01.png");
                assert_eq!(
                    s.caption.and_then(borrowed::Content::as_plain),
                    Some("挿絵キャプション")
                );
            }
            other => panic!("expected Sashie, got {other:?}"),
        }
    }

    #[test]
    fn sashie_without_caption() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let n = a.sashie("fig02.png", None, None);
        match n {
            borrowed::AozoraNode::Sashie(s) => {
                assert_eq!(s.file.as_str(), "fig02.png");
                assert!(s.caption.is_none());
            }
            other => panic!("expected Sashie, got {other:?}"),
        }
    }

    #[test]
    fn kaeriten_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let n = a.kaeriten("一");
        match n {
            borrowed::AozoraNode::Kaeriten(k) => assert_eq!(k.mark.as_str(), "一"),
            other => panic!("expected Kaeriten, got {other:?}"),
        }
    }

    #[test]
    fn annotation_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let payload = a.make_annotation("［＃X］", AnnotationKind::Unknown);
        let n = a.annotation(payload);
        match n {
            borrowed::AozoraNode::Annotation(an) => {
                assert_eq!(an.raw.as_str(), "［＃X］");
                assert_eq!(an.kind, AnnotationKind::Unknown);
            }
            other => panic!("expected Annotation, got {other:?}"),
        }
    }

    #[test]
    fn angle_quote_round_trip() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let content = a.content_plain("重要");
        let n = a.angle_quote(content);
        match n {
            borrowed::AozoraNode::AngleQuote(d) => {
                assert_eq!(d.content.as_plain(), Some("重要"));
            }
            other => panic!("expected AngleQuote, got {other:?}"),
        }
    }

    #[test]
    fn container_round_trip() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        let c = Container {
            kind: ContainerKind::Indent {
                amount: 1,
                wrap: None,
                center: false,
                head_flush: false,
            },
        };
        let n = a.container(c);
        assert!(matches!(n, borrowed::AozoraNode::Container(cc) if cc == c));
    }

    // ---------------------------------------------------------------------
    // Content / segment composition (canonicalisation rules)
    // ---------------------------------------------------------------------

    #[test]
    fn content_plain_empty_collapses_to_empty_segments() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let c = a.content_plain("");
        assert!(matches!(c, borrowed::Content::Segments(s) if s.is_empty()));
    }

    #[test]
    fn content_plain_nonempty_returns_plain_variant() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let c = a.content_plain("hello");
        assert_eq!(c.as_plain(), Some("hello"));
    }

    #[test]
    fn content_segments_preserves_order_and_kind() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let g = a.make_gaiji("X", None, None);
        let seg_g = a.seg_gaiji(g);
        let seg_t1 = a.seg_text("before ");
        let seg_t2 = a.seg_text(" after");
        let ann = a.make_annotation("［＃X］", AnnotationKind::Unknown);
        let seg_a = a.seg_annotation(ann);
        let c = a.content_segments(&[seg_t1, seg_g, seg_t2, seg_a]);
        let borrowed::Content::Segments(segs) = c else {
            panic!("expected Segments variant for mixed-kind input");
        };
        assert_eq!(segs.len(), 4);
        assert!(matches!(&segs[0], borrowed::Segment::Text(t) if *t == "before "));
        assert!(matches!(&segs[1], borrowed::Segment::Gaiji(_)));
        assert!(matches!(&segs[2], borrowed::Segment::Text(t) if *t == " after"));
        assert!(matches!(&segs[3], borrowed::Segment::Annotation(_)));
    }

    #[test]
    fn content_segments_all_text_collapses_to_plain() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let s1 = a.seg_text("hi ");
        let s2 = a.seg_text("there");
        let c = a.content_segments(&[s1, s2]);
        assert_eq!(c.as_plain(), Some("hi there"));
    }

    #[test]
    fn content_segments_empty_collapses_to_empty_segments() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let c = a.content_segments(&[]);
        assert!(matches!(c, borrowed::Content::Segments(s) if s.is_empty()));
    }

    // ---------------------------------------------------------------------
    // Interner is wired up — repeated short strings share a single
    // arena slot.
    // ---------------------------------------------------------------------

    #[test]
    fn interner_dedups_repeated_readings() {
        let arena = Arena::new();
        let mut a = fresh_alloc(&arena);
        let base1 = a.content_plain("青梅");
        let reading1 = a.content_plain("おうめ");
        let n1 = a.ruby(base1, reading1, false);
        let base2 = a.content_plain("青梅");
        let reading2 = a.content_plain("おうめ");
        let n2 = a.ruby(base2, reading2, false);
        let borrowed::AozoraNode::Ruby(r1) = n1 else {
            unreachable!();
        };
        let borrowed::AozoraNode::Ruby(r2) = n2 else {
            unreachable!();
        };
        let s1 = r1.reading.as_plain().expect("plain");
        let s2 = r2.reading.as_plain().expect("plain");
        assert_eq!(
            s1.as_ptr(),
            s2.as_ptr(),
            "interner must dedup repeated readings"
        );
    }

    #[test]
    fn arena_accessor_returns_construction_arena() {
        let arena = Arena::new();
        let a = fresh_alloc(&arena);
        assert!(ptr::eq(a.arena(), &raw const arena));
    }
}
