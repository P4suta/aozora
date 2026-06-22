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

use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::{Arena, Node};
use aozora_syntax::{
    AlignEnd, BoutenKind, BoutenPosition, Center, Container, ContainerKind, DirectiveKind, Framed,
    HeadingKind, HeadingStyle, Indent, IndentLayout, MarginNoteKind, SectionKind,
};

fn build_one_of_each<'a>(alloc: &mut BorrowedAllocator<'a>) -> Vec<Node<'a>> {
    let base = alloc.content_plain("base");
    let reading = alloc.content_plain("よみ");
    let upper = alloc.content_plain("up");
    let lower = alloc.content_plain("lo");
    let g = alloc.make_gaiji("木＋吶", Some("第3水準"), false);
    let a = alloc.make_directive("annotation", DirectiveKind::Unknown);

    vec![
        alloc.ruby(base, reading),
        // Both MarginNote flavours — pin each kind's Debug shape.
        alloc.side_note(MarginNoteKind::Gloss, base, reading),
        alloc.side_note(MarginNoteKind::Marginal, base, reading),
        alloc.bouten(BoutenKind::Goma, base, BoutenPosition::Right, false),
        alloc.tate_chu_yoko(base, false),
        alloc.gaiji(g),
        alloc.indent(Indent { amount: 2 }),
        alloc.align_end(AlignEnd { offset: 2 }),
        alloc.center(Center { page: true }),
        alloc.warichu(upper, lower),
        alloc.keigakomi(Framed),
        alloc.page_break(),
        alloc.section_break(SectionKind::Kaicho),
        alloc.aozora_heading(HeadingKind::Medium, HeadingStyle::Window, base),
        alloc.heading_hint(HeadingKind::Medium, HeadingStyle::SameLine, "対象"),
        alloc.sashie("file.png", None, None, None),
        alloc.kaeriten("一"),
        alloc.annotation(a),
        alloc.angle_quote(base),
        alloc.container(Container {
            kind: ContainerKind::Indent {
                amount: 1,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
            },
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
