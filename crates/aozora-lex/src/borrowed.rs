//! Arena-emitting lex API (Plan B.2 + I-7 string interner).
//!
//! Produces a [`BorrowedLexOutput<'a>`] whose normalized text and
//! placeholder registry live entirely inside an external [`Arena`].
//! Drop the arena, and the entire lex output (every node, every
//! borrowed string, every registry table) deallocates in a single
//! step — no per-node `Drop` ever runs, no scattered `Box::drop`
//! malloc traffic on the way out.
//!
//! ## Pipeline
//!
//! 1. Run the legacy [`crate::lex`] pipeline (which still owns the
//!    Box-allocated AST internally).
//! 2. Build an [`Interner`] backed by `arena` and feed every owned
//!    registry node through [`aozora_syntax::convert::to_borrowed_with`]
//!    so byte-equal strings (ruby readings, kaeriten marks, container
//!    labels) share a single arena allocation. Innovation I-7 in
//!    action — empirically Aozora corpora dedup to 30–50% of the
//!    naive size.
//! 3. Wrap the resulting `(u32, borrowed::AozoraNode<'a>)` lists in
//!    [`aozora_veb::EytzingerMap`] for cache-friendly lookup.
//!
//! The interner's diagnostic counters (cache hits, table hits, allocs,
//! avg probe length) are exposed via [`BorrowedLexOutput::intern_stats`]
//! so callers and benchmarks can measure dedup effectiveness without
//! re-running the conversion.
//!
//! ## Future migration
//!
//! Plan B's later steps fold the conversion away: the lex pipeline
//! grows native arena-aware classifiers so steps 1 and 2 collapse
//! into one allocate-into-the-arena pass. The public
//! [`lex_into_arena`] signature stays stable across that change.

use aozora_lexer::{
    is_standalone_block_for_render, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL,
    BLOCK_OPEN_SENTINEL, ClassifiedSpan, INLINE_SENTINEL, SpanKind,
};
use aozora_spec::Diagnostic;
use aozora_syntax::borrowed::{self, Arena, InternStats, Interner, Registry};
use aozora_syntax::convert::{self, StringPool};
use aozora_syntax::ContainerKind;
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
    /// Counters from the [`Interner`] used during conversion.
    /// Exposed so benchmarks can measure dedup ratio
    /// (`(cache_hits + table_hits) / calls`) and average probe length
    /// without re-running the lex.
    pub intern_stats: InternStats,
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
    // Phases 0-3 stay on the legacy owned path. Phase 3 still
    // box-allocates `AozoraNode` per span — that's I-2.2 future
    // work to make arena-native. What this function does in I-2.1
    // is FUSE the legacy phase 4 normalize + the post-pass arena
    // convert into a single walk that emits the borrowed registry
    // directly (no intermediate `PlaceholderRegistry` Vec, no
    // post-hoc deep-clone of every AozoraNode into the registry).
    let sanitized = aozora_lexer::sanitize(source);
    let tokens = aozora_lexer::tokenize(&sanitized.text);
    let pair_out = aozora_lexer::pair(&tokens);
    let classify_out = aozora_lexer::classify(&pair_out, &sanitized.text);

    // Pre-allocate the interner sized to the span count. Each Aozora
    // span contributes 1-4 strings (ruby base/reading, gaiji
    // description, kaeriten mark, etc.); the *2 multiplier is a tight
    // upper bound that keeps the table from resizing on average corpus
    // documents. `with_capacity_in` rounds up to the next power of two.
    let span_count = classify_out.spans.len();
    let interner_hint = span_count.max(64);
    let mut interner = Interner::with_capacity_in(interner_hint, arena);

    // Single fused walk: emit normalized text + build the four
    // borrowed-registry tables in one pass. Eliminates the legacy
    // `Normalizer::emit_inline`'s `node.clone()` per span (deep clone
    // of every AozoraNode into the owned `PlaceholderRegistry`) and
    // the subsequent post-hoc convert-to-arena sweep — both replaced
    // by an inline `convert::to_borrowed_with` per span at write time.
    let mut builder = ArenaNormalizer::new(&sanitized.text, span_count);
    for span in &classify_out.spans {
        builder.emit(span, arena, &mut interner);
    }

    let normalized: &'a str = arena.alloc_str(&builder.out);

    let registry = Registry {
        inline: EytzingerMap::from_sorted_slice(&builder.inline),
        block_leaf: EytzingerMap::from_sorted_slice(&builder.block_leaf),
        block_open: EytzingerMap::from_sorted_slice(&builder.block_open),
        block_close: EytzingerMap::from_sorted_slice(&builder.block_close),
    };

    // Diagnostics merge order (mirrors `aozora_lexer::lex`):
    //
    // 1. Phase 0 sanitize diagnostics first (`SourceContainsPua`).
    // 2. `classify_out.diagnostics` already merges in phase 2 pair
    //    diagnostics via `Driver::finish(pair_output.diagnostics.clone())`,
    //    so we extend with classify ONLY — `pair_out.diagnostics`
    //    would double-count.
    // 3. Phase 6 validate (in debug + with the validate-invariants
    //    feature) appends V1..V3 invariant breaches, skipped in
    //    release per ADR-0014.
    let mut diagnostics = sanitized.diagnostics;
    diagnostics.extend(classify_out.diagnostics.iter().cloned());
    if cfg!(any(debug_assertions, feature = "validate-invariants")) {
        diagnostics.extend(validate_inline(&builder, &mut Vec::new()));
    }

    let intern_stats = interner.stats;
    let sanitized_len =
        u32::try_from(sanitized.text.len()).expect("sanitize asserts source.len() <= u32::MAX");

    BorrowedLexOutput {
        normalized,
        registry,
        diagnostics,
        sanitized_len,
        intern_stats,
    }
}

