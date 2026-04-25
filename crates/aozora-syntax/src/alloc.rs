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

/// Strings flow through the interner; per-node payloads are
/// `arena.alloc(...)`. Mirrors `convert::to_borrowed_with` exactly so
/// the byte-identical proptest in `aozora-lex/tests/property_borrowed_arena.rs`
/// continues to pass once Phase 3 routes through this allocator
/// (Commit C).
///
/// ## Canonicalisation
///
/// Both `content_plain("")` and `content_segments(vec![])` return
/// `Content::Segments(&[])` to match `owned::Content::from("")` and
/// `owned::Content::from_segments(vec![])` exactly. `content_segments`
/// also collapses an all-`Text` input into a single concatenated
/// `Plain` (the concatenation is interned). Without this collapse the
/// borrowed AST would diverge from the owned shape on documents like
/// `｜青空《せい—く》` where the body parses into multiple text-only
/// segments, and the byte-identical proptest would fail at Commit C.
impl<'a> NodeAllocator<'a> for BorrowedAllocator<'a> {
    type Node = borrowed::AozoraNode<'a>;
    type Content = borrowed::Content<'a>;
    type Segment = borrowed::Segment<'a>;
    type Gaiji = &'a borrowed::Gaiji<'a>;
    type Annotation = &'a borrowed::Annotation<'a>;

    fn content_plain(&mut self, s: &str) -> Self::Content {
        if s.is_empty() {
            borrowed::Content::EMPTY
        } else {
            borrowed::Content::Plain(self.interner.intern(s))
        }
    }

    fn content_segments(&mut self, segs: Vec<Self::Segment>) -> Self::Content {
        if segs.is_empty() {
            return borrowed::Content::EMPTY;
        }
        // Canonicalisation: all-Text → concatenate + Plain. Mirrors
        // `owned::Content::from_segments` so byte-identical output is
        // preserved.
        if segs
            .iter()
            .all(|s| matches!(s, borrowed::Segment::Text(_)))
        {
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
            for s in &segs {
                if let borrowed::Segment::Text(t) = s {
                    buf.push_str(t);
                }
            }
            return borrowed::Content::Plain(self.interner.intern(&buf));
        }
        borrowed::Content::Segments(self.arena.alloc_slice_copy(&segs))
    }

    fn seg_text(&mut self, s: &str) -> Self::Segment {
        borrowed::Segment::Text(self.interner.intern(s))
    }

    fn seg_gaiji(&mut self, g: Self::Gaiji) -> Self::Segment {
        borrowed::Segment::Gaiji(g)
    }

    fn seg_annotation(&mut self, a: Self::Annotation) -> Self::Segment {
        borrowed::Segment::Annotation(a)
    }

    fn make_gaiji(
        &mut self,
        description: &str,
        ucs: Option<char>,
        mencode: Option<&str>,
    ) -> Self::Gaiji {
        let g = borrowed::Gaiji {
            description: self.interner.intern(description),
            ucs,
            mencode: mencode.map(|s| self.interner.intern(s)),
        };
        self.arena.alloc(g)
    }

    fn make_annotation(&mut self, raw: &str, kind: AnnotationKind) -> Self::Annotation {
        let a = borrowed::Annotation {
            raw: self.interner.intern(raw),
            kind,
        };
        self.arena.alloc(a)
    }

    fn ruby(
        &mut self,
        base: Self::Content,
        reading: Self::Content,
        delim_explicit: bool,
    ) -> Self::Node {
        borrowed::AozoraNode::Ruby(self.arena.alloc(borrowed::Ruby {
            base,
            reading,
            delim_explicit,
        }))
    }

    fn bouten(
        &mut self,
        kind: BoutenKind,
        target: Self::Content,
        position: BoutenPosition,
    ) -> Self::Node {
        borrowed::AozoraNode::Bouten(self.arena.alloc(borrowed::Bouten {
            kind,
            target,
            position,
        }))
    }

    fn tate_chu_yoko(&mut self, text: Self::Content) -> Self::Node {
        borrowed::AozoraNode::TateChuYoko(self.arena.alloc(borrowed::TateChuYoko { text }))
    }

    fn gaiji(&mut self, g: Self::Gaiji) -> Self::Node {
        borrowed::AozoraNode::Gaiji(g)
    }

    fn indent(&mut self, i: Indent) -> Self::Node {
        borrowed::AozoraNode::Indent(i)
    }

    fn align_end(&mut self, a: AlignEnd) -> Self::Node {
        borrowed::AozoraNode::AlignEnd(a)
    }

    fn warichu(&mut self, upper: Self::Content, lower: Self::Content) -> Self::Node {
        borrowed::AozoraNode::Warichu(self.arena.alloc(borrowed::Warichu { upper, lower }))
    }

    fn keigakomi(&mut self, k: Keigakomi) -> Self::Node {
        borrowed::AozoraNode::Keigakomi(k)
    }

    fn page_break(&mut self) -> Self::Node {
        borrowed::AozoraNode::PageBreak
    }

    fn section_break(&mut self, k: SectionKind) -> Self::Node {
        borrowed::AozoraNode::SectionBreak(k)
    }

    fn aozora_heading(&mut self, kind: AozoraHeadingKind, text: Self::Content) -> Self::Node {
        borrowed::AozoraNode::AozoraHeading(self.arena.alloc(borrowed::AozoraHeading {
            kind,
            text,
        }))
    }

    fn heading_hint(&mut self, level: u8, target: &str) -> Self::Node {
        borrowed::AozoraNode::HeadingHint(self.arena.alloc(borrowed::HeadingHint {
            level,
            target: self.interner.intern(target),
        }))
    }

    fn sashie(&mut self, file: &str, caption: Option<Self::Content>) -> Self::Node {
        borrowed::AozoraNode::Sashie(self.arena.alloc(borrowed::Sashie {
            file: self.interner.intern(file),
            caption,
        }))
    }

    fn kaeriten(&mut self, mark: &str) -> Self::Node {
        borrowed::AozoraNode::Kaeriten(self.arena.alloc(borrowed::Kaeriten {
            mark: self.interner.intern(mark),
        }))
    }

    fn annotation(&mut self, a: Self::Annotation) -> Self::Node {
        borrowed::AozoraNode::Annotation(a)
    }

    fn double_ruby(&mut self, content: Self::Content) -> Self::Node {
        borrowed::AozoraNode::DoubleRuby(self.arena.alloc(borrowed::DoubleRuby { content }))
    }

    fn container(&mut self, c: Container) -> Self::Node {
        borrowed::AozoraNode::Container(c)
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
    // BorrowedAllocator round-trip equivalence (Commit B)
    //
    // For each variant, build the same logical node via OwnedAllocator
    // and BorrowedAllocator and assert the borrowed result has the
    // matching shape + payload. We compare via xml_node_name +
    // explicit field reads rather than synthesising owned-from-borrowed
    // (the borrowed AST has no `to_owned`), since field-by-field is the
    // direct check the byte-identical proptest needs at Commit C.
    // -----------------------------------------------------------------

    #[test]
    fn borrowed_allocator_constructible_with_arena() {
        let arena = Arena::new();
        let b = BorrowedAllocator::new(&arena);
        assert!(core::ptr::eq(b.arena(), &arena));
    }

    #[test]
    fn borrowed_ruby_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let base = b.content_plain("青梅");
        let reading = b.content_plain("おうめ");
        let n = b.ruby(base, reading, true);
        let r = match n {
            borrowed::AozoraNode::Ruby(r) => r,
            other => panic!("expected Ruby, got {other:?}"),
        };
        assert_eq!(r.base.as_plain(), Some("青梅"));
        assert_eq!(r.reading.as_plain(), Some("おうめ"));
        assert!(r.delim_explicit);
    }

    #[test]
    fn borrowed_bouten_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let target = b.content_plain("青空");
        let n = b.bouten(BoutenKind::Goma, target, BoutenPosition::Right);
        let bo = match n {
            borrowed::AozoraNode::Bouten(b) => b,
            other => panic!("expected Bouten, got {other:?}"),
        };
        assert_eq!(bo.kind, BoutenKind::Goma);
        assert_eq!(bo.target.as_plain(), Some("青空"));
        assert_eq!(bo.position, BoutenPosition::Right);
    }

    #[test]
    fn borrowed_tate_chu_yoko_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let text = b.content_plain("12");
        let n = b.tate_chu_yoko(text);
        let t = match n {
            borrowed::AozoraNode::TateChuYoko(t) => t,
            other => panic!("expected TateChuYoko, got {other:?}"),
        };
        assert_eq!(t.text.as_plain(), Some("12"));
    }

    #[test]
    fn borrowed_gaiji_with_full_metadata() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let g = b.make_gaiji("木＋吶のつくり", Some('𠀋'), Some("第3水準1-85-54"));
        let n = b.gaiji(g);
        let gn = match n {
            borrowed::AozoraNode::Gaiji(g) => g,
            other => panic!("expected Gaiji, got {other:?}"),
        };
        assert_eq!(gn.description, "木＋吶のつくり");
        assert_eq!(gn.ucs, Some('𠀋'));
        assert_eq!(gn.mencode, Some("第3水準1-85-54"));
    }

    #[test]
    fn borrowed_gaiji_with_no_mencode() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let g = b.make_gaiji("desc", None, None);
        let n = b.gaiji(g);
        let gn = match n {
            borrowed::AozoraNode::Gaiji(g) => g,
            other => panic!("expected Gaiji, got {other:?}"),
        };
        assert_eq!(gn.description, "desc");
        assert!(gn.ucs.is_none());
        assert!(gn.mencode.is_none());
    }

    #[test]
    fn borrowed_indent_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.indent(Indent { amount: 3 });
        assert!(matches!(
            n,
            borrowed::AozoraNode::Indent(Indent { amount: 3 })
        ));
    }

    #[test]
    fn borrowed_align_end_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.align_end(AlignEnd { offset: 2 });
        assert!(matches!(
            n,
            borrowed::AozoraNode::AlignEnd(AlignEnd { offset: 2 })
        ));
    }

    #[test]
    fn borrowed_warichu_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let upper = b.content_plain("上");
        let lower = b.content_plain("下");
        let n = b.warichu(upper, lower);
        let w = match n {
            borrowed::AozoraNode::Warichu(w) => w,
            other => panic!("expected Warichu, got {other:?}"),
        };
        assert_eq!(w.upper.as_plain(), Some("上"));
        assert_eq!(w.lower.as_plain(), Some("下"));
    }

    #[test]
    fn borrowed_keigakomi_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.keigakomi(Keigakomi);
        assert!(matches!(n, borrowed::AozoraNode::Keigakomi(Keigakomi)));
    }

    #[test]
    fn borrowed_page_break_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.page_break();
        assert!(matches!(n, borrowed::AozoraNode::PageBreak));
    }

    #[test]
    fn borrowed_section_break_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.section_break(SectionKind::Choho);
        assert!(matches!(
            n,
            borrowed::AozoraNode::SectionBreak(SectionKind::Choho)
        ));
    }

    #[test]
    fn borrowed_aozora_heading_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let text = b.content_plain("見出し");
        let n = b.aozora_heading(AozoraHeadingKind::Window, text);
        let h = match n {
            borrowed::AozoraNode::AozoraHeading(h) => h,
            other => panic!("expected AozoraHeading, got {other:?}"),
        };
        assert_eq!(h.kind, AozoraHeadingKind::Window);
        assert_eq!(h.text.as_plain(), Some("見出し"));
    }

    #[test]
    fn borrowed_heading_hint_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.heading_hint(2, "対象");
        let h = match n {
            borrowed::AozoraNode::HeadingHint(h) => h,
            other => panic!("expected HeadingHint, got {other:?}"),
        };
        assert_eq!(h.level, 2);
        assert_eq!(h.target, "対象");
    }

    #[test]
    fn borrowed_sashie_with_caption() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let caption = b.content_plain("挿絵キャプション");
        let n = b.sashie("fig01.png", Some(caption));
        let s = match n {
            borrowed::AozoraNode::Sashie(s) => s,
            other => panic!("expected Sashie, got {other:?}"),
        };
        assert_eq!(s.file, "fig01.png");
        assert_eq!(
            s.caption.and_then(borrowed::Content::as_plain),
            Some("挿絵キャプション")
        );
    }

    #[test]
    fn borrowed_sashie_without_caption() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.sashie("fig02.png", None);
        let s = match n {
            borrowed::AozoraNode::Sashie(s) => s,
            other => panic!("expected Sashie, got {other:?}"),
        };
        assert_eq!(s.file, "fig02.png");
        assert!(s.caption.is_none());
    }

    #[test]
    fn borrowed_kaeriten_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let n = b.kaeriten("一");
        let k = match n {
            borrowed::AozoraNode::Kaeriten(k) => k,
            other => panic!("expected Kaeriten, got {other:?}"),
        };
        assert_eq!(k.mark, "一");
    }

    #[test]
    fn borrowed_annotation_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let payload = b.make_annotation("［＃X］", AnnotationKind::Unknown);
        let n = b.annotation(payload);
        let a = match n {
            borrowed::AozoraNode::Annotation(a) => a,
            other => panic!("expected Annotation, got {other:?}"),
        };
        assert_eq!(a.raw, "［＃X］");
        assert_eq!(a.kind, AnnotationKind::Unknown);
    }

    #[test]
    fn borrowed_double_ruby_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let content = b.content_plain("重要");
        let n = b.double_ruby(content);
        let d = match n {
            borrowed::AozoraNode::DoubleRuby(d) => d,
            other => panic!("expected DoubleRuby, got {other:?}"),
        };
        assert_eq!(d.content.as_plain(), Some("重要"));
    }

    #[test]
    fn borrowed_container_round_trip_equals_owned() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let c = Container {
            kind: ContainerKind::Indent { amount: 1 },
        };
        let n = b.container(c);
        assert!(matches!(n, borrowed::AozoraNode::Container(cc) if cc == c));
    }

    // -----------------------------------------------------------------
    // Borrowed content / segment composition (canonicalisation rules)
    // -----------------------------------------------------------------

    #[test]
    fn borrowed_content_plain_empty_collapses_to_empty_segments() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let c = b.content_plain("");
        // Mirrors `owned::Content::from("")` canonicalisation; needed
        // for byte-identical proptest equivalence.
        assert!(matches!(c, borrowed::Content::Segments(s) if s.is_empty()));
    }

    #[test]
    fn borrowed_content_plain_nonempty_returns_plain_variant() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let c = b.content_plain("hello");
        assert_eq!(c.as_plain(), Some("hello"));
    }

    #[test]
    fn borrowed_content_segments_preserves_order_and_kind() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let g = b.make_gaiji("X", None, None);
        let seg_g = b.seg_gaiji(g);
        let seg_t1 = b.seg_text("before ");
        let seg_t2 = b.seg_text(" after");
        let ann = b.make_annotation("［＃X］", AnnotationKind::Unknown);
        let seg_a = b.seg_annotation(ann);
        let c = b.content_segments(vec![seg_t1, seg_g, seg_t2, seg_a]);
        let segs = match c {
            borrowed::Content::Segments(s) => s,
            _ => panic!("expected Segments variant for mixed-kind input"),
        };
        assert_eq!(segs.len(), 4);
        assert!(matches!(&segs[0], borrowed::Segment::Text(t) if *t == "before "));
        assert!(matches!(&segs[1], borrowed::Segment::Gaiji(_)));
        assert!(matches!(&segs[2], borrowed::Segment::Text(t) if *t == " after"));
        assert!(matches!(&segs[3], borrowed::Segment::Annotation(_)));
    }

    #[test]
    fn borrowed_content_segments_all_text_collapses_to_plain() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let s1 = b.seg_text("hi ");
        let s2 = b.seg_text("there");
        let c = b.content_segments(vec![s1, s2]);
        // Mirrors `owned::Content::from_segments` canonicalisation.
        assert_eq!(c.as_plain(), Some("hi there"));
    }

    #[test]
    fn borrowed_content_segments_empty_collapses_to_empty_segments() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        let c = b.content_segments(vec![]);
        assert!(matches!(c, borrowed::Content::Segments(s) if s.is_empty()));
    }

    // -----------------------------------------------------------------
    // Interner dedup is wired up — repeated short strings must share
    // a single arena slot.
    // -----------------------------------------------------------------

    #[test]
    fn borrowed_interner_dedups_repeated_readings() {
        let arena = Arena::new();
        let mut b = BorrowedAllocator::new(&arena);
        // Build two Ruby nodes with the same reading. Pointer
        // equality of the resulting `&'a str` proves the interner
        // is wired through `content_plain`.
        let base1 = b.content_plain("青梅");
        let reading1 = b.content_plain("おうめ");
        let n1 = b.ruby(base1, reading1, false);
        let base2 = b.content_plain("青梅");
        let reading2 = b.content_plain("おうめ");
        let n2 = b.ruby(base2, reading2, false);
        let r1 = match n1 {
            borrowed::AozoraNode::Ruby(r) => r,
            _ => unreachable!(),
        };
        let r2 = match n2 {
            borrowed::AozoraNode::Ruby(r) => r,
            _ => unreachable!(),
        };
        let s1 = r1.reading.as_plain().expect("plain");
        let s2 = r2.reading.as_plain().expect("plain");
        assert_eq!(s1.as_ptr(), s2.as_ptr(), "interner must dedup repeated readings");
    }

    // -----------------------------------------------------------------
    // Cross-allocator equivalence: every variant constructed via both
    // backends must produce the same xml_node_name and is_block.
    // -----------------------------------------------------------------

    #[allow(
        clippy::too_many_lines,
        reason = "single equivalence sweep — splitting hides the parallel-construction pattern"
    )]
    #[test]
    fn cross_allocator_xml_names_match_for_every_variant() {
        let arena = Arena::new();
        let mut o = OwnedAllocator;
        let mut b = BorrowedAllocator::new(&arena);

        // Build both shapes side-by-side. Each entry uses scoped
        // blocks so the borrow on the allocator is released between
        // arg construction and node construction (Rust enforces this
        // even for stateless `OwnedAllocator`, since the trait
        // signature is `&mut self`).
        let pairs: Vec<(AozoraNode, borrowed::AozoraNode<'_>)> = vec![
            (
                {
                    let base = o.content_plain("a");
                    let reading = o.content_plain("b");
                    o.ruby(base, reading, true)
                },
                {
                    let base = b.content_plain("a");
                    let reading = b.content_plain("b");
                    b.ruby(base, reading, true)
                },
            ),
            (
                {
                    let target = o.content_plain("x");
                    o.bouten(BoutenKind::Goma, target, BoutenPosition::Right)
                },
                {
                    let target = b.content_plain("x");
                    b.bouten(BoutenKind::Goma, target, BoutenPosition::Right)
                },
            ),
            (
                {
                    let text = o.content_plain("12");
                    o.tate_chu_yoko(text)
                },
                {
                    let text = b.content_plain("12");
                    b.tate_chu_yoko(text)
                },
            ),
            (
                {
                    let g = o.make_gaiji("desc", Some('A'), Some("1-2-3"));
                    o.gaiji(g)
                },
                {
                    let g = b.make_gaiji("desc", Some('A'), Some("1-2-3"));
                    b.gaiji(g)
                },
            ),
            (o.indent(Indent { amount: 3 }), b.indent(Indent { amount: 3 })),
            (
                o.align_end(AlignEnd { offset: 2 }),
                b.align_end(AlignEnd { offset: 2 }),
            ),
            (
                {
                    let upper = o.content_plain("u");
                    let lower = o.content_plain("l");
                    o.warichu(upper, lower)
                },
                {
                    let upper = b.content_plain("u");
                    let lower = b.content_plain("l");
                    b.warichu(upper, lower)
                },
            ),
            (o.keigakomi(Keigakomi), b.keigakomi(Keigakomi)),
            (o.page_break(), b.page_break()),
            (
                o.section_break(SectionKind::Choho),
                b.section_break(SectionKind::Choho),
            ),
            (
                {
                    let text = o.content_plain("h");
                    o.aozora_heading(AozoraHeadingKind::Window, text)
                },
                {
                    let text = b.content_plain("h");
                    b.aozora_heading(AozoraHeadingKind::Window, text)
                },
            ),
            (o.heading_hint(1, "t"), b.heading_hint(1, "t")),
            (
                {
                    let cap = o.content_plain("c");
                    o.sashie("f.png", Some(cap))
                },
                {
                    let cap = b.content_plain("c");
                    b.sashie("f.png", Some(cap))
                },
            ),
            (o.kaeriten("一"), b.kaeriten("一")),
            (
                {
                    let p = o.make_annotation("r", AnnotationKind::Unknown);
                    o.annotation(p)
                },
                {
                    let p = b.make_annotation("r", AnnotationKind::Unknown);
                    b.annotation(p)
                },
            ),
            (
                {
                    let c = o.content_plain("d");
                    o.double_ruby(c)
                },
                {
                    let c = b.content_plain("d");
                    b.double_ruby(c)
                },
            ),
            (
                o.container(Container {
                    kind: ContainerKind::Indent { amount: 1 },
                }),
                b.container(Container {
                    kind: ContainerKind::Indent { amount: 1 },
                }),
            ),
        ];

        for (owned_n, borrowed_n) in &pairs {
            assert_eq!(
                owned_n.xml_node_name(),
                borrowed_n.xml_node_name(),
                "xml_node_name diverged for variant {owned_n:?}"
            );
            assert_eq!(
                owned_n.is_block(),
                borrowed_n.is_block(),
                "is_block diverged for variant {owned_n:?}"
            );
            assert_eq!(
                owned_n.contains_inlines(),
                borrowed_n.contains_inlines(),
                "contains_inlines diverged for variant {owned_n:?}"
            );
        }

        // Pin the variant count so a future enum addition forces the
        // cross-allocator sweep to be updated.
        assert_eq!(pairs.len(), 17, "AozoraNode has 17 variants today");
    }
}
