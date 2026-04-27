//! Arena-emitting lex API (Plan B.2 + I-7 interner + I-2.2 native
//! borrowed Phase 3).
//!
//! Produces a [`BorrowedLexOutput<'a>`] whose normalized text and
//! placeholder registry live entirely inside an external [`Arena`].
//! Drop the arena, and the entire lex output (every node, every
//! borrowed string, every registry table) deallocates in a single
//! step — no per-node `Drop` ever runs, no scattered `Box::drop`
//! malloc traffic on the way out.
//!
//! ## Pipeline (post I-2.2)
//!
//! 1. Phases 0-2 (sanitize / tokenize / pair) run as owned-data
//!    helpers operating on byte spans and event indices — they never
//!    construct AST.
//! 2. Phase 3 [`aozora_lexer::classify_with`] is invoked with a
//!    [`BorrowedAllocator`] backed by `arena`. Borrowed AST nodes
//!    land directly in the arena; strings flow through the I-7
//!    [`aozora_syntax::borrowed::Interner`] owned by the allocator
//!    so byte-equal content (ruby readings, container labels,
//!    kaeriten marks, …) shares a single allocation.
//! 3. A single fused walk emits the PUA-rewritten text into the arena
//!    and builds the four borrowed-registry tables. Replaces the
//!    legacy two-pass pipeline (owned classify → per-span
//!    `convert::to_borrowed_with` deep-clone) entirely.
//! 4. Each per-kind position list is wrapped in an
//!    [`aozora_veb::EytzingerMap`] for cache-friendly lookup.
//!
//! The interner's diagnostic counters (cache hits, table hits, allocs,
//! avg probe length) are exposed via [`BorrowedLexOutput::intern_stats`]
//! so callers and benchmarks can measure dedup effectiveness without
//! re-running the conversion.

use aozora_lexer::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, ClassifiedSpan,
    INLINE_SENTINEL, SpanKind,
};
use aozora_spec::{Diagnostic, PairLink, Span};
use aozora_syntax::ContainerKind;
use aozora_syntax::borrowed::{self, Arena, InternStats, NodeRef, Registry};

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
    /// Resolved (open, close) pair side-table from Phase 2 (in close
    /// order — the order matches close events as the stack drains).
    /// Built by [`aozora_lexer::pair_in`] and forwarded verbatim. Lives
    /// in the same arena as `normalized`. Editor surfaces (LSP
    /// `linkedEditingRange` / `documentHighlight`) consume this
    /// directly; the four registries above only carry sentinel-position
    /// information, which is the wrong coordinate system for a source
    /// editor.
    pub pairs: &'a [PairLink],
    /// Source-keyed node side-table: one entry per emitted Aozora /
    /// container span, in source order. Built during the Phase 3 →
    /// arena-normalize fold so editor surfaces can answer "what node
    /// is at this source byte offset?" without re-walking the
    /// registries.
    ///
    /// Entries are sorted by `source_span.start` (the classifier tiles
    /// spans contiguously, so the sort comes for free). Use
    /// [`Self::node_at_source`] for a binary-searched lookup.
    ///
    /// Coordinates are in **sanitized-source** bytes — for the typical
    /// document (no BOM, only LF, no `〔...〕` accent spans, no long
    /// decorative rule lines) sanitized == source byte-for-byte.
    /// Editor callers that produce raw-source offsets and run against
    /// a sanitization-rewriting input must do their own translation.
    pub source_nodes: &'a [SourceNode<'a>],
    /// Counters from the [`Interner`] used during conversion.
    /// Exposed so benchmarks can measure dedup ratio
    /// (`(cache_hits + table_hits) / calls`) and average probe length
    /// without re-running the lex.
    pub intern_stats: InternStats,
}

/// Source-keyed registry entry — pairs a source-byte span with the
/// classified node landed there. Lives in the bumpalo arena.
#[derive(Debug, Clone, Copy)]
pub struct SourceNode<'a> {
    pub source_span: Span,
    pub node: NodeRef<'a>,
}

impl<'a> BorrowedLexOutput<'a> {
    /// Find the [`SourceNode`] whose `source_span` covers `src_off`,
    /// where `src_off` is a sanitized-source byte offset.
    ///
    /// Spans are half-open: `start <= src_off < end`. Returns `None` if
    /// no entry covers the position (typically because the offset lies
    /// in a Plain run between Aozora constructs).
    ///
    /// Lookup is `O(log n)` via binary search on the source-sorted
    /// `source_nodes` slice.
    #[must_use]
    pub fn node_at_source(&self, src_off: u32) -> Option<&SourceNode<'a>> {
        // Binary search by source_span.start; the run we want either
        // starts at or before src_off. partition_point returns the
        // first index whose start > src_off, so subtracting one gives
        // the candidate.
        let idx = self
            .source_nodes
            .partition_point(|entry| entry.source_span.start <= src_off);
        if idx == 0 {
            return None;
        }
        let candidate = &self.source_nodes[idx - 1];
        (src_off < candidate.source_span.end).then_some(candidate)
    }
}

