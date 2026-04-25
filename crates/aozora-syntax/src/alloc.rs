//! AST construction abstraction (Plan I-2.2).
//!
//! [`NodeAllocator<'a>`] decouples the lex pipeline (`aozora-lexer`'s
//! Phase 3 classifier) from the concrete AST representation it
//! produces. Two implementations live in this module:
//!
//! - [`OwnedAllocator`] — emits the legacy [`crate::owned`] AST
//!   (`Box<str>` / `Box<AozoraNode>`). Zero state.
//! - [`BorrowedAllocator`] — emits the arena-backed
//!   [`crate::borrowed`] AST. Owns an [`Interner`] (Innovation I-7)
//!   so byte-equal strings (ruby readings, kaeriten marks, container
//!   labels) share a single arena allocation.
//!
//! ## Why a trait, not a plain function
//!
//! Without this trait, Phase 3 would have to either (a) hard-code the
//! owned types and rely on a post-pass converter (the status quo,
//! which double-walks the AST and per-node clones into the arena), or
//! (b) hard-code the borrowed types and add a complete second
//! classifier for the legacy owned path. The trait lets one classifier
//! body service both backends; the migration to a borrowed-only AST
//! becomes a straight delete-the-old-impl change rather than a code
//! rewrite, with proptest-pinned byte-identical output as a safety
//! net the entire way.
//!
//! ## Migration vehicle, not end state
//!
//! Once Phase 3 is rewritten against this trait (Commit C of the
//! I-2.2 sequence) and downstream consumers move off the owned AST
//! (Commit E), [`OwnedAllocator`] and the trait itself can be
//! deleted: Phase 3 then becomes a non-generic function calling
//! [`BorrowedAllocator`] methods directly. The trait gymnastics live
//! only as long as the migration needs them.
//!
//! ## Naming convention
//!
//! - `make_*` methods build *payload* types (`Self::Gaiji`,
//!   `Self::Annotation`) without wrapping them in a [`Self::Node`].
//! - Variant-named methods (`ruby`, `bouten`, `gaiji`, …) build the
//!   final [`Self::Node`]. The `gaiji` and `annotation` node
//!   constructors take the payload (built via `make_gaiji` /
//!   `make_annotation`) so a payload can be shared between a
//!   `Segment` and a `Node` without recomputing the string interns.
//! - `seg_*` methods build segment elements for `content_segments`.

use crate::borrowed::{self, Arena, Interner};
use crate::owned;
use crate::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, Container, Indent,
    Keigakomi, SectionKind,
};

