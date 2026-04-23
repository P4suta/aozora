//! Phase 3 — classify the Phase 2 event stream into [`AozoraNode`] spans.
//!
//! Walks the cross-linked [`PairEvent`] stream produced by Phase 2 and
//! produces a contiguous vector of [`ClassifiedSpan`] whose
//! `source_span` values tile every byte of the sanitized source
//! end-to-end, in byte-offset order.
//!
//! The span kinds are:
//!
//! * [`SpanKind::Plain`] — a run of text that carries no Aozora
//!   construct. Adjacent un-classified events (text, stray triggers,
//!   unclosed opens, unmatched closes) are merged into one span so
//!   Phase 4 can emit them verbatim in a single write.
//! * [`SpanKind::Aozora`] — a classified Aozora construct, carrying the
//!   concrete [`AozoraNode`] that Phase 4 will replace with a PUA
//!   placeholder sentinel (see [`crate::INLINE_SENTINEL`] and friends).
//! * [`SpanKind::Newline`] — a `\n` in the sanitized text, kept as its
//!   own span kind because block-level annotations (Phase 4 block
//!   sentinel substitution) care about line boundaries.
//!
//! ## Span-coverage invariant
//!
//! When `source.len() > 0`:
//!
//! 1. `spans[0].source_span.start == 0`
//! 2. `spans[i].source_span.end == spans[i + 1].source_span.start`
//! 3. `spans[last].source_span.end == source.len()`
//!
//! When `source.is_empty()`, `spans` is empty.
//!
//! Phase 4 relies on this invariant to emit `normalized` text without
//! ever re-scanning `source`.
//!
//! ## Staged build-out
//!
//! C4a (this commit) ships only the scaffolding: every un-classified
//! run is emitted as [`SpanKind::Plain`] and every `\n` as
//! [`SpanKind::Newline`]. Subsequent commits bolt in concrete
//! recognizers on top of the same driver:
//!
//! * C4b — ruby (explicit `｜base《reading》` and implicit-kanji).
//! * C4c — bracket-annotation keyword dispatch (leaf blocks, bouten
//!   keyword table).
//! * C4d — inline: forward-ref bouten, tcy, gaiji, kaeriten.
//! * C4e — paired containers (字下げ / 地付き / 罫囲み / 割り注 /
//!   小書き / 大中小見出し).
//!
//! Each recognizer is a narrow function that inspects a
//! `&[PairEvent]` slice (often one pair's `body_events`) plus the
//! sanitized source. The driver loop stays the same — only the
//! `recognize` step grows.

use afm_syntax::{AozoraNode, Span};

use crate::diagnostic::Diagnostic;
#[cfg(test)]
use crate::phase2_pair::PairKind;
use crate::phase2_pair::{PairEvent, PairOutput};

/// Output of Phase 3. `spans` tiles the sanitized source contiguously
/// (see the span-coverage invariant in the module docs).
#[derive(Debug, Clone)]
pub struct ClassifyOutput {
    pub spans: Vec<ClassifiedSpan>,
    pub diagnostics: Vec<Diagnostic>,
}

/// One classified slice of the sanitized source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedSpan {
    pub kind: SpanKind,
    pub source_span: Span,
}

/// Classification of a [`ClassifiedSpan`].
///
/// The enum is intentionally small: Phase 4 only needs to distinguish
/// "emit verbatim", "replace with sentinel and record in registry",
/// and "keep as line boundary".
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanKind {
    /// Source bytes that carry no Aozora construct. Emitted verbatim
    /// by Phase 4.
    Plain,
    /// Classified Aozora construct. Phase 4 replaces the source span
    /// with a PUA sentinel and records the node in the placeholder
    /// registry keyed at the sentinel's normalized position.
    Aozora(AozoraNode),
    /// A `\n` in the sanitized text. Retained as its own span kind
    /// because block-level recognizers need line boundaries.
    Newline,
}