/// Run the lex pipeline and collect the result into `arena`.
///
/// The returned [`BorrowedLexOutput`] has lifetime `'a` tied to
/// `arena`; consumers can hold the output for as long as the arena
/// lives, then drop the arena to free the entire allocation in one
/// `Bump::reset`-equivalent step.
///
/// Pipeline (post I-2.2):
///
/// 1. Sanitize / tokenize / pair (Phases 0-2) — unchanged owned-data
///    helpers operating on byte spans and event indices.
/// 2. `classify_with::<BorrowedAllocator>` — Phase 3 builds borrowed
///    `AozoraNode<'a>` directly into `arena`, with strings interned
///    through the I-7 `Interner` owned by the allocator. No owned-AST
///    intermediate is constructed; `convert::to_borrowed_with` is
///    not called.
/// 3. Single fused normalize walk: build the four borrowed-registry
///    tables and stream the PUA-rewritten text into `arena` in one
///    pass. Mirrors `aozora_lexer::phase4_normalize::Normalizer`'s
///    sentinel / padding contract byte-for-byte, so the output is
///    proptest-pinned for determinism + sentinel-alignment in
///    `tests/property_borrowed_arena.rs`.
#[must_use]
pub fn lex_into_arena<'a>(source: &str, arena: &'a Arena) -> BorrowedLexOutput<'a> {
    // Thin wrapper around the canonical Pipeline (Innovation I-3,
    // post-deforestation). The Pipeline owns the type-state machine
    // that enforces phase order at compile time; this function exists
    // for API compatibility and is what `Document::parse` calls.
    crate::pipeline::Pipeline::run_to_completion(source, arena)
}