/// AST construction contract — Phase 3 of the lex pipeline calls
/// these methods to build nodes, and the implementation chooses
/// whether the result is owned or arena-borrowed.
///
/// The lifetime parameter `'a` is the arena lifetime (or `'static`
/// for the owned impl, which doesn't use it). Associated types
/// abstract over the AST shape — they intentionally do not require
/// `Copy`, because the owned shape carries `Box<str>` / `Vec`
/// payloads that move rather than copy.
pub trait NodeAllocator<'a> {
    /// Top-level [`AozoraNode`](owned::AozoraNode) /
    /// [`borrowed::AozoraNode<'a>`] result type.
    type Node;
    /// Body content type. Mirrors `Content` in either AST.
    type Content;
    /// Segment type carried by `content_segments`.
    type Segment;
    /// `Gaiji` payload, used both as a `Segment` and as a `Node`.
    type Gaiji;
    /// `Annotation` payload, used both as a `Segment` and as a `Node`.
    type Annotation;

    // -----------------------------------------------------------------
    // Content / segment builders
    // -----------------------------------------------------------------

    /// Build a plain-text body content. Empty input canonicalises to
    /// `Segments(&[])` to match the owned `Content::from(&str)`
    /// behaviour — keeps the byte-identical proptest happy.
    fn content_plain(&mut self, s: &str) -> Self::Content;

    /// Build a body content from a sequence of segments. Implementations
    /// MUST canonicalise: all-`Text` segments collapse into a single
    /// `Plain`, empty input collapses into `Segments(&[])`. The owned
    /// AST already does this in [`owned::Content::from_segments`]; the
    /// borrowed impl mirrors the rule so both backends produce
    /// byte-identical output.
    fn content_segments(&mut self, segs: Vec<Self::Segment>) -> Self::Content;

    /// `Segment::Text(s)`.
    fn seg_text(&mut self, s: &str) -> Self::Segment;
    /// `Segment::Gaiji(g)` — takes a payload built via [`Self::make_gaiji`].
    fn seg_gaiji(&mut self, g: Self::Gaiji) -> Self::Segment;
    /// `Segment::Annotation(a)` — takes a payload built via [`Self::make_annotation`].
    fn seg_annotation(&mut self, a: Self::Annotation) -> Self::Segment;

    // -----------------------------------------------------------------
    // Payload builders (used by both Segment and Node constructors)
    // -----------------------------------------------------------------

    /// Build a `Gaiji` payload. Use [`Self::seg_gaiji`] to wrap as a
    /// segment, or [`Self::gaiji`] to wrap as a node.
    fn make_gaiji(
        &mut self,
        description: &str,
        ucs: Option<char>,
        mencode: Option<&str>,
    ) -> Self::Gaiji;

    /// Build an `Annotation` payload. Use [`Self::seg_annotation`] to
    /// wrap as a segment, or [`Self::annotation`] to wrap as a node.
    fn make_annotation(&mut self, raw: &str, kind: AnnotationKind) -> Self::Annotation;

    // -----------------------------------------------------------------
    // Node variant constructors (17 — matches the AozoraNode enum)
    // -----------------------------------------------------------------

    /// `AozoraNode::Ruby(Ruby { base, reading, delim_explicit })`.
    fn ruby(
        &mut self,
        base: Self::Content,
        reading: Self::Content,
        delim_explicit: bool,
    ) -> Self::Node;
    /// `AozoraNode::Bouten(Bouten { kind, target, position })`.
    fn bouten(
        &mut self,
        kind: BoutenKind,
        target: Self::Content,
        position: BoutenPosition,
    ) -> Self::Node;
    /// `AozoraNode::TateChuYoko(TateChuYoko { text })`.
    fn tate_chu_yoko(&mut self, text: Self::Content) -> Self::Node;
    /// `AozoraNode::Gaiji(g)`.
    fn gaiji(&mut self, g: Self::Gaiji) -> Self::Node;
    /// `AozoraNode::Indent(i)`.
    fn indent(&mut self, i: Indent) -> Self::Node;
    /// `AozoraNode::AlignEnd(a)`.
    fn align_end(&mut self, a: AlignEnd) -> Self::Node;
    /// `AozoraNode::Warichu(Warichu { upper, lower })`.
    fn warichu(&mut self, upper: Self::Content, lower: Self::Content) -> Self::Node;
    /// `AozoraNode::Keigakomi(k)`.
    fn keigakomi(&mut self, k: Keigakomi) -> Self::Node;
    /// `AozoraNode::PageBreak`.
    fn page_break(&mut self) -> Self::Node;
    /// `AozoraNode::SectionBreak(k)`.
    fn section_break(&mut self, k: SectionKind) -> Self::Node;
    /// `AozoraNode::AozoraHeading(AozoraHeading { kind, text })`.
    fn aozora_heading(&mut self, kind: AozoraHeadingKind, text: Self::Content) -> Self::Node;
    /// `AozoraNode::HeadingHint(HeadingHint { level, target })`.
    fn heading_hint(&mut self, level: u8, target: &str) -> Self::Node;
    /// `AozoraNode::Sashie(Sashie { file, caption })`.
    fn sashie(&mut self, file: &str, caption: Option<Self::Content>) -> Self::Node;
    /// `AozoraNode::Kaeriten(Kaeriten { mark })`.
    fn kaeriten(&mut self, mark: &str) -> Self::Node;
    /// `AozoraNode::Annotation(a)`.
    fn annotation(&mut self, a: Self::Annotation) -> Self::Node;
    /// `AozoraNode::DoubleRuby(DoubleRuby { content })`.
    fn double_ruby(&mut self, content: Self::Content) -> Self::Node;
    /// `AozoraNode::Container(c)`.
    fn container(&mut self, c: Container) -> Self::Node;
}

