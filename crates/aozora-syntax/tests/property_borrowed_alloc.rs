//! Allocator + node-tag invariants for the borrowed AST.
//!
//! [`BorrowedAllocator`] sits between the lexer and the rendered AST —
//! every node ever constructed flows through it. Two invariants gate
//! the whole stack downstream:
//!
//! 1. **Interner dedup**: byte-equal strings return byte-equal `&str`
//!    references (pointer equality at minimum). Renderers that
//!    pointer-key on intern hits would silently double-allocate if the
//!    interner ever returned distinct pointers for byte-equal input.
//! 2. **`xml_node_name` injectivity** over [`AozoraNode`] variants:
//!    every variant produces a distinct stable name. A duplicate would
//!    silently make two AST shapes render to the same XML element,
//!    collapsing snapshot diffs to a no-op.
//!
//! Negative property: a `kind()` ↔ `xml_node_name()` mapping is
//! single-valued — same variant always produces same kind, same XML
//! name, regardless of the payload.

use aozora_encoding::gaiji::Resolved;
use aozora_proptest::config::default_config;
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::{Arena, Content};
use aozora_syntax::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, Container,
    ContainerKind, Indent, Keigakomi, SectionKind,
};
use proptest::prelude::*;

// ----------------------------------------------------------------------
// Hand-curated injectivity check — exhaustive over the 17 variants.
// Lives as a unit test (not a proptest) because the variant set is
// finite and small; the value is in the *exhaustive* sweep, not in
// shrinker-driven discovery.
// ----------------------------------------------------------------------

#[test]
fn xml_node_name_is_injective_over_all_variants() {
    let arena = Arena::new();
    let mut alloc = BorrowedAllocator::with_capacity(&arena, 16);
    let base = alloc.content_plain("base");
    let reading = alloc.content_plain("よみ");
    let upper = alloc.content_plain("up");
    let lower = alloc.content_plain("lo");
    let g = alloc.make_gaiji("木＋吶", Some(Resolved::Char('A')), Some("第3水準"));
    let a = alloc.make_annotation("annotation", AnnotationKind::Unknown);

    let nodes = [
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
    ];

    let mut names: Vec<&'static str> = nodes
        .iter()
        .map(aozora_syntax::borrowed::AozoraNode::xml_node_name)
        .collect();
    names.sort_unstable();
    let len_before = names.len();
    names.dedup();
    assert_eq!(
        len_before,
        names.len(),
        "xml_node_name collision among AozoraNode variants: {names:?}"
    );

    // `kind()` is similarly injective — no two variants project to
    // the same `NodeKind`. Catches a `NodeKind` enum variant that
    // ever doubles up on a discriminant.
    let mut kinds: Vec<aozora_syntax::NodeKind> = nodes
        .iter()
        .map(aozora_syntax::borrowed::AozoraNode::kind)
        .collect();
    kinds.sort_by_key(|k| k.as_camel_case());
    let len_kinds_before = kinds.len();
    kinds.dedup();
    assert_eq!(
        len_kinds_before,
        kinds.len(),
        "NodeKind collision among AozoraNode variants"
    );
}

// ----------------------------------------------------------------------
// Property tests for the interner dedup invariant.
// ----------------------------------------------------------------------

fn plain_pointer(c: Content<'_>) -> Option<*const u8> {
    if let Content::Plain(p) = c {
        Some(p.as_ptr())
    } else {
        None
    }
}

proptest! {
    #![proptest_config(default_config())]

    /// Interning the same string twice through `content_plain` returns
    /// byte-equal slices that point at the same arena byte. Pointer
    /// equality is a tighter property than `==` and gates the
    /// allocator's dedup invariant directly.
    #[test]
    fn intern_dedups_byte_equal_strings(s in "[a-zA-Z0-9]{1,32}") {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::with_capacity(&arena, 16);
        let first = alloc.content_plain(&s);
        let second = alloc.content_plain(&s);
        let p1 = plain_pointer(first).expect("Plain expected");
        let p2 = plain_pointer(second).expect("Plain expected");
        prop_assert_eq!(p1, p2, "interner returned distinct pointers for byte-equal input {:?}", s);
    }

    /// Distinct strings produce distinct interned `Content::Plain`
    /// payloads. Catches a hash collision masked by an `==` rather
    /// than `as_ptr ==` comparison in the interner — a regression
    /// that would silently fold two inputs into one allocation.
    #[test]
    fn intern_keeps_distinct_strings_distinct(
        a in "[a-z]{1,16}",
        b in "[a-z]{1,16}",
    ) {
        prop_assume!(a != b);
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::with_capacity(&arena, 16);
        let ca = alloc.content_plain(&a);
        let cb = alloc.content_plain(&b);
        prop_assert_ne!(ca, cb);
    }
}