/// Classify a Phase 2 event stream against the sanitized source.
///
/// Pure function; no I/O. The output is a byte-contiguous cover of
/// `source` — see the module-level span-coverage invariant.
#[must_use]
pub fn classify(pair_output: &PairOutput, source: &str) -> ClassifyOutput {
    let mut driver = Driver::new(source);
    for ev in &pair_output.events {
        driver.accept(ev);
    }
    driver.finish(pair_output.diagnostics.clone())
}

/// Mutable state for the event-walk.
///
/// `pending_plain_start` is `Some(start_byte)` when the driver is in
/// the middle of accumulating a Plain span; `None` when the last span
/// emitted was a Newline (or nothing yet). Flushing the pending plain
/// span is the only place Plain spans are produced.
struct Driver<'s> {
    source_len: u32,
    spans: Vec<ClassifiedSpan>,
    pending_plain_start: Option<u32>,
    /// Held as a safety net for future recognizer additions — some
    /// classifiers will want the sanitized source to slice body text
    /// out. Plain-only C4a does not read it yet but the scaffold puts
    /// the plumbing in place.
    _source: &'s str,
}

impl<'s> Driver<'s> {
    fn new(source: &'s str) -> Self {
        Self {
            source_len: u32::try_from(source.len()).expect("sanitize asserts fit in u32"),
            spans: Vec::new(),
            pending_plain_start: None,
            _source: source,
        }
    }

    fn accept(&mut self, event: &PairEvent) {
        if let PairEvent::Newline { pos } = *event {
            // Close any in-progress plain run up to `pos`, then emit
            // the newline as its own span. LF is always a single byte
            // in UTF-8, so `pos + 1` is safe.
            self.flush_plain_up_to(pos);
            self.spans.push(ClassifiedSpan {
                kind: SpanKind::Newline,
                source_span: Span::new(pos, pos + 1),
            });
            return;
        }

        // Un-classified in C4a: merge into the pending plain run.
        // Every non-Newline PairEvent carries a span. The end is
        // implicitly tracked by the *next* event's start or the
        // end-of-stream finish pass — the 1:1 token↔event invariant
        // from Phase 2, combined with Phase 1's contiguous byte
        // coverage, means sequential event spans meet end-to-start
        // with no gaps.
        let span = event.span().expect("non-Newline event has a span");
        if self.pending_plain_start.is_none() {
            self.pending_plain_start = Some(span.start);
        }
    }

