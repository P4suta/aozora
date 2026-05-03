//! Snapshot the borrowed-AST `Debug` representation for a canonical
//! sample of every node variant.
//!
//! The byte-identical render gate (`aozora-conformance::render_gate`)
//! pins HTML output drift; this test pins **AST shape drift**. A
//! field rename, variant reordering, or `Debug` derive change that
//! does not affect HTML still surfaces as a snapshot diff. Reviewers
//! see the exact shape change in `cargo insta review` (or the
//! `*.snap.new` diff) and can decide whether to accept.
//!
//! Coverage rationale: one node per variant, hand-built through
//! [`BorrowedAllocator`]. Constructed nodes use minimal placeholder
//! payloads so the snapshot stays focused on the *shape* (which
//! fields are present, in what order, what types) rather than on
//! incidental string content.

use aozora_encoding::gaiji::Resolved;
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::{AozoraNode, Arena};
use aozora_syntax::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, Container,
    ContainerKind, Indent, Keigakomi, SectionKind,
};

fn build_one_of_each<'a>(alloc: &mut BorrowedAllocator<'a>) -> Vec<AozoraNode<'a>> {
    let base = alloc.content_plain("base");
    let reading = alloc.content_plain("よみ");
    let upper = alloc.content_plain("up");
    let lower = alloc.content_plain("lo");
    let g = alloc.make_gaiji("木＋吶", Some(Resolved::Char('A')), Some("第3水準"));
    let a = alloc.make_annotation("annotation", AnnotationKind::Unknown);

    vec![
        alloc.ruby(base, reading, true),
        alloc.bouten(BoutenKind::Goma, base, BoutenPosition::Right),
        alloc.tate_chu_yoko(base),
        alloc.gaiji(g),
        alloc.indent(Indent { amount: 2 }),
        alloc.align_end(AlignEnd { offset: 2 }),
        alloc.warichu(upper, lower),
        alloc.keigakomi(Keigakomi),
        alloc.page_break(),
        alloc.section_break(SectionKind::Choho),
        alloc.aozora_heading(AozoraHeadingKind::Window, base),
        alloc.heading_hint(2, "対象"),
        alloc.sashie("file.png", None),
        alloc.kaeriten("一"),
        alloc.annotation(a),
        alloc.double_ruby(base),
        alloc.container(Container {
            kind: ContainerKind::Indent { amount: 1 },
        }),
    ]
}

#[test]
fn snapshot_one_of_each_aozora_node() {
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::with_capacity(&arena, 32);
    let nodes = build_one_of_each(&mut alloc);
    insta::assert_snapshot!(format!("{nodes:#?}"));
}