// =====================================================================
// OwnedAllocator — emits the legacy owned AST
// =====================================================================

/// [`NodeAllocator`] emitting the owned AST (`Box<str>` /
/// `Box<AozoraNode>`). Stateless unit struct; the `'a` parameter on
/// the trait impl is unused.
#[derive(Debug, Default, Clone, Copy)]
pub struct OwnedAllocator;

impl NodeAllocator<'_> for OwnedAllocator {
    type Node = owned::AozoraNode;
    type Content = owned::Content;
    type Segment = owned::Segment;
    type Gaiji = owned::Gaiji;
    type Annotation = owned::Annotation;

    fn content_plain(&mut self, s: &str) -> Self::Content {
        owned::Content::from(s)
    }

    fn content_segments(&mut self, segs: Vec<Self::Segment>) -> Self::Content {
        owned::Content::from_segments(segs)
    }

    fn seg_text(&mut self, s: &str) -> Self::Segment {
        owned::Segment::Text(s.into())
    }

    fn seg_gaiji(&mut self, g: Self::Gaiji) -> Self::Segment {
        owned::Segment::Gaiji(g)
    }

    fn seg_annotation(&mut self, a: Self::Annotation) -> Self::Segment {
        owned::Segment::Annotation(a)
    }

    fn make_gaiji(
        &mut self,
        description: &str,
        ucs: Option<char>,
        mencode: Option<&str>,
    ) -> Self::Gaiji {
        owned::Gaiji {
            description: description.into(),
            ucs,
            mencode: mencode.map(Into::into),
        }
    }

    fn make_annotation(&mut self, raw: &str, kind: AnnotationKind) -> Self::Annotation {
        owned::Annotation {
            raw: raw.into(),
            kind,
        }
    }

    fn ruby(
        &mut self,
        base: Self::Content,
        reading: Self::Content,
        delim_explicit: bool,
    ) -> Self::Node {
        owned::AozoraNode::Ruby(owned::Ruby {
            base,
            reading,
            delim_explicit,
        })
    }

    fn bouten(
        &mut self,
        kind: BoutenKind,
        target: Self::Content,
        position: BoutenPosition,
    ) -> Self::Node {
        owned::AozoraNode::Bouten(owned::Bouten {
            kind,
            target,
            position,
        })
    }

    fn tate_chu_yoko(&mut self, text: Self::Content) -> Self::Node {
        owned::AozoraNode::TateChuYoko(owned::TateChuYoko { text })
    }

    fn gaiji(&mut self, g: Self::Gaiji) -> Self::Node {
        owned::AozoraNode::Gaiji(g)
    }

    fn indent(&mut self, i: Indent) -> Self::Node {
        owned::AozoraNode::Indent(i)
    }

    fn align_end(&mut self, a: AlignEnd) -> Self::Node {
        owned::AozoraNode::AlignEnd(a)
    }

    fn warichu(&mut self, upper: Self::Content, lower: Self::Content) -> Self::Node {
        owned::AozoraNode::Warichu(owned::Warichu { upper, lower })
    }

    fn keigakomi(&mut self, k: Keigakomi) -> Self::Node {
        owned::AozoraNode::Keigakomi(k)
    }

    fn page_break(&mut self) -> Self::Node {
        owned::AozoraNode::PageBreak
    }

    fn section_break(&mut self, k: SectionKind) -> Self::Node {
        owned::AozoraNode::SectionBreak(k)
    }

    fn aozora_heading(&mut self, kind: AozoraHeadingKind, text: Self::Content) -> Self::Node {
        owned::AozoraNode::AozoraHeading(owned::AozoraHeading { kind, text })
    }

    fn heading_hint(&mut self, level: u8, target: &str) -> Self::Node {
        owned::AozoraNode::HeadingHint(owned::HeadingHint {
            level,
            target: target.into(),
        })
    }

    fn sashie(&mut self, file: &str, caption: Option<Self::Content>) -> Self::Node {
        owned::AozoraNode::Sashie(owned::Sashie {
            file: file.into(),
            caption,
        })
    }

    fn kaeriten(&mut self, mark: &str) -> Self::Node {
        owned::AozoraNode::Kaeriten(owned::Kaeriten { mark: mark.into() })
    }

    fn annotation(&mut self, a: Self::Annotation) -> Self::Node {
        owned::AozoraNode::Annotation(a)
    }

    fn double_ruby(&mut self, content: Self::Content) -> Self::Node {
        owned::AozoraNode::DoubleRuby(owned::DoubleRuby { content })
    }

    fn container(&mut self, c: Container) -> Self::Node {
        owned::AozoraNode::Container(c)
    }
}