/// Single-pass arena-emitting normalizer (Plan I-2.1 + I-2.2).
///
/// Mirrors `aozora_lexer::phase4_normalize::Normalizer`'s sentinel /
/// padding contract byte-for-byte, but pushes into per-kind
/// `Vec<(u32, borrowed::AozoraNode<'a>)>` tables. The nodes are
/// allocated upstream by [`BorrowedAllocator`] during
/// [`classify_with`] (I-2.2); this walker is now strictly the
/// PUA-rewriter + position-recorder, doing zero AST allocation of
/// its own.
pub(crate) struct ArenaNormalizer<'src, 'a> {
    pub(crate) out: String,
    source: &'src str,
    pub(crate) inline: Vec<(u32, borrowed::AozoraNode<'a>)>,
    pub(crate) block_leaf: Vec<(u32, borrowed::AozoraNode<'a>)>,
    pub(crate) block_open: Vec<(u32, ContainerKind)>,
    pub(crate) block_close: Vec<(u32, ContainerKind)>,
    /// Source-keyed (`source_span`, `NodeRef`) parallel to the four
    /// per-kind tables above. Drained into the arena
    /// `&'a [SourceNode]` at pipeline-build time. Naturally sorted by
    /// `source_span.start` because the classifier emits spans in
    /// source order.
    pub(crate) source_nodes: Vec<SourceNode<'a>>,
}

impl<'src, 'a> ArenaNormalizer<'src, 'a> {
    pub(crate) fn new(source: &'src str, span_capacity_hint: usize) -> Self {
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
            // source_nodes mirrors the union of the four tables above.
            // Sum of the per-kind hints is a tight upper bound.
            source_nodes: Vec::with_capacity(span_capacity_hint),
        }
    }

    fn current_pos(&self) -> u32 {
        u32::try_from(self.out.len()).expect("normalized fits u32 per Phase 0 cap")
    }

    pub(crate) fn emit(&mut self, span: &ClassifiedSpan<'a>) {
        match &span.kind {
            SpanKind::Plain => {
                self.out.push_str(span.source_span.slice(self.source));
            }
            SpanKind::Newline => {
                self.out.push('\n');
            }
            SpanKind::Aozora(node) => {
                // Phase 3 has already allocated the borrowed node into
                // the arena via `BorrowedAllocator`. We only have to
                // emit the appropriate sentinel and remember the
                // position. No conversion, no per-node allocation.
                if is_standalone_block_for_render_borrowed(*node) {
                    // Block-leaf padding: blank-line / sentinel /
                    // blank-line. Mirrors
                    // `aozora_lexer::phase4_normalize::Normalizer::emit_block_leaf`
                    // byte-for-byte so comrak still sees the standalone
                    // paragraph shape.
                    self.out.push_str("\n\n");
                    let pos = self.current_pos();
                    self.out.push(BLOCK_LEAF_SENTINEL);
                    self.out.push_str("\n\n");
                    self.block_leaf.push((pos, *node));
                    self.source_nodes.push(SourceNode {
                        source_span: span.source_span,
                        node: NodeRef::BlockLeaf(*node),
                    });
                } else {
                    let pos = self.current_pos();
                    self.out.push(INLINE_SENTINEL);
                    self.inline.push((pos, *node));
                    self.source_nodes.push(SourceNode {
                        source_span: span.source_span,
                        node: NodeRef::Inline(*node),
                    });
                }
            }
            SpanKind::BlockOpen(container) => {
                self.out.push_str("\n\n");
                let pos = self.current_pos();
                self.out.push(BLOCK_OPEN_SENTINEL);
                self.out.push_str("\n\n");
                self.block_open.push((pos, *container));
                self.source_nodes.push(SourceNode {
                    source_span: span.source_span,
                    node: NodeRef::BlockOpen(*container),
                });
            }
            SpanKind::BlockClose(container) => {
                self.out.push_str("\n\n");
                let pos = self.current_pos();
                self.out.push(BLOCK_CLOSE_SENTINEL);
                self.out.push_str("\n\n");
                self.block_close.push((pos, *container));
                self.source_nodes.push(SourceNode {
                    source_span: span.source_span,
                    node: NodeRef::BlockClose(*container),
                });
            }
            // `SpanKind` is `#[non_exhaustive]`; new variants land
            // here as no-op until the normalizer adds a dedicated arm.
            _ => {}
        }
    }
}

/// Borrowed-AST mirror of
/// [`aozora_lexer::is_standalone_block_for_render`]. Pinned by variant
/// kind, not payload, so it stays in sync trivially with the owned
/// helper (any new standalone-block variant must be added in both
/// places — caught by the byte-identical proptest).
fn is_standalone_block_for_render_borrowed(node: borrowed::AozoraNode<'_>) -> bool {
    matches!(
        node,
        borrowed::AozoraNode::PageBreak
            | borrowed::AozoraNode::SectionBreak(_)
            | borrowed::AozoraNode::AozoraHeading(_)
            | borrowed::AozoraNode::Sashie(_)
    )
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
        let (pos, node) = out.registry.inline.iter_sorted().next().expect("one entry");
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
    fn sanitized_len_equals_input_for_plain_text() {
        // Sanitize is identity on plain UTF-8 text, so sanitized_len
        // matches the input length.
        let arena = Arena::new();
        let input = "plain text\nwith newline";
        let out = lex_into_arena(input, &arena);
        assert_eq!(usize::try_from(out.sanitized_len), Ok(input.len()));
    }

    #[test]
    fn arena_owns_normalized_after_source_drops() {
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
        assert!(
            matches!(node, borrowed::AozoraNode::Ruby(r) if r.reading.as_plain() == Some("おうめ"))
        );
    }

    #[test]
    fn many_inline_entries_preserve_sort_order() {
        let arena = Arena::new();
        // Five distinct ruby spans → five inline registry entries in
        // monotonic source order.
        let src = "a｜A《a》b｜B《b》c｜C《c》d｜D《d》e｜E《e》";
        let out = lex_into_arena(src, &arena);
        assert_eq!(out.registry.inline.len(), 5);
        let positions: Vec<u32> = out
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

    /// Exercise multiple variant kinds in a single dense paragraph so a
    /// regression in any one classifier shows up in the registry sizes.
    /// Numbers are pinned at the values produced by the canonical
    /// pipeline at the time of writing — refresh if a future
    /// classifier upgrade legitimately changes the count.
    #[test]
    fn dense_corpus_paragraph_lands_expected_pieces() {
        let arena = Arena::new();
        let src = "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
                   なる珍しき木が立つ。［＃ここから2字下げ］\n\
                   その下で人々は語らひ、［＃「青空」に傍点］\n\
                   ［＃ここで字下げ終わり］";
        let out = lex_into_arena(src, &arena);
        // Inline: ruby + gaiji + bouten ⇒ 3 entries. Block container
        // open/close ⇒ 1 each. No leaves.
        assert_eq!(out.registry.inline.len(), 3);
        assert_eq!(out.registry.block_leaf.len(), 0);
        assert_eq!(out.registry.block_open.len(), 1);
        assert_eq!(out.registry.block_close.len(), 1);
        // Every registered position must round-trip via lookup.
        for (pos, _) in out.registry.inline.iter_sorted() {
            assert!(out.registry.inline.contains_key(pos));
        }
    }

    /// Pin the contract that the explicit Pipeline chain and the
    /// `lex_into_arena` one-shot agree byte-for-byte on the dense
    /// corpus shape. (`pipeline.rs` already pins this for the simpler
    /// ruby input; this case adds container open/close + gaiji +
    /// bouten coverage.)
    #[test]
    fn pipeline_chain_matches_lex_into_arena_byte_for_byte() {
        let src = "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
                   なる珍しき木が立つ。［＃ここから2字下げ］\n\
                   その下で人々は語らひ、［＃「青空」に傍点］\n\
                   ［＃ここで字下げ終わり］";
        let arena_chain = Arena::new();
        let arena_one = Arena::new();
        let chain = crate::pipeline::Pipeline::new(src, &arena_chain)
            .sanitize()
            .tokenize()
            .pair()
            .build();
        let oneshot = lex_into_arena(src, &arena_one);

        assert_eq!(chain.normalized, oneshot.normalized);
        assert_eq!(chain.sanitized_len, oneshot.sanitized_len);
        assert_eq!(chain.registry.inline.len(), oneshot.registry.inline.len());
        assert_eq!(
            chain.registry.block_leaf.len(),
            oneshot.registry.block_leaf.len()
        );
        assert_eq!(
            chain.registry.block_open.len(),
            oneshot.registry.block_open.len()
        );
        assert_eq!(
            chain.registry.block_close.len(),
            oneshot.registry.block_close.len()
        );
        assert_eq!(chain.diagnostics.len(), oneshot.diagnostics.len());
    }

    /// Phase 0 (sanitize) rewrites CR/LF to LF. Inspect the
    /// intermediate `Sanitized` state and confirm `sanitized_text()`
    /// reflects the rewrite — the Pipeline accessor is the supported
    /// way to peek between phases.
    #[test]
    fn pipeline_intermediate_inspection_after_sanitize() {
        let arena = Arena::new();
        let src = "line1\r\nline2\rline3\n";
        let p = crate::pipeline::Pipeline::new(src, &arena).sanitize();
        // After Phase 0, every CR / CRLF is collapsed to a single LF.
        assert_eq!(p.sanitized_text(), "line1\nline2\nline3\n");
        // Drive the rest to make sure the inspection didn't consume
        // anything required downstream.
        let final_out = p.tokenize().pair().build();
        // Plain text → no inline/block entries.
        assert!(final_out.registry.is_empty());
    }

    /// Block open/close sentinels carry blank-line padding on both
    /// sides so comrak treats them as standalone paragraph lines. The
    /// padding is part of the documented sentinel contract — pin the
    /// exact `\n\n<sentinel>\n\n` byte sequence.
    #[test]
    fn arena_normalizer_block_open_close_padding_is_blank_line_sentinel_blank_line() {
        let arena = Arena::new();
        let src = "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］";
        let out = lex_into_arena(src, &arena);

        // Find the open and close sentinel positions from the registry.
        let (&open_pos, _) = out
            .registry
            .block_open
            .iter_sorted()
            .next()
            .expect("one open entry");
        let (&close_pos, _) = out
            .registry
            .block_close
            .iter_sorted()
            .next()
            .expect("one close entry");

        let bytes = out.normalized.as_bytes();
        // BLOCK_OPEN_SENTINEL is U+E003 = 3 bytes UTF-8 (EE 80 83).
        let open_sentinel_bytes = "\u{E003}".as_bytes();
        let close_sentinel_bytes = "\u{E004}".as_bytes();

        // The two bytes before the sentinel position must be `\n\n`.
        assert!(open_pos as usize >= 2);
        assert_eq!(
            &bytes[(open_pos as usize - 2)..open_pos as usize],
            b"\n\n",
            "block_open: expected \\n\\n before sentinel"
        );
        // The bytes AT the sentinel position must be the open sentinel.
        let open_after = open_pos as usize + open_sentinel_bytes.len();
        assert_eq!(&bytes[open_pos as usize..open_after], open_sentinel_bytes);
        // Followed by `\n\n`.
        assert!(open_after + 2 <= bytes.len());
        assert_eq!(&bytes[open_after..open_after + 2], b"\n\n");

        // Same for close.
        assert!(close_pos as usize >= 2);
        assert_eq!(
            &bytes[(close_pos as usize - 2)..close_pos as usize],
            b"\n\n",
            "block_close: expected \\n\\n before sentinel"
        );
        let close_after = close_pos as usize + close_sentinel_bytes.len();
        assert_eq!(
            &bytes[close_pos as usize..close_after],
            close_sentinel_bytes
        );
        assert!(close_after + 2 <= bytes.len());
        assert_eq!(&bytes[close_after..close_after + 2], b"\n\n");
    }
}
