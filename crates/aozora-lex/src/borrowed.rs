//! Arena-emitting lex API (Plan B.2).
//!
//! Produces a [`BorrowedLexOutput<'a>`] whose normalized text and
//! placeholder registry live entirely inside an external [`Arena`].
//! Drop the arena, and the entire lex output (every node, every
//! borrowed string, every registry table) deallocates in a single
//! step — no per-node `Drop` ever runs, no scattered `Box::drop`
//! malloc traffic on the way out.
//!
//! ## Pipeline (today)
//!
//! 1. Run the legacy [`crate::lex`] pipeline (which still owns the
//!    Box-allocated AST internally).
//! 2. Convert each registry node into the arena via
//!    [`aozora_syntax::convert::to_borrowed`].
//! 3. Wrap the resulting `(u32, borrowed::AozoraNode<'a>)` lists in
//!    [`aozora_veb::EytzingerMap`] for cache-friendly lookup.
//!
//! Step 1 is heap-allocating; steps 2–3 collapse the heap tree into
//! the arena. The conversion pass is a single linear walk of the
//! registry vectors, expected ~100 µs for a 2 MB document with ~2,700
//! nodes.
//!
//! ## Future migration
//!
//! Plan B's later steps fold the conversion away: the lex pipeline
//! grows native arena-aware classifiers so steps 1 and 2 collapse
//! into one allocate-into-the-arena pass. The public
//! [`lex_into_arena`] signature stays stable across that change.

use aozora_spec::Diagnostic;
use aozora_syntax::borrowed::{self, Arena, Registry};
use aozora_syntax::{convert, ContainerKind};
use aozora_veb::EytzingerMap;

/// Borrowed-AST analogue of [`crate::LexOutput`].
///
/// The normalized text and registry payloads borrow from `arena`;
/// diagnostics stay on the heap (they own non-`Copy`
/// `miette::SourceSpan` payloads, which bumpalo cannot drop
/// correctly).
#[derive(Debug)]
#[non_exhaustive]
pub struct BorrowedLexOutput<'a> {
    /// Normalized text with PUA sentinels. Allocated in `arena`.
    pub normalized: &'a str,
    /// Cache-friendly sentinel-position → node lookup tables.
    pub registry: Registry<'a>,
    /// Non-fatal observations from every phase. Owned `Vec` because
    /// `Diagnostic` carries non-`Copy` `miette::SourceSpan` data and
    /// the bump arena cannot drop those correctly. Diagnostics are
    /// rare (typically 0–3 per document) so the small heap allocation
    /// is negligible.
    pub diagnostics: Vec<Diagnostic>,
    /// Byte length of the Phase 0 sanitized buffer. Same semantics as
    /// the owned [`crate::LexOutput::sanitized_len`].
    pub sanitized_len: u32,
}

/// Run the lex pipeline and collect the result into `arena`.
///
/// The returned [`BorrowedLexOutput`] has lifetime `'a` tied to
/// `arena`; consumers can hold the output for as long as the arena
/// lives, then drop the arena to free the entire allocation in one
/// `Bump::reset`-equivalent step.
///
/// Equivalent to:
///
/// 1. `let owned = crate::lex(source);`
/// 2. Copy `owned.normalized` into `arena`.
/// 3. Convert each registry entry through
///    [`aozora_syntax::convert::to_borrowed`].
/// 4. Wrap each per-kind position list in an
///    [`aozora_veb::EytzingerMap`].
///
/// Once the new lex pipeline learns native arena emission, steps 1
/// and 3 collapse and step 2 disappears (the normalizer writes
/// directly into the arena). The public signature does not change.
#[must_use]
pub fn lex_into_arena<'a>(source: &str, arena: &'a Arena) -> BorrowedLexOutput<'a> {
    let owned = crate::lex(source);
    // Step 2: copy normalized text into the arena.
    let normalized: &'a str = arena.alloc_str(&owned.normalized);

    // Step 3: convert each registry entry. The legacy registry stores
    // entries in monotonically increasing position order; we preserve
    // that order so the EytzingerMap construction can skip its sort
    // step (debug_assert verifies the invariant).
    let inline = convert_node_table(&owned.registry.inline, arena);
    let block_leaf = convert_node_table(&owned.registry.block_leaf, arena);

    // Step 4: build the four EytzingerMaps. The block_open / block_close
    // tables already use `ContainerKind` (a `Copy` enum) so they pass
    // straight through with no per-entry conversion.
    let block_open = EytzingerMap::from_sorted_slice(&owned.registry.block_open);
    let block_close = EytzingerMap::from_sorted_slice(&owned.registry.block_close);

    BorrowedLexOutput {
        normalized,
        registry: Registry {
            inline,
            block_leaf,
            block_open,
            block_close,
        },
        diagnostics: owned.diagnostics,
        sanitized_len: owned.sanitized_len,
    }
}

