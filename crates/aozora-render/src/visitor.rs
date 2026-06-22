//! Visitor trait for borrowed-AST tree walking.
//!
//! The HTML and Aozora-source renderers in this crate share an
//! identical *traversal* — walk the normalized text, dispatch each
//! sentinel through the registry, recurse into per-node payloads —
//! and differ only in *what bytes they emit at each node*. The
//! visitor trait factors out the dispatch so a third renderer (TeX,
//! EPUB, JSON, …) becomes a single trait-impl addition rather than
//! a re-implementation of the block walker + per-node dispatch.
//!
//! # Design
//!
//! - `AozoraVisitor<'src>` has one method per [`Node`] variant
//!   plus container open / close. Default impls are no-ops so a
//!   visitor that only cares about a subset of variants stays terse.
//! - `dispatch_node` routes a `borrowed::Node<'src>` through
//!   the visitor. Mirrors the legacy `render_node::render` enter /
//!   exit semantics: containers fire open on enter and close on
//!   exit; every other variant ignores the exit pass.
//! - Higher-level walkers (`html::render_into` and
//!   `serialize::serialize_into`) drive `dispatch_node` for every
//!   sentinel they encounter in the lex output's normalised text.

use core::fmt;

use aozora_syntax::borrowed::{
    AngleQuote, Bouten, CombineUpright, Directive, Gaiji, Heading, HeadingHint, Illustration,
    Kaeriten, MarginNote, Node, Ruby, Warichu,
};
use aozora_syntax::{AlignEnd, Center, Container, Framed, Indent, SectionKind};