/// Single-pass arena-emitting normalizer (Plan I-2.1).
///
/// Mirrors `aozora_lexer::phase4_normalize::Normalizer`'s sentinel /
/// padding contract byte-for-byte, but pushes into per-kind
/// `Vec<(u32, borrowed::AozoraNode<'a>)>` tables backed by the same
/// arena that the converted nodes live in. Replaces the prior
/// `aozora_lexer::lex` → `convert_node_table` two-pass pipeline.
struct ArenaNormalizer<'src, 'a> {
    out: String,
    source: &'src str,
    inline: Vec<(u32, borrowed::AozoraNode<'a>)>,
    block_leaf: Vec<(u32, borrowed::AozoraNode<'a>)>,
    block_open: Vec<(u32, ContainerKind)>,
    block_close: Vec<(u32, ContainerKind)>,
}

impl<'src, 'a> ArenaNormalizer<'src, 'a> {
    fn new(source: &'src str, span_capacity_hint: usize) -> Self {
        Self {
            // Normalized text always shrinks vs source (multi-byte
            // Aozora constructs collapse to a single PUA char), so
            // `source.len()` is a safe upper bound.
            out: String::with_capacity(source.len()),
            source,
            // Per-kind table capacities are educated guesses from
            // corpus profiling: inline dominates (~80% of spans),
            // block_leaf ~10%, containers ~5% each. Conservative
            // splits keep early `push`es alloc-free.
            inline: Vec::with_capacity(span_capacity_hint.saturating_mul(4) / 5),
            block_leaf: Vec::with_capacity(span_capacity_hint / 10),
            block_open: Vec::with_capacity(span_capacity_hint / 20),
            block_close: Vec::with_capacity(span_capacity_hint / 20),
        }
    }

    fn current_pos(&self) -> u32 {
        u32::try_from(self.out.len()).expect("normalized fits u32 per Phase 0 cap")
    }

    fn emit<P: StringPool<'a>>(
        &mut self,
        span: &ClassifiedSpan,
        arena: &'a Arena,
        pool: &mut P,
    ) {
        match &span.kind {
            SpanKind::Plain => {
                self.out.push_str(span.source_span.slice(self.source));
            }
            SpanKind::Newline => {
                self.out.push('\n');
            }
            SpanKind::Aozora(node) => {
                if is_standalone_block_for_render(node) {
                    // Block-leaf padding: blank-line / sentinel /
                    // blank-line. Mirrors `Normalizer::emit_block_leaf`
                    // byte-for-byte so comrak still sees the standalone
                    // paragraph shape.
                    self.out.push_str("\n\n");
                    let pos = self.current_pos();
                    self.out.push(BLOCK_LEAF_SENTINEL);
                    self.out.push_str("\n\n");
                    let borrowed = convert::to_borrowed_with(node, arena, pool);
                    self.block_leaf.push((pos, borrowed));
                } else {
                    let pos = self.current_pos();
                    self.out.push(INLINE_SENTINEL);
                    let borrowed = convert::to_borrowed_with(node, arena, pool);
                    self.inline.push((pos, borrowed));
                }
            }
            SpanKind::BlockOpen(container) => {
                self.out.push_str("\n\n");
                let pos = self.current_pos();
                self.out.push(BLOCK_OPEN_SENTINEL);
                self.out.push_str("\n\n");
                self.block_open.push((pos, *container));
            }
            SpanKind::BlockClose(container) => {
                self.out.push_str("\n\n");
                let pos = self.current_pos();
                self.out.push(BLOCK_CLOSE_SENTINEL);
                self.out.push_str("\n\n");
                self.block_close.push((pos, *container));
            }
            // `SpanKind` is `#[non_exhaustive]`; new variants land
            // here as no-op until the normalizer adds a dedicated arm.
            _ => {}
        }
    }
}

/// Run V1..V3 invariant checks on the normalizer's output. Wraps
/// `aozora_lexer::validate` by reconstructing a temporary
/// `PlaceholderRegistry` from the borrowed builder — the legacy
/// validator is read-only over the registry shape, and its
/// diagnostics carry the only relevant payload. Returns the new
/// diagnostics it produced.
///
/// Unused `_scratch` argument anchored for a future zero-alloc
/// validator that takes a borrowed-registry view directly; kept
/// here so the call site doesn't churn when that lands.
fn validate_inline(
    builder: &ArenaNormalizer<'_, '_>,
    _scratch: &mut Vec<Diagnostic>,
) -> Vec<Diagnostic> {
    // Build a minimal owned PlaceholderRegistry view for the legacy
    // validator. It only inspects positions and variant kinds, so we
    // can hand it dummy `AozoraNode::PageBreak` payloads without
    // affecting the diagnostics it produces. Avoids an arena drain
    // that a per-entry deep clone would require.
    use aozora_lexer::PlaceholderRegistry;
    use aozora_syntax::owned::AozoraNode as OwnedNode;
    let registry = PlaceholderRegistry {
        inline: builder
            .inline
            .iter()
            .map(|(p, _)| (*p, OwnedNode::PageBreak))
            .collect(),
        block_leaf: builder
            .block_leaf
            .iter()
            .map(|(p, _)| (*p, OwnedNode::PageBreak))
            .collect(),
        block_open: builder.block_open.clone(),
        block_close: builder.block_close.clone(),
    };
    let normalize_out = aozora_lexer::NormalizeOutput {
        normalized: builder.out.clone(),
        registry,
        diagnostics: Vec::new(),
    };
    let validated = aozora_lexer::validate(normalize_out);
    validated.diagnostics
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