// =====================================================================
// BorrowedAllocator — Commit B (next agent). Stub left here so the
// trait file documents both backends in one place.
// =====================================================================

/// [`NodeAllocator`] emitting the arena-backed borrowed AST.
///
/// Owns an [`Interner`] (Innovation I-7) so repeated short strings
/// (ruby readings like `の` / `に` / `を`, container labels, kaeriten
/// marks) share a single arena allocation. The arena is borrowed,
/// not owned — drop the arena and the entire emitted AST is gone in
/// one step, no per-node `Drop` traffic.
///
/// _Implementation lands in Commit B._
#[derive(Debug)]
pub struct BorrowedAllocator<'a> {
    arena: &'a Arena,
    interner: Interner<'a>,
}

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

    /// Construct with the interner's default initial capacity (256).
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
    pub fn into_interner(self) -> Interner<'a> {
        self.interner
    }
}

// Stub trait impl: every method panics with an explicit "not yet
// implemented in Commit A" message. Commit B replaces this. The stub
// exists so the trait file documents *both* backends and downstream
// code that imports `BorrowedAllocator` against the trait surface
// type-checks today.
impl<'a> NodeAllocator<'a> for BorrowedAllocator<'a> {
    type Node = borrowed::AozoraNode<'a>;
    type Content = borrowed::Content<'a>;
    type Segment = borrowed::Segment<'a>;
    type Gaiji = &'a borrowed::Gaiji<'a>;
    type Annotation = &'a borrowed::Annotation<'a>;

    fn content_plain(&mut self, _s: &str) -> Self::Content {
        unimplemented!("BorrowedAllocator::content_plain — landing in Commit B")
    }
    fn content_segments(&mut self, _segs: Vec<Self::Segment>) -> Self::Content {
        unimplemented!("BorrowedAllocator::content_segments — landing in Commit B")
    }
    fn seg_text(&mut self, _s: &str) -> Self::Segment {
        unimplemented!("BorrowedAllocator::seg_text — landing in Commit B")
    }
    fn seg_gaiji(&mut self, _g: Self::Gaiji) -> Self::Segment {
        unimplemented!("BorrowedAllocator::seg_gaiji — landing in Commit B")
    }
    fn seg_annotation(&mut self, _a: Self::Annotation) -> Self::Segment {
        unimplemented!("BorrowedAllocator::seg_annotation — landing in Commit B")
    }
    fn make_gaiji(
        &mut self,
        _description: &str,
        _ucs: Option<char>,
        _mencode: Option<&str>,
    ) -> Self::Gaiji {
        unimplemented!("BorrowedAllocator::make_gaiji — landing in Commit B")
    }
    fn make_annotation(&mut self, _raw: &str, _kind: AnnotationKind) -> Self::Annotation {
        unimplemented!("BorrowedAllocator::make_annotation — landing in Commit B")
    }
    fn ruby(
        &mut self,
        _base: Self::Content,
        _reading: Self::Content,
        _delim_explicit: bool,
    ) -> Self::Node {
        unimplemented!("BorrowedAllocator::ruby — landing in Commit B")
    }
    fn bouten(
        &mut self,
        _kind: BoutenKind,
        _target: Self::Content,
        _position: BoutenPosition,
    ) -> Self::Node {
        unimplemented!("BorrowedAllocator::bouten — landing in Commit B")
    }
    fn tate_chu_yoko(&mut self, _text: Self::Content) -> Self::Node {
        unimplemented!("BorrowedAllocator::tate_chu_yoko — landing in Commit B")
    }
    fn gaiji(&mut self, _g: Self::Gaiji) -> Self::Node {
        unimplemented!("BorrowedAllocator::gaiji — landing in Commit B")
    }
    fn indent(&mut self, _i: Indent) -> Self::Node {
        unimplemented!("BorrowedAllocator::indent — landing in Commit B")
    }
    fn align_end(&mut self, _a: AlignEnd) -> Self::Node {
        unimplemented!("BorrowedAllocator::align_end — landing in Commit B")
    }
    fn warichu(&mut self, _upper: Self::Content, _lower: Self::Content) -> Self::Node {
        unimplemented!("BorrowedAllocator::warichu — landing in Commit B")
    }
    fn keigakomi(&mut self, _k: Keigakomi) -> Self::Node {
        unimplemented!("BorrowedAllocator::keigakomi — landing in Commit B")
    }
    fn page_break(&mut self) -> Self::Node {
        unimplemented!("BorrowedAllocator::page_break — landing in Commit B")
    }
    fn section_break(&mut self, _k: SectionKind) -> Self::Node {
        unimplemented!("BorrowedAllocator::section_break — landing in Commit B")
    }
    fn aozora_heading(&mut self, _kind: AozoraHeadingKind, _text: Self::Content) -> Self::Node {
        unimplemented!("BorrowedAllocator::aozora_heading — landing in Commit B")
    }
    fn heading_hint(&mut self, _level: u8, _target: &str) -> Self::Node {
        unimplemented!("BorrowedAllocator::heading_hint — landing in Commit B")
    }
    fn sashie(&mut self, _file: &str, _caption: Option<Self::Content>) -> Self::Node {
        unimplemented!("BorrowedAllocator::sashie — landing in Commit B")
    }
    fn kaeriten(&mut self, _mark: &str) -> Self::Node {
        unimplemented!("BorrowedAllocator::kaeriten — landing in Commit B")
    }
    fn annotation(&mut self, _a: Self::Annotation) -> Self::Node {
        unimplemented!("BorrowedAllocator::annotation — landing in Commit B")
    }
    fn double_ruby(&mut self, _content: Self::Content) -> Self::Node {
        unimplemented!("BorrowedAllocator::double_ruby — landing in Commit B")
    }
    fn container(&mut self, _c: Container) -> Self::Node {
        unimplemented!("BorrowedAllocator::container — landing in Commit B")
    }
}

