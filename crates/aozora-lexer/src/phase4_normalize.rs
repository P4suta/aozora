//! Phase 4 — substitute Aozora spans with PUA sentinels, produce the
//! normalized text that feeds comrak.
//!
//! The [`Phase 3`](crate::phase3_classify) output is a contiguous span
//! cover of the sanitized source. Phase 4 walks it and produces:
//!
//! * [`NormalizeOutput::normalized`] — a `String` where every Aozora
//!   span has been replaced by a single-character PUA sentinel.
//!   Plain spans are copied verbatim; newlines are preserved.
//! * [`NormalizeOutput::registry`] — a lookup from each sentinel's
//!   normalized byte offset back to the [`AozoraNode`] or
//!   [`ContainerKind`] that Phase 3 classified. `post_process` uses the
//!   registry to splice the original Aozora construct back into the
//!   comrak AST.
//!
//! ## Sentinel scheme
//!
//! | kind                | sentinel char               | wrap policy            |
//! |---------------------|-----------------------------|------------------------|
//! | inline Aozora       | [`INLINE_SENTINEL`] U+E001  | bare — no wrapping     |
//! | block-leaf Aozora   | [`BLOCK_LEAF_SENTINEL`] U+E002 | `\n\n<sentinel>\n\n` |
//! | paired-container open  | [`BLOCK_OPEN_SENTINEL`] U+E003 | `\n\n<sentinel>\n\n` |
//! | paired-container close | [`BLOCK_CLOSE_SENTINEL`] U+E004 | `\n\n<sentinel>\n\n` |
//!
//! Block sentinels are padded with **blank lines** (`\n\n`) rather
//! than single newlines so comrak treats each sentinel as a standalone
//! paragraph. A single `\n` only folds into a soft break within the
//! surrounding paragraph — `post_process::splice_block_leaf` would
//! then fail to detect the single-sentinel-paragraph pattern.
//! Multiple consecutive blank lines collapse harmlessly per the
//! CommonMark spec.
//!
//! The output registry keeps entries in strictly increasing byte
//! order because the driver only appends at the end of `normalized`.
//! `phase5_registry` wraps the `Vec`s with a `binary_search`-backed
//! public API; the driver skips that sort cost because sorted-by-
//! construction invariant holds.
//!
//! Accent decomposition happens inside the earlier
//! [`crate::phase0_sanitize`] pass; gaiji UCS resolution folds into
//! [`crate::phase3_classify`] via `afm_encoding::gaiji::lookup`.
//! This pass only performs the sentinel substitution.

use afm_syntax::{AozoraNode, ContainerKind};

use crate::diagnostic::Diagnostic;
use crate::phase3_classify::{ClassifiedSpan, ClassifyOutput, SpanKind};
use crate::{BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL};

/// Output of Phase 4.
#[derive(Debug, Clone)]
pub struct NormalizeOutput {
    /// Text with every Aozora span replaced by a PUA sentinel.
    pub normalized: String,
    /// Maps sentinel positions (in `normalized`) back to their
    /// originating Aozora classification.
    pub registry: PlaceholderRegistry,
    /// Accumulated non-fatal observations from Phase 2 and Phase 3.
    pub diagnostics: Vec<Diagnostic>,
}

/// Sentinel-position → original classification.
///
/// Entries land in strictly increasing order of normalized byte
/// offset — the driver only ever appends at the end of `normalized`,
/// so sorts at build-time would be redundant. `phase5_registry`
/// wraps this with a `binary_search`-backed public API.
#[derive(Debug, Clone, Default)]
pub struct PlaceholderRegistry {
    /// `(normalized_byte_pos, node)` for inline Aozora spans.
    pub inline: Vec<(u32, AozoraNode)>,
    /// `(normalized_byte_pos, node)` for block-leaf Aozora spans.
    pub block_leaf: Vec<(u32, AozoraNode)>,
    /// `(normalized_byte_pos, container)` for paired-container openers.
    pub block_open: Vec<(u32, ContainerKind)>,
    /// `(normalized_byte_pos, container)` for paired-container closers.
    pub block_close: Vec<(u32, ContainerKind)>,
}

/// Run Phase 4 over a Phase 3 classification output.
///
/// Pure function; no I/O. The `source` argument must be the same
/// sanitized text that Phase 3 classified — otherwise span offsets
/// will not index correctly.
#[must_use]
pub fn normalize(classify_output: &ClassifyOutput, source: &str) -> NormalizeOutput {
    let mut n = Normalizer::new(source);
    for span in &classify_output.spans {
        n.emit(span);
    }
    NormalizeOutput {
        normalized: n.out,
        registry: n.registry,
        diagnostics: classify_output.diagnostics.clone(),
    }
}

struct Normalizer<'s> {
    out: String,
    source: &'s str,
    registry: PlaceholderRegistry,
}