/// Tree-walker visitor for borrowed Aozora AST nodes.
///
/// All methods take `&mut self` so a visitor can carry mutable
/// state (output buffer, escape policy, depth counter, …). Default
/// impls are no-ops; implementors override only the variants they
/// produce output for. The `'src` lifetime mirrors the borrowed-AST
/// lifetime — node payloads borrow from the same arena that the
/// `LexOutput` borrows from.
///
/// # Errors
///
/// Methods return `fmt::Result` so visitors that write to a
/// [`fmt::Write`] sink can propagate I/O errors. Visitors with
/// infallible state (e.g., counting visits) can ignore the result.
#[allow(
    unused_variables,
    reason = "default no-op impls; downstream visitors override per-variant"
)]
#[allow(
    clippy::missing_errors_doc,
    reason = "every visit_* method shares the trait-level # Errors contract above (propagates the underlying fmt::Write sink's error). Per-method # Errors lines would be 17 redundant duplicates."
)]
pub trait AozoraVisitor<'src> {
    /// Visit a ruby (furigana) node: a `base` run with its `reading`.
    fn visit_ruby(&mut self, r: &Ruby<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a side annotation — 注記 / 傍記, a left-side gloss or
    /// redaction marker attached to a preceding run. Named `side_note`;
    /// the payload type is [`MarginNote`].
    fn visit_side_note(&mut self, s: &MarginNote<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a 傍点 / 傍線 node: emphasis dots or sidelines over `target`.
    fn visit_bouten(&mut self, b: &Bouten<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a 縦中横 (tate-chu-yoko) node — a short run set horizontally
    /// inside vertical text. Named `tate_chu_yoko`; the payload type is
    /// [`CombineUpright`].
    fn visit_tate_chu_yoko(&mut self, t: &CombineUpright<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a gaiji node — a glyph outside the character range, carrying
    /// its source description and any resolved Unicode / mencode reference.
    fn visit_gaiji(&mut self, g: &Gaiji<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a 字下げ (indent) marker opening an indented run of `amount`
    /// chars. A leaf event; the indented body follows as sibling nodes.
    fn visit_indent(&mut self, i: Indent) -> fmt::Result {
        Ok(())
    }
    /// Visit a 地付き / 地上げ (bottom-aligned) marker. `offset` is the gap
    /// from the right edge in chars (`0` = 地付き, `n` = n 字上げ).
    fn visit_align_end(&mut self, a: AlignEnd) -> fmt::Result {
        Ok(())
    }
    /// Visit a single-line centring marker — `中央揃え` or, when
    /// [`Center::page`] is set, `ページの左右中央`.
    fn visit_center(&mut self, c: Center) -> fmt::Result {
        Ok(())
    }
    /// Visit a 割り注 (warichu) node — a two-line split annotation with
    /// `upper` and `lower` rows.
    fn visit_warichu(&mut self, w: &Warichu<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a 罫囲み (keigakomi) marker — a ruled box around the run.
    /// Named `keigakomi`; the payload type is the unit struct [`Framed`].
    fn visit_keigakomi(&mut self, k: Framed) -> fmt::Result {
        Ok(())
    }
    /// Visit a 改ページ (page break) marker. Takes no payload.
    fn visit_page_break(&mut self) -> fmt::Result {
        Ok(())
    }
    /// Visit a section break — 改丁 / 改段 / 改見開き, selected by `k`.
    fn visit_section_break(&mut self, k: SectionKind) -> fmt::Result {
        Ok(())
    }
    /// Visit an Aozora heading — a 大 / 中 / 小 level paired with a style
    /// (standard / 同行 / 窓) and its label. Named `aozora_heading`; the
    /// payload type is [`Heading`].
    fn visit_aozora_heading(&mut self, h: &Heading<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a forward-reference heading hint — the intended outline
    /// `level` and `style` for a quoted target that was promoted to a
    /// heading.
    fn visit_heading_hint(&mut self, h: &HeadingHint<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a 挿絵 (illustration) node — image path plus optional number,
    /// dimensions, caption, and alt description. Named `sashie`; the
    /// payload type is [`Illustration`].
    fn visit_sashie(&mut self, s: &Illustration<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a 返り点 (kaeriten) node — a Chinese-reading-order mark.
    fn visit_kaeriten(&mut self, k: &Kaeriten<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a generic annotation — an Aozora-shaped `［＃…］` directive
    /// (sic / 底本では / unknown, …) carrying its raw body. Named
    /// `annotation`; the payload type is [`Directive`].
    fn visit_annotation(&mut self, a: &Directive<'src>) -> fmt::Result {
        Ok(())
    }
    /// Visit a double-angle quotation node — source `≪…≫`, displayed `《…》`.
    fn visit_angle_quote(&mut self, d: &AngleQuote<'src>) -> fmt::Result {
        Ok(())
    }
    /// Container-open event. Fires on the entering pass for
    /// `Node::Container` nodes; the corresponding
    /// `visit_container_close` fires on exit.
    fn visit_container_open(&mut self, c: Container) -> fmt::Result {
        Ok(())
    }
    /// Container-close event. Fires on the exiting pass for
    /// `Node::Container` nodes, pairing the earlier
    /// `visit_container_open`. Carries the same [`Container`] so the
    /// visitor can emit the matching close markup.
    fn visit_container_close(&mut self, c: Container) -> fmt::Result {
        Ok(())
    }
}

/// Dispatch a single borrowed [`Node`] through the visitor,
/// honouring the standard enter / exit convention.
///
/// `entering = true` fires the per-variant `visit_*` method.
/// `entering = false` is a no-op for every variant except
/// `Container`, which then fires `visit_container_close`.
///
/// # Errors
///
/// Propagates the visitor method's `fmt::Result`.
pub fn dispatch_node<'src, V: AozoraVisitor<'src>>(
    node: Node<'src>,
    entering: bool,
    v: &mut V,
) -> fmt::Result {
    match node {
        Node::Container(c) => {
            if entering {
                v.visit_container_open(c)
            } else {
                v.visit_container_close(c)
            }
        }
        _ if !entering => Ok(()),
        Node::Ruby(r) => v.visit_ruby(r),
        Node::MarginNote(s) => v.visit_side_note(s),
        Node::Bouten(b) => v.visit_bouten(b),
        Node::CombineUpright(t) => v.visit_tate_chu_yoko(t),
        Node::Gaiji(g) => v.visit_gaiji(g),
        Node::Indent(i) => v.visit_indent(i),
        Node::AlignEnd(a) => v.visit_align_end(a),
        Node::Center(c) => v.visit_center(c),
        Node::Warichu(w) => v.visit_warichu(w),
        Node::Framed(k) => v.visit_keigakomi(k),
        Node::PageBreak => v.visit_page_break(),
        Node::SectionBreak(k) => v.visit_section_break(k),
        Node::Heading(h) => v.visit_aozora_heading(h),
        Node::HeadingHint(h) => v.visit_heading_hint(h),
        Node::Illustration(s) => v.visit_sashie(s),
        Node::Kaeriten(k) => v.visit_kaeriten(k),
        Node::Directive(a) => v.visit_annotation(a),
        Node::AngleQuote(d) => v.visit_angle_quote(d),
        // `Node` is `#[non_exhaustive]`; future variants no-op
        // until a visitor method is added for them.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::alloc::BorrowedAllocator;
    use aozora_syntax::borrowed::Arena;

    /// Demonstration visitor: count one tick per node visited. Proves
    /// that adding a new "renderer" via the trait is a one-impl
    /// extension — no walker, no dispatch boilerplate.
    #[derive(Default)]
    struct Counter {
        rubies: usize,
        page_breaks: usize,
        containers_opened: usize,
        containers_closed: usize,
        any_other: usize,
    }

    impl<'src> AozoraVisitor<'src> for Counter {
        fn visit_ruby(&mut self, _r: &Ruby<'src>) -> fmt::Result {
            self.rubies += 1;
            Ok(())
        }
        fn visit_page_break(&mut self) -> fmt::Result {
            self.page_breaks += 1;
            Ok(())
        }
        fn visit_container_open(&mut self, _c: Container) -> fmt::Result {
            self.containers_opened += 1;
            Ok(())
        }
        fn visit_container_close(&mut self, _c: Container) -> fmt::Result {
            self.containers_closed += 1;
            Ok(())
        }
        fn visit_bouten(&mut self, _b: &Bouten<'src>) -> fmt::Result {
            self.any_other += 1;
            Ok(())
        }
        fn visit_gaiji(&mut self, _g: &Gaiji<'src>) -> fmt::Result {
            self.any_other += 1;
            Ok(())
        }
    }

    #[test]
    fn counter_visitor_tracks_each_kind() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let base = alloc.content_plain("x");
        let reading = alloc.content_plain("y");
        let borrowed_ruby = alloc.ruby(base, reading);
        let mut counter = Counter::default();
        dispatch_node(borrowed_ruby, true, &mut counter).unwrap();
        dispatch_node(Node::PageBreak, true, &mut counter).unwrap();
        dispatch_node(
            Node::Container(Container {
                kind: aozora_syntax::ContainerKind::Framed,
            }),
            true,
            &mut counter,
        )
        .unwrap();
        dispatch_node(
            Node::Container(Container {
                kind: aozora_syntax::ContainerKind::Framed,
            }),
            false,
            &mut counter,
        )
        .unwrap();
        assert_eq!(counter.rubies, 1);
        assert_eq!(counter.page_breaks, 1);
        assert_eq!(counter.containers_opened, 1);
        assert_eq!(counter.containers_closed, 1);
    }

    #[test]
    fn exit_pass_is_noop_for_non_container_variants() {
        let mut counter = Counter::default();
        dispatch_node(Node::PageBreak, false, &mut counter).unwrap();
        assert_eq!(counter.page_breaks, 0);
    }

    #[test]
    fn unimplemented_methods_default_to_noop() {
        // `Counter` doesn't override visit_section_break — calling
        // it must not panic and must not affect any other counter.
        let mut counter = Counter::default();
        dispatch_node(Node::SectionBreak(SectionKind::Kaicho), true, &mut counter).unwrap();
        assert_eq!(counter.rubies, 0, "ruby counter must stay untouched");
        assert_eq!(counter.any_other, 0, "other counter must stay untouched");
    }

    // -------------------------------------------------------------------
    // Full dispatch coverage: a recorder visitor that overrides *every*
    // `visit_*` method and pushes a stable tag, so `dispatch_node` can be
    // asserted to route each `Node` variant to the matching method.
    // -------------------------------------------------------------------

    /// Records the tag of each visit in call order.
    #[derive(Default)]
    struct Recorder {
        log: Vec<&'static str>,
    }

    impl<'src> AozoraVisitor<'src> for Recorder {
        fn visit_ruby(&mut self, _r: &Ruby<'src>) -> fmt::Result {
            self.log.push("ruby");
            Ok(())
        }
        fn visit_side_note(&mut self, _s: &MarginNote<'src>) -> fmt::Result {
            self.log.push("side_note");
            Ok(())
        }
        fn visit_bouten(&mut self, _b: &Bouten<'src>) -> fmt::Result {
            self.log.push("bouten");
            Ok(())
        }
        fn visit_tate_chu_yoko(&mut self, _t: &CombineUpright<'src>) -> fmt::Result {
            self.log.push("tcy");
            Ok(())
        }
        fn visit_gaiji(&mut self, _g: &Gaiji<'src>) -> fmt::Result {
            self.log.push("gaiji");
            Ok(())
        }
        fn visit_indent(&mut self, _i: Indent) -> fmt::Result {
            self.log.push("indent");
            Ok(())
        }
        fn visit_align_end(&mut self, _a: AlignEnd) -> fmt::Result {
            self.log.push("align_end");
            Ok(())
        }
        fn visit_center(&mut self, _c: Center) -> fmt::Result {
            self.log.push("center");
            Ok(())
        }
        fn visit_warichu(&mut self, _w: &Warichu<'src>) -> fmt::Result {
            self.log.push("warichu");
            Ok(())
        }
        fn visit_keigakomi(&mut self, _k: Framed) -> fmt::Result {
            self.log.push("keigakomi");
            Ok(())
        }
        fn visit_page_break(&mut self) -> fmt::Result {
            self.log.push("page_break");
            Ok(())
        }
        fn visit_section_break(&mut self, _k: SectionKind) -> fmt::Result {
            self.log.push("section_break");
            Ok(())
        }
        fn visit_aozora_heading(&mut self, _h: &Heading<'src>) -> fmt::Result {
            self.log.push("aozora_heading");
            Ok(())
        }
        fn visit_heading_hint(&mut self, _h: &HeadingHint<'src>) -> fmt::Result {
            self.log.push("heading_hint");
            Ok(())
        }
        fn visit_sashie(&mut self, _s: &Illustration<'src>) -> fmt::Result {
            self.log.push("sashie");
            Ok(())
        }
        fn visit_kaeriten(&mut self, _k: &Kaeriten<'src>) -> fmt::Result {
            self.log.push("kaeriten");
            Ok(())
        }
        fn visit_annotation(&mut self, _a: &Directive<'src>) -> fmt::Result {
            self.log.push("annotation");
            Ok(())
        }
        fn visit_angle_quote(&mut self, _d: &AngleQuote<'src>) -> fmt::Result {
            self.log.push("angle_quote");
            Ok(())
        }
        fn visit_container_open(&mut self, _c: Container) -> fmt::Result {
            self.log.push("container_open");
            Ok(())
        }
        fn visit_container_close(&mut self, _c: Container) -> fmt::Result {
            self.log.push("container_close");
            Ok(())
        }
    }

    /// Build one of every dispatchable `Node` variant (skipping
    /// `Emphasis`, which `dispatch_node` deliberately has no visit method
    /// for — its enter pass no-ops via the `#[non_exhaustive]` `_` arm).
    #[allow(
        clippy::too_many_lines,
        reason = "exhaustive variant construction for dispatch coverage"
    )]
    fn dispatch_every_variant_into(recorder: &mut Recorder) {
        use aozora_syntax::{
            BoutenKind, BoutenPosition, ContainerKind, DirectiveKind, HeadingKind, HeadingStyle,
            MarginNoteKind,
        };

        let arena = Arena::new();
        let mut a = BorrowedAllocator::new(&arena);

        let base = a.content_plain("基");
        let reading = a.content_plain("よ");
        let ruby = a.ruby(base, reading);

        let nbase = a.content_plain("底");
        let note = a.content_plain("注");
        let side_note = a.side_note(MarginNoteKind::Gloss, nbase, note);

        let btarget = a.content_plain("点");
        let bouten = a.bouten(BoutenKind::Goma, btarget, BoutenPosition::Right, false);

        let ttext = a.content_plain("12");
        let tcy = a.tate_chu_yoko(ttext, false);

        let g = a.make_gaiji("外字", None, false);
        let gaiji = a.gaiji(g);

        let indent = a.indent(Indent { amount: 2 });
        let align_end = a.align_end(AlignEnd { offset: 1 });
        let center = a.center(Center { page: true });

        let upper = a.content_plain("上");
        let lower = a.content_plain("下");
        let warichu = a.warichu(upper, lower);

        let keigakomi = a.keigakomi(Framed);
        let page_break = a.page_break();
        let section_break = a.section_break(SectionKind::Kaicho);

        let htext = a.content_plain("見出し");
        let heading = a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, htext);
        let heading_hint = a.heading_hint(HeadingKind::Large, HeadingStyle::Standard, "対象");

        let sashie = a.sashie("fig.png", None, None, None);
        let kaeriten = a.kaeriten("レ");

        let ann_payload = a.make_directive("［＃X］", DirectiveKind::Unknown);
        let annotation = a.annotation(ann_payload);

        let qcontent = a.content_plain("引用");
        let angle_quote = a.angle_quote(qcontent);

        let container = a.container(Container {
            kind: ContainerKind::Framed,
        });

        let nodes = [
            ruby,
            side_note,
            bouten,
            tcy,
            gaiji,
            indent,
            align_end,
            center,
            warichu,
            keigakomi,
            page_break,
            section_break,
            heading,
            heading_hint,
            sashie,
            kaeriten,
            annotation,
            angle_quote,
        ];
        for n in nodes {
            dispatch_node(n, true, recorder).expect("dispatch into Recorder never fails");
        }
        // Container enter / exit drive open then close.
        dispatch_node(container, true, recorder).expect("container open dispatch");
        dispatch_node(container, false, recorder).expect("container close dispatch");
    }

    #[test]
    fn dispatch_routes_every_variant_to_its_method() {
        let mut recorder = Recorder::default();
        dispatch_every_variant_into(&mut recorder);
        assert_eq!(
            recorder.log,
            vec![
                "ruby",
                "side_note",
                "bouten",
                "tcy",
                "gaiji",
                "indent",
                "align_end",
                "center",
                "warichu",
                "keigakomi",
                "page_break",
                "section_break",
                "aozora_heading",
                "heading_hint",
                "sashie",
                "kaeriten",
                "annotation",
                "angle_quote",
                "container_open",
                "container_close",
            ],
            "dispatch_node must route each variant to its matching visit_* method"
        );
    }

    #[test]
    fn exit_pass_is_noop_for_every_non_container_variant() {
        // The single `_ if !entering => Ok(())` arm short-circuits every
        // leaf / inline node on the exit pass; only Container fires close.
        let arena = Arena::new();
        let mut a = BorrowedAllocator::new(&arena);
        let base = a.content_plain("基");
        let reading = a.content_plain("よ");
        let ruby = a.ruby(base, reading);
        let btarget = a.content_plain("点");
        let bouten = a.bouten(
            aozora_syntax::BoutenKind::Goma,
            btarget,
            aozora_syntax::BoutenPosition::Right,
            false,
        );
        let mut recorder = Recorder::default();
        for n in [ruby, bouten, Node::PageBreak] {
            dispatch_node(n, false, &mut recorder).expect("exit dispatch never fails");
        }
        assert!(
            recorder.log.is_empty(),
            "exit pass must be a no-op for non-container variants, got {:?}",
            recorder.log
        );
    }

    #[test]
    fn emphasis_variant_has_no_visit_method_and_no_ops() {
        // `dispatch_node` has no `visit_emphasis` arm — an Emphasis node
        // falls through the `#[non_exhaustive]` catch-all on both passes.
        let arena = Arena::new();
        let mut a = BorrowedAllocator::new(&arena);
        let text = a.content_plain("重要");
        let emphasis = a.emphasis(aozora_syntax::EmphasisKind::Bold, text, false);
        let mut recorder = Recorder::default();
        dispatch_node(emphasis, true, &mut recorder).expect("enter dispatch never fails");
        dispatch_node(emphasis, false, &mut recorder).expect("exit dispatch never fails");
        assert!(
            recorder.log.is_empty(),
            "Emphasis has no visit method and must no-op, got {:?}",
            recorder.log
        );
    }

    /// A visitor that uses only the default (no-op) trait methods — proves
    /// the default impls compile, dispatch, and never error for every
    /// variant.
    struct DefaultsOnly;
    impl AozoraVisitor<'_> for DefaultsOnly {}

    #[test]
    fn default_trait_impls_cover_every_variant_without_error() {
        let mut visitor = DefaultsOnly;
        let arena = Arena::new();
        let mut a = BorrowedAllocator::new(&arena);

        // A representative spread covering both passes through the default
        // no-op methods (which return Ok(()) for every variant).
        let base = a.content_plain("基");
        let reading = a.content_plain("よ");
        let ruby = a.ruby(base, reading);
        dispatch_node(ruby, true, &mut visitor).expect("default visit_ruby");
        dispatch_node(Node::PageBreak, true, &mut visitor).expect("default visit_page_break");
        let container = a.container(Container {
            kind: aozora_syntax::ContainerKind::Framed,
        });
        dispatch_node(container, true, &mut visitor).expect("default container_open");
        dispatch_node(container, false, &mut visitor).expect("default container_close");
    }
}