#[cfg(test)]
mod tests {
    //! Owned-allocator equivalence tests.
    //!
    //! Each variant goes through two construction paths — the new
    //! `OwnedAllocator` trait method and the direct enum constructor
    //! — and we assert *full payload equality* (not just
    //! `xml_node_name`). This is the gate that catches a misrouted
    //! string in Commit C: if Phase 3 builds a `Ruby` via
    //! `alloc.ruby(base, reading, false)` and the trait impl swaps
    //! `base` with `reading`, the assertion fires immediately.
    //!
    //! Borrowed-allocator round-trip equivalence lands in Commit B.

    use super::*;
    use crate::owned::{
        Annotation, AozoraHeading, AozoraNode, Bouten, Content, DoubleRuby, Gaiji, HeadingHint,
        Kaeriten, Ruby, Sashie, Segment, TateChuYoko, Warichu,
    };
    use crate::{
        AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, Container,
        ContainerKind, Indent, Keigakomi, SectionKind,
    };

    fn alloc() -> OwnedAllocator {
        OwnedAllocator
    }

    #[test]
    fn ruby_payload_equals_direct_constructor() {
        let mut a = alloc();
        let base = a.content_plain("青梅");
        let reading = a.content_plain("おうめ");
        let n = a.ruby(base, reading, true);
        let expected = AozoraNode::Ruby(Ruby {
            base: Content::from("青梅"),
            reading: Content::from("おうめ"),
            delim_explicit: true,
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn bouten_payload_equals_direct_constructor() {
        let mut a = alloc();
        let target = a.content_plain("青空");
        let n = a.bouten(BoutenKind::Goma, target, BoutenPosition::Right);
        let expected = AozoraNode::Bouten(Bouten {
            kind: BoutenKind::Goma,
            target: Content::from("青空"),
            position: BoutenPosition::Right,
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn tate_chu_yoko_payload_equals_direct_constructor() {
        let mut a = alloc();
        let text = a.content_plain("12");
        let n = a.tate_chu_yoko(text);
        let expected = AozoraNode::TateChuYoko(TateChuYoko {
            text: Content::from("12"),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn gaiji_payload_equals_direct_constructor() {
        let mut a = alloc();
        let g = a.make_gaiji("木＋吶のつくり", Some('𠀋'), Some("第3水準1-85-54"));
        let n = a.gaiji(g);
        let expected = AozoraNode::Gaiji(Gaiji {
            description: "木＋吶のつくり".into(),
            ucs: Some('𠀋'),
            mencode: Some("第3水準1-85-54".into()),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn gaiji_with_no_mencode_payload_equals_direct_constructor() {
        let mut a = alloc();
        let g = a.make_gaiji("desc", None, None);
        let n = a.gaiji(g);
        let expected = AozoraNode::Gaiji(Gaiji {
            description: "desc".into(),
            ucs: None,
            mencode: None,
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn indent_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.indent(Indent { amount: 3 });
        assert_eq!(n, AozoraNode::Indent(Indent { amount: 3 }));
    }

    #[test]
    fn align_end_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.align_end(AlignEnd { offset: 2 });
        assert_eq!(n, AozoraNode::AlignEnd(AlignEnd { offset: 2 }));
    }

    #[test]
    fn warichu_payload_equals_direct_constructor() {
        let mut a = alloc();
        let upper = a.content_plain("上");
        let lower = a.content_plain("下");
        let n = a.warichu(upper, lower);
        let expected = AozoraNode::Warichu(Warichu {
            upper: Content::from("上"),
            lower: Content::from("下"),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn keigakomi_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.keigakomi(Keigakomi);
        assert_eq!(n, AozoraNode::Keigakomi(Keigakomi));
    }

    #[test]
    fn page_break_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.page_break();
        assert_eq!(n, AozoraNode::PageBreak);
    }

    #[test]
    fn section_break_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.section_break(SectionKind::Choho);
        assert_eq!(n, AozoraNode::SectionBreak(SectionKind::Choho));
    }

    #[test]
    fn aozora_heading_payload_equals_direct_constructor() {
        let mut a = alloc();
        let text = a.content_plain("見出し");
        let n = a.aozora_heading(AozoraHeadingKind::Window, text);
        let expected = AozoraNode::AozoraHeading(AozoraHeading {
            kind: AozoraHeadingKind::Window,
            text: Content::from("見出し"),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn heading_hint_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.heading_hint(2, "対象");
        let expected = AozoraNode::HeadingHint(HeadingHint {
            level: 2,
            target: "対象".into(),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn sashie_with_caption_payload_equals_direct_constructor() {
        let mut a = alloc();
        let caption = Some(a.content_plain("挿絵キャプション"));
        let n = a.sashie("fig01.png", caption);
        let expected = AozoraNode::Sashie(Sashie {
            file: "fig01.png".into(),
            caption: Some(Content::from("挿絵キャプション")),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn sashie_without_caption_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.sashie("fig02.png", None);
        let expected = AozoraNode::Sashie(Sashie {
            file: "fig02.png".into(),
            caption: None,
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn kaeriten_payload_equals_direct_constructor() {
        let mut a = alloc();
        let n = a.kaeriten("一");
        let expected = AozoraNode::Kaeriten(Kaeriten { mark: "一".into() });
        assert_eq!(n, expected);
    }

    #[test]
    fn annotation_payload_equals_direct_constructor() {
        let mut a = alloc();
        let payload = a.make_annotation("［＃X］", AnnotationKind::Unknown);
        let n = a.annotation(payload);
        let expected = AozoraNode::Annotation(Annotation {
            raw: "［＃X］".into(),
            kind: AnnotationKind::Unknown,
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn double_ruby_payload_equals_direct_constructor() {
        let mut a = alloc();
        let content = a.content_plain("重要");
        let n = a.double_ruby(content);
        let expected = AozoraNode::DoubleRuby(DoubleRuby {
            content: Content::from("重要"),
        });
        assert_eq!(n, expected);
    }

    #[test]
    fn container_payload_equals_direct_constructor() {
        let mut a = alloc();
        let c = Container {
            kind: ContainerKind::Indent { amount: 1 },
        };
        let n = a.container(c);
        assert_eq!(n, AozoraNode::Container(c));
    }

    // -----------------------------------------------------------------
    // Content / Segment composition tests
    // -----------------------------------------------------------------

    #[test]
    fn content_plain_empty_collapses_to_empty_segments() {
        let mut a = alloc();
        let c = a.content_plain("");
        // Mirrors `owned::Content::from("")` canonicalisation.
        assert_eq!(c, Content::default());
    }

    #[test]
    fn content_plain_nonempty_returns_plain_variant() {
        let mut a = alloc();
        let c = a.content_plain("hello");
        assert_eq!(c.as_plain(), Some("hello"));
    }

    #[test]
    fn content_segments_preserves_order_and_kind() {
        let mut a = alloc();
        let g = a.make_gaiji("X", None, None);
        let seg_g = a.seg_gaiji(g);
        let seg_t1 = a.seg_text("before ");
        let seg_t2 = a.seg_text(" after");
        let ann = a.make_annotation("［＃X］", AnnotationKind::Unknown);
        let seg_a = a.seg_annotation(ann);
        let c = a.content_segments(vec![seg_t1, seg_g, seg_t2, seg_a]);
        // mixed kinds ⇒ Segments path (no canonicalisation collapse)
        let segments = match c {
            Content::Segments(s) => s,
            _ => panic!("expected Segments variant for mixed-kind input"),
        };
        assert_eq!(segments.len(), 4);
        assert!(matches!(&segments[0], Segment::Text(t) if &**t == "before "));
        assert!(matches!(&segments[1], Segment::Gaiji(_)));
        assert!(matches!(&segments[2], Segment::Text(t) if &**t == " after"));
        assert!(matches!(&segments[3], Segment::Annotation(_)));
    }

    #[test]
    fn content_segments_all_text_collapses_to_plain() {
        let mut a = alloc();
        let s1 = a.seg_text("hi ");
        let s2 = a.seg_text("there");
        let c = a.content_segments(vec![s1, s2]);
        // Owned `Content::from_segments` canonicalises all-Text → Plain.
        assert_eq!(c.as_plain(), Some("hi there"));
    }

    #[test]
    fn content_segments_empty_collapses_to_empty_segments() {
        let mut a = alloc();
        let c = a.content_segments(vec![]);
        assert_eq!(c, Content::default());
    }

    // -----------------------------------------------------------------
    // BorrowedAllocator surface — only metadata accessors today, full
    // method impls land in Commit B.
    // -----------------------------------------------------------------

    #[test]
    fn borrowed_allocator_constructible_with_arena() {
        let arena = Arena::new();
        let b = BorrowedAllocator::new(&arena);
        assert!(core::ptr::eq(b.arena(), &arena));
    }

    #[test]
    #[should_panic(expected = "Commit B")]
    fn borrowed_allocator_methods_panic_until_commit_b() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let _ = b.content_plain("x");
    }
}