impl<'s> Normalizer<'s> {
    fn new(source: &'s str) -> Self {
        // Normalized text is always shorter than source (Aozora spans
        // collapse from multi-byte constructs to a single PUA char).
        // Pre-allocating `source.len()` avoids every reasonable
        // reallocation.
        Self {
            out: String::with_capacity(source.len()),
            source,
            registry: PlaceholderRegistry::default(),
        }
    }

    fn emit(&mut self, span: &ClassifiedSpan) {
        match span.kind.clone() {
            SpanKind::Plain => {
                // Sanitize-phase guarantees valid UTF-8 boundaries in
                // the span, so `slice` never panics.
                self.out.push_str(span.source_span.slice(self.source));
            }
            SpanKind::Newline => {
                self.out.push('\n');
            }
            SpanKind::Aozora(node) => {
                // `is_block()` on the AozoraNode schema answers a
                // *semantic* question — whether the node occupies a
                // paragraph position in the tree — but that is too
                // broad for the rendering decision. `Indent`,
                // `AlignEnd`, and `Warichu` are leaf *markers* that
                // live inside the surrounding paragraph; only true
                // document-level separators (page break / section
                // break / heading / illustration) should be promoted
                // to a block-leaf sentinel that splits paragraphs.
                // Pre-existing adapter fixtures — and real Aozora
                // reading convention — both assume these leaf markers
                // sit inline with the text they modify.
                if is_standalone_block_for_render(&node) {
                    self.emit_block_leaf(node);
                } else {
                    self.emit_inline(node);
                }
            }
            SpanKind::BlockOpen(container) => {
                self.emit_block_open(container);
            }
            SpanKind::BlockClose(container) => {
                self.emit_block_close(container);
            }
        }
    }

    fn emit_inline(&mut self, node: AozoraNode) {
        let pos = self.current_pos();
        self.out.push(INLINE_SENTINEL);
        self.registry.inline.push((pos, node));
    }

    fn emit_block_leaf(&mut self, node: AozoraNode) {
        // Pad with **blank** lines (`\n\n`) so comrak treats the
        // sentinel character as a standalone paragraph rather than
        // soft-joining it with adjacent inline text. A single `\n`
        // lets comrak fold `前\nS\n後` into one paragraph with soft
        // breaks, which post_process then cannot promote because the
        // paragraph contains more than just the sentinel.
        self.out.push_str("\n\n");
        let pos = self.current_pos();
        self.out.push(BLOCK_LEAF_SENTINEL);
        self.out.push_str("\n\n");
        self.registry.block_leaf.push((pos, node));
    }

    fn emit_block_open(&mut self, container: ContainerKind) {
        self.out.push_str("\n\n");
        let pos = self.current_pos();
        self.out.push(BLOCK_OPEN_SENTINEL);
        self.out.push_str("\n\n");
        self.registry.block_open.push((pos, container));
    }

    fn emit_block_close(&mut self, container: ContainerKind) {
        self.out.push_str("\n\n");
        let pos = self.current_pos();
        self.out.push(BLOCK_CLOSE_SENTINEL);
        self.out.push_str("\n\n");
        self.registry.block_close.push((pos, container));
    }

    fn current_pos(&self) -> u32 {
        // The sanitize phase caps `source.len()` at `u32::MAX` and
        // Phase 4 never grows the buffer beyond the source size (each
        // Aozora span collapses in bytes). The cast therefore always
        // fits.
        u32::try_from(self.out.len()).expect("normalized length fits u32")
    }
}