    /// Emit any pending plain span whose end is `end`, if one is open.
    ///
    /// When `end == start` the pending span covers zero bytes — this
    /// happens only if a Newline follows the previous flush
    /// immediately — and the empty span is dropped rather than emitted.
    fn flush_plain_up_to(&mut self, end: u32) {
        if let Some(start) = self.pending_plain_start.take()
            && end > start
        {
            self.spans.push(ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(start, end),
            });
        }
    }

    fn finish(mut self, diagnostics: Vec<Diagnostic>) -> ClassifyOutput {
        self.flush_plain_up_to(self.source_len);
        ClassifyOutput {
            spans: self.spans,
            diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::phase1_events::tokenize;
    use crate::phase2_pair::pair;

    fn run(src: &str) -> ClassifyOutput {
        let tokens = tokenize(src);
        let pair_out = pair(&tokens);
        classify(&pair_out, src)
    }

    #[test]
    fn empty_input_produces_empty_span_vector() {
        let out = run("");
        assert!(out.spans.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn plain_ascii_becomes_single_plain_span() {
        let out = run("hello");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[0].source_span, Span::new(0, 5));
    }

    #[test]
    fn plain_multibyte_becomes_single_plain_span() {
        let src = "こんにちは";
        let out = run(src);
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(
            out.spans[0].source_span,
            Span::new(0, u32::try_from(src.len()).expect("fits"))
        );
    }

    #[test]
    fn newline_in_middle_splits_into_three_spans() {
        let out = run("line1\nline2");
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[0].source_span, Span::new(0, 5));
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].source_span, Span::new(5, 6));
        assert_eq!(out.spans[2].kind, SpanKind::Plain);
        assert_eq!(out.spans[2].source_span, Span::new(6, 11));
    }

    #[test]
    fn leading_and_trailing_newlines_do_not_emit_empty_plain_spans() {
        let out = run("\nbody\n");
        // Expected: Newline, Plain("body"), Newline. No empty Plain at the edges.
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].kind, SpanKind::Plain);
        assert_eq!(out.spans[2].kind, SpanKind::Newline);
    }

    #[test]
    fn triggers_are_folded_into_plain_for_c4a_scaffold() {
        // With no recognizers yet, an Aozora-like snippet is still
        // classified as Plain. The invariants (contiguous cover) must
        // still hold.
        let out = run("｜漢字《かんじ》");
        // Classification: one contiguous Plain span (no newlines).
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(
            out.spans[0].source_span.end as usize,
            "｜漢字《かんじ》".len()
        );
    }

    #[test]
    fn only_newline_source_emits_only_newline_span() {
        let out = run("\n");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Newline);
        assert_eq!(out.spans[0].source_span, Span::new(0, 1));
    }

    #[test]
    fn diagnostics_from_phase2_are_forwarded() {
        let out = run("stray］");
        // Phase 2 emits an UnmatchedClose diagnostic for `］`. The
        // classifier must propagate it (and not swallow it silently).
        assert!(
            out.diagnostics.iter().any(|d| matches!(
                d,
                Diagnostic::UnmatchedClose {
                    kind: PairKind::Bracket,
                    ..
                }
            )),
            "expected UnmatchedClose to be forwarded, got {:?}",
            out.diagnostics
        );
    }

    proptest! {
        /// Spans must tile the source contiguously, starting at 0 and
        /// ending at `source.len()` with no gaps or overlaps.
        #[test]
        fn proptest_spans_tile_source_contiguously(src in source_strategy()) {
            let out = run(&src);
            if src.is_empty() {
                prop_assert!(out.spans.is_empty());
                return Ok(());
            }
            prop_assert!(!out.spans.is_empty());
            prop_assert_eq!(out.spans[0].source_span.start, 0);
            for window in out.spans.windows(2) {
                prop_assert_eq!(
                    window[0].source_span.end,
                    window[1].source_span.start
                );
            }
            prop_assert_eq!(
                out.spans.last().unwrap().source_span.end as usize,
                src.len()
            );
        }

        /// No empty-range spans leak into the output. An empty span
        /// would usually indicate a double-flush bug and breaks the
        /// "each span represents at least one source byte" expectation
        /// Phase 4 holds.
        #[test]
        fn proptest_no_empty_spans(src in source_strategy()) {
            let out = run(&src);
            for span in &out.spans {
                prop_assert!(span.source_span.end > span.source_span.start);
            }
        }

        /// Every Newline span covers exactly one byte at a `\n`
        /// position.
        #[test]
        fn proptest_newline_spans_are_single_byte(src in source_strategy()) {
            let out = run(&src);
            for span in &out.spans {
                if span.kind == SpanKind::Newline {
                    prop_assert_eq!(span.source_span.len(), 1);
                    prop_assert_eq!(
                        &src[span.source_span.start as usize..span.source_span.end as usize],
                        "\n"
                    );
                }
            }
        }

        /// Classification is a pure function of the input.
        #[test]
        fn proptest_classify_is_deterministic(src in source_strategy()) {
            let a = run(&src);
            let b = run(&src);
            prop_assert_eq!(a.spans, b.spans);
        }
    }

    fn source_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('a'),
                Just('あ'),
                Just('漢'),
                Just('｜'),
                Just('《'),
                Just('》'),
                Just('［'),
                Just('］'),
                Just('＃'),
                Just('※'),
                Just('〔'),
                Just('〕'),
                Just('「'),
                Just('」'),
                Just('\n'),
            ],
            0..40,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }
}