/// Helper: convert a `Vec<(u32, owned::AozoraNode)>` registry table
/// into an arena-backed `EytzingerMap<u32, borrowed::AozoraNode<'a>>`.
fn convert_node_table<'a>(
    owned_entries: &[(u32, aozora_syntax::AozoraNode)],
    arena: &'a Arena,
) -> EytzingerMap<u32, borrowed::AozoraNode<'a>> {
    let pairs: Vec<(u32, borrowed::AozoraNode<'a>)> = owned_entries
        .iter()
        .map(|(pos, node)| (*pos, convert::to_borrowed(node, arena)))
        .collect();
    EytzingerMap::from_sorted_slice(&pairs)
}

// Container registries: pure copy of (u32, ContainerKind) — both are
// already `Copy`. We don't strictly need a helper, but a static
// assertion against the type pins the no-conversion expectation so a
// future ContainerKind change that makes it non-Copy would surface here.
const _: fn() = || {
    fn assert_copy<T: Copy>() {}
    assert_copy::<(u32, ContainerKind)>();
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_source_round_trips() {
        let arena = Arena::new();
        let out = lex_into_arena("", &arena);
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.sanitized_len, 0);
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let arena = Arena::new();
        let out = lex_into_arena("hello, world", &arena);
        assert_eq!(out.normalized, "hello, world");
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn explicit_ruby_lands_in_inline_registry() {
        let arena = Arena::new();
        let out = lex_into_arena("｜青梅《おうめ》", &arena);
        // Exactly one inline sentinel emitted by the normalizer.
        assert_eq!(out.registry.inline.len(), 1);
        // The borrowed AozoraNode behind it must be a Ruby.
        let (pos, node) = out
            .registry
            .inline
            .iter_sorted()
            .next()
            .expect("one entry");
        assert!(out.normalized.as_bytes()[*pos as usize..].starts_with(&[0xEE, 0x80, 0x81]));
        assert!(matches!(node, borrowed::AozoraNode::Ruby(_)));
    }

    #[test]
    fn page_break_lands_in_block_leaf_registry() {
        let arena = Arena::new();
        let out = lex_into_arena("text［＃改ページ］more", &arena);
        // Page break is a standalone block, lands in block_leaf.
        assert_eq!(out.registry.block_leaf.len(), 1);
        let (_pos, node) = out
            .registry
            .block_leaf
            .iter_sorted()
            .next()
            .expect("one entry");
        assert!(matches!(node, borrowed::AozoraNode::PageBreak));
    }

    #[test]
    fn paired_container_lands_in_open_close_registries() {
        let arena = Arena::new();
        let out = lex_into_arena(
            "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］",
            &arena,
        );
        assert_eq!(out.registry.block_open.len(), 1);
        assert_eq!(out.registry.block_close.len(), 1);
        let (_, kind) = out.registry.block_open.iter_sorted().next().unwrap();
        assert!(matches!(kind, ContainerKind::Indent { amount: 2 }));
    }

    #[test]
    fn diagnostics_carry_through_to_borrowed_output() {
        let arena = Arena::new();
        let out = lex_into_arena("source has \u{E001} reserved sentinel", &arena);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn sanitized_len_matches_owned_pipeline() {
        let arena = Arena::new();
        let owned = crate::lex("plain text\nwith newline");
        let borrowed = lex_into_arena("plain text\nwith newline", &arena);
        assert_eq!(borrowed.sanitized_len, owned.sanitized_len);
    }

    #[test]
    fn normalized_text_byte_identical_to_owned_pipeline() {
        let arena = Arena::new();
        let inputs = [
            "",
            "hello, world",
            "明治の頃｜青梅《おうめ》街道沿いに、",
            "［＃改ページ］",
            "［＃ここから字下げ］\nA\n［＃ここで字下げ終わり］",
            "※［＃「木＋吶のつくり」、第3水準1-85-54］",
            "［＃「青空」に傍点］",
            "line1\r\nline2\r\nline3",
        ];
        for src in inputs {
            let owned = crate::lex(src);
            let borrowed = lex_into_arena(src, &arena);
            assert_eq!(
                borrowed.normalized, owned.normalized,
                "normalized text diverged for input {src:?}"
            );
            assert_eq!(borrowed.sanitized_len, owned.sanitized_len);
            assert_eq!(borrowed.registry.inline.len(), owned.registry.inline.len());
            assert_eq!(
                borrowed.registry.block_leaf.len(),
                owned.registry.block_leaf.len()
            );
            assert_eq!(
                borrowed.registry.block_open.len(),
                owned.registry.block_open.len()
            );
            assert_eq!(
                borrowed.registry.block_close.len(),
                owned.registry.block_close.len()
            );
        }
    }

    #[test]
    fn arena_owns_normalized_after_owned_pipeline_drops() {
        // Pin lifetime invariant: the borrowed output continues to be
        // valid after the owned source-side strings have been dropped,
        // because everything was copied into the arena. We can't test
        // dropping the source `&str` (it's a stack slice) but we can
        // verify the conversion path made a copy.
        let arena = Arena::new();
        let out = {
            let owned_string = String::from("a｜青梅《おうめ》b");
            lex_into_arena(&owned_string, &arena)
            // owned_string drops here at end of inner scope
        };
        // out is still usable
        assert!(out.normalized.contains('\u{E001}'));
        assert_eq!(out.registry.inline.len(), 1);
        let (_, node) = out.registry.inline.iter_sorted().next().unwrap();
        assert!(matches!(node, borrowed::AozoraNode::Ruby(r) if r.reading.as_plain() == Some("おうめ")));
    }

    #[test]
    fn many_inline_entries_preserve_order() {
        let arena = Arena::new();
        // Five distinct ruby spans → five inline registry entries in
        // monotonic source order.
        let src = "a｜A《a》b｜B《b》c｜C《c》d｜D《d》e｜E《e》";
        let owned = crate::lex(src);
        let borrowed = lex_into_arena(src, &arena);
        assert_eq!(borrowed.registry.inline.len(), 5);
        assert_eq!(borrowed.registry.inline.len(), owned.registry.inline.len());
        let positions: Vec<u32> = borrowed
            .registry
            .inline
            .iter_sorted()
            .map(|(pos, _)| *pos)
            .collect();
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "registry must remain in sorted order");
    }

    #[test]
    fn container_kind_indent_amount_preserved() {
        let arena = Arena::new();
        let out = lex_into_arena(
            "［＃ここから3字下げ］\ntext\n［＃ここで字下げ終わり］",
            &arena,
        );
        // Pin Indent amount survives the arena round-trip.
        let (_, kind) = out.registry.block_open.iter_sorted().next().unwrap();
        match kind {
            ContainerKind::Indent { amount } => assert_eq!(*amount, 3),
            other => panic!("expected Indent {{ amount: 3 }}, got {other:?}"),
        }
    }

    /// Exercise an instance of every owned variant kind that appears
    /// in the lex output, to make sure the converter walks every arm.
    /// Synthesises a corpus-shaped paragraph with as many constructs
    /// as we can pack densely; not every variant lands in the registry
    /// from a single shape (`DoubleRuby` etc. need the right context),
    /// so this is a soft cover, not a strict per-variant matrix.
    #[test]
    fn dense_corpus_paragraph_all_pieces_land_borrowed() {
        let arena = Arena::new();
        let src = "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
                   なる珍しき木が立つ。［＃ここから2字下げ］\n\
                   その下で人々は語らひ、［＃「青空」に傍点］\n\
                   ［＃ここで字下げ終わり］";
        let owned = crate::lex(src);
        let borrowed = lex_into_arena(src, &arena);
        assert_eq!(borrowed.normalized, owned.normalized);
        // Each table preserved its size.
        assert_eq!(borrowed.registry.inline.len(), owned.registry.inline.len());
        assert_eq!(
            borrowed.registry.block_leaf.len(),
            owned.registry.block_leaf.len()
        );
        assert_eq!(
            borrowed.registry.block_open.len(),
            owned.registry.block_open.len()
        );
        assert_eq!(
            borrowed.registry.block_close.len(),
            owned.registry.block_close.len()
        );
        // Every registered position must round-trip via lookup.
        for (pos, _) in borrowed.registry.inline.iter_sorted() {
            assert!(borrowed.registry.inline.contains_key(pos));
        }
    }
}