/// Decide whether an `AozoraNode` should be emitted as a block-leaf
/// sentinel — a standalone paragraph in the normalized text — or as
/// an inline sentinel that lives inside the surrounding paragraph.
///
/// Only the true document-structural nodes (page break, section
/// break, heading, illustration) warrant paragraph splits. `Indent`,
/// `AlignEnd`, `Warichu`, `Keigakomi` are marker / container nodes
/// that still live within the inline stream of the paragraph they
/// modify; promoting them to block-leaf separates them from the text
/// they are supposed to apply to. Matches the adapter path's
/// implicit behaviour (adapter's `try_start_block` never fires, so
/// every match is inline).
fn is_standalone_block_for_render(node: &AozoraNode) -> bool {
    matches!(
        node,
        AozoraNode::PageBreak
            | AozoraNode::SectionBreak(_)
            | AozoraNode::AozoraHeading(_)
            | AozoraNode::Sashie(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase1_events::tokenize;
    use crate::phase2_pair::pair;
    use crate::phase3_classify::classify;

    fn run(src: &str) -> NormalizeOutput {
        let tokens = tokenize(src);
        let pair_out = pair(&tokens);
        let classify_out = classify(&pair_out, src);
        normalize(&classify_out, src)
    }

    #[test]
    fn empty_source_produces_empty_normalized() {
        let out = run("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.inline.is_empty());
        assert!(out.registry.block_leaf.is_empty());
    }

    #[test]
    fn plain_text_passes_through_verbatim() {
        let src = "hello こんにちは";
        let out = run(src);
        assert_eq!(out.normalized, src);
        assert!(out.registry.inline.is_empty());
    }

    #[test]
    fn newlines_are_preserved() {
        let out = run("line1\nline2\nline3");
        assert_eq!(out.normalized, "line1\nline2\nline3");
    }

    #[test]
    fn inline_ruby_is_replaced_by_single_pua_char() {
        let src = "｜漢字《かんじ》";
        let out = run(src);
        assert_eq!(out.normalized, "\u{E001}");
        assert_eq!(out.registry.inline.len(), 1);
        let (pos, ref node) = out.registry.inline[0];
        assert_eq!(pos, 0);
        let AozoraNode::Ruby(ref ruby) = *node else {
            panic!("expected Ruby, got {node:?}");
        };
        assert_eq!(ruby.base.as_plain(), Some("漢字"));
        assert_eq!(ruby.reading.as_plain(), Some("かんじ"));
    }

    #[test]
    fn inline_ruby_keeps_surrounding_plain_bytes() {
        let out = run("前｜漢《かん》後");
        assert_eq!(out.normalized, "前\u{E001}後");
        assert_eq!(out.registry.inline.len(), 1);
        // Byte positions: "前" = 3 bytes → sentinel at 3, then "後" at 6.
        assert_eq!(out.registry.inline[0].0, 3);
    }

    #[test]
    fn page_break_becomes_block_leaf_sentinel_on_own_line() {
        let src = "前\n［＃改ページ］\n後";
        let out = run(src);
        // Each block sentinel is padded with blank lines (`\n\n`) so
        // comrak treats it as a standalone paragraph. Adjacent source
        // newlines stack with the padding — harmless because comrak
        // collapses repeated blank lines.
        assert_eq!(out.normalized, "前\n\n\n\u{E002}\n\n\n後");
        assert_eq!(out.registry.block_leaf.len(), 1);
        let (pos, ref node) = out.registry.block_leaf[0];
        assert!(matches!(node, AozoraNode::PageBreak));
        // Pos should match a byte where BLOCK_LEAF_SENTINEL sits.
        assert_eq!(
            &out.normalized[pos as usize..pos as usize + BLOCK_LEAF_SENTINEL.len_utf8()],
            "\u{E002}",
        );
    }

    #[test]
    fn page_break_in_raw_text_without_surrounding_newlines_still_isolates() {
        let out = run("前［＃改ページ］後");
        // Driver injects blank lines (`\n\n`) around the block sentinel
        // so comrak treats it as a standalone paragraph separate from
        // the preceding/trailing inline text.
        assert_eq!(out.normalized, "前\n\n\u{E002}\n\n後");
    }

    #[test]
    fn block_open_and_close_emit_own_sentinel_lines() {
        let out = run("［＃ここから字下げ］本文［＃ここで字下げ終わり］");
        // Expected: blank-line padding around each paired sentinel.
        assert_eq!(out.normalized, "\n\n\u{E003}\n\n本文\n\n\u{E004}\n\n");
        assert_eq!(out.registry.block_open.len(), 1);
        assert_eq!(out.registry.block_close.len(), 1);
        assert!(matches!(
            out.registry.block_open[0].1,
            ContainerKind::Indent { amount: 1 }
        ));
        assert!(matches!(
            out.registry.block_close[0].1,
            ContainerKind::Indent { .. }
        ));
    }

    #[test]
    fn registry_inline_entries_are_sorted_by_position() {
        let out = run("A｜漢《か》B｜字《じ》C");
        let positions: Vec<u32> = out.registry.inline.iter().map(|(p, _)| *p).collect();
        assert_eq!(positions.len(), 2);
        assert!(
            positions[0] < positions[1],
            "inline registry positions must be increasing: {positions:?}"
        );
    }

    #[test]
    fn sentinels_are_not_adjacent_to_aozora_original_bytes() {
        // None of the original Aozora construct chars (｜《》＃［］) should
        // leak into the normalized text.
        let out = run("前｜漢《かん》［＃改ページ］後");
        for ch in ['｜', '《', '》', '［', '］', '＃'] {
            assert!(
                !out.normalized.contains(ch),
                "trigger char {ch:?} must not leak into normalized: {:?}",
                out.normalized
            );
        }
    }

    #[test]
    fn diagnostics_from_earlier_phases_are_forwarded() {
        // A stray ］ triggers Phase 2's UnmatchedClose.
        let out = run("text］more");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnmatchedClose { .. })),
            "expected UnmatchedClose diagnostic, got {:?}",
            out.diagnostics
        );
    }
}
