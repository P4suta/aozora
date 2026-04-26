//! Visitor trait for borrowed-AST tree walking (Innovation I-10).
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
//! - `AozoraVisitor<'src>` has one method per [`AozoraNode`] variant
//!   plus container open / close. Default impls are no-ops so a
//!   visitor that only cares about a subset of variants stays terse.
//! - `dispatch_node` routes a `borrowed::AozoraNode<'src>` through
//!   the visitor. Mirrors the legacy `render_node::render` enter /
//!   exit semantics: containers fire open on enter and close on
//!   exit; every other variant ignores the exit pass.
//! - Higher-level walkers (`html::render_into` and
//!   `serialize::serialize_into`) drive `dispatch_node` for every
//!   sentinel they encounter in the lex output's normalised text.

use core::fmt;

use aozora_syntax::borrowed::{
    Annotation, AozoraHeading, AozoraNode, Bouten, DoubleRuby, Gaiji, HeadingHint, Kaeriten,
    Ruby, Sashie, TateChuYoko, Warichu,
};
use aozora_syntax::{AlignEnd, Container, Indent, Keigakomi, SectionKind};

/// Tree-walker visitor for borrowed Aozora AST nodes.
///
/// All methods take `&mut self` so a visitor can carry mutable
/// state (output buffer, escape policy, depth counter, …). Default
/// impls are no-ops; implementors override only the variants they
/// produce output for. The `'src` lifetime mirrors the borrowed-AST
/// lifetime — node payloads borrow from the same arena that the
/// `BorrowedLexOutput` borrows from.
///
/// # Errors
///
/// Methods return `fmt::Result` so visitors that write to a
/// [`fmt::Write`] sink can propagate I/O errors. Visitors with
/// infallible state (e.g., counting visits) can ignore the result.
#[allow(unused_variables, reason = "default no-op impls; downstream visitors override per-variant")]
pub trait AozoraVisitor<'src> {
    fn visit_ruby(&mut self, r: &Ruby<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_bouten(&mut self, b: &Bouten<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_tate_chu_yoko(&mut self, t: &TateChuYoko<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_gaiji(&mut self, g: &Gaiji<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_indent(&mut self, i: Indent) -> fmt::Result {
        Ok(())
    }
    fn visit_align_end(&mut self, a: AlignEnd) -> fmt::Result {
        Ok(())
    }
    fn visit_warichu(&mut self, w: &Warichu<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_keigakomi(&mut self, k: Keigakomi) -> fmt::Result {
        Ok(())
    }
    fn visit_page_break(&mut self) -> fmt::Result {
        Ok(())
    }
    fn visit_section_break(&mut self, k: SectionKind) -> fmt::Result {
        Ok(())
    }
    fn visit_aozora_heading(&mut self, h: &AozoraHeading<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_heading_hint(&mut self, h: &HeadingHint<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_sashie(&mut self, s: &Sashie<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_kaeriten(&mut self, k: &Kaeriten<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_annotation(&mut self, a: &Annotation<'src>) -> fmt::Result {
        Ok(())
    }
    fn visit_double_ruby(&mut self, d: &DoubleRuby<'src>) -> fmt::Result {
        Ok(())
    }
    /// Container-open event. Fires on the entering pass for
    /// `AozoraNode::Container` nodes; the corresponding
    /// `visit_container_close` fires on exit.
    fn visit_container_open(&mut self, c: Container) -> fmt::Result {
        Ok(())
    }
    fn visit_container_close(&mut self, c: Container) -> fmt::Result {
        Ok(())
    }
}

/// Dispatch a single borrowed [`AozoraNode`] through the visitor,
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
    node: AozoraNode<'src>,
    entering: bool,
    v: &mut V,
) -> fmt::Result {
    match node {
        AozoraNode::Container(c) => {
            if entering {
                v.visit_container_open(c)
            } else {
                v.visit_container_close(c)
            }
        }
        _ if !entering => Ok(()),
        AozoraNode::Ruby(r) => v.visit_ruby(r),
        AozoraNode::Bouten(b) => v.visit_bouten(b),
        AozoraNode::TateChuYoko(t) => v.visit_tate_chu_yoko(t),
        AozoraNode::Gaiji(g) => v.visit_gaiji(g),
        AozoraNode::Indent(i) => v.visit_indent(i),
        AozoraNode::AlignEnd(a) => v.visit_align_end(a),
        AozoraNode::Warichu(w) => v.visit_warichu(w),
        AozoraNode::Keigakomi(k) => v.visit_keigakomi(k),
        AozoraNode::PageBreak => v.visit_page_break(),
        AozoraNode::SectionBreak(k) => v.visit_section_break(k),
        AozoraNode::AozoraHeading(h) => v.visit_aozora_heading(h),
        AozoraNode::HeadingHint(h) => v.visit_heading_hint(h),
        AozoraNode::Sashie(s) => v.visit_sashie(s),
        AozoraNode::Kaeriten(k) => v.visit_kaeriten(k),
        AozoraNode::Annotation(a) => v.visit_annotation(a),
        AozoraNode::DoubleRuby(d) => v.visit_double_ruby(d),
        // `AozoraNode` is `#[non_exhaustive]`; future variants no-op
        // until a visitor method is added for them.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::alloc::{BorrowedAllocator, NodeAllocator};
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
        let borrowed_ruby = alloc.ruby(base, reading, false);
        let mut counter = Counter::default();
        dispatch_node(borrowed_ruby, true, &mut counter).unwrap();
        dispatch_node(AozoraNode::PageBreak, true, &mut counter).unwrap();
        dispatch_node(
            AozoraNode::Container(Container {
                kind: aozora_syntax::ContainerKind::Keigakomi,
            }),
            true,
            &mut counter,
        )
        .unwrap();
        dispatch_node(
            AozoraNode::Container(Container {
                kind: aozora_syntax::ContainerKind::Keigakomi,
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
        dispatch_node(AozoraNode::PageBreak, false, &mut counter).unwrap();
        assert_eq!(counter.page_breaks, 0);
    }

    #[test]
    fn unimplemented_methods_default_to_noop() {
        // `Counter` doesn't override visit_section_break — calling
        // it must not panic and must not affect any other counter.
        let mut counter = Counter::default();
        dispatch_node(
            AozoraNode::SectionBreak(SectionKind::Choho),
            true,
            &mut counter,
        )
        .unwrap();
        assert_eq!(counter.rubies, 0);
        assert_eq!(counter.any_other, 0);
    }
}
