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
//! ## Recogniser layout
//!
//! Every recogniser is a narrow function that inspects a
//! `&[PairEvent]` slice (often one pair's `body_events`) plus the
//! sanitized source. The driver loop's [`Classifier::try_recognize`]
//! dispatches based on the leading event kind:
//!
//! * Ruby (`｜X《Y》` explicit, trailing-kanji implicit)
//! * Bracket annotations, dispatched on the body keyword:
//!   fixed keyword (`改ページ` / `地付き` / ...), kaeriten
//!   (`一`/`二`/... plus okurigana `（X）`), indent / align-end
//!   (`N字下げ` / `地からN字上げ`), sashie (`挿絵`), forward-ref
//!   bouten, forward-ref TCY, paired-container open / close, and
//!   an `Annotation{Unknown}` catch-all.
//! * Gaiji — `※［＃...］` reference-mark + bracket combos.
//! * Double angle-bracket `《《…》》` escape (`DoubleRuby`).
//!
//! The catch-all makes every well-formed `［＃…］` bracket produce
//! *some* `AozoraNode`, so the Tier-A canary (no bare `［＃` in the
//! HTML output outside an `afm-annotation` wrapper) holds regardless
//! of which specialised recogniser claims the bracket.

use core::ops::Range;

use aozora_encoding::gaiji as gaiji_resolve;
use aozora_syntax::{
    AlignEnd, Annotation, AnnotationKind, AozoraNode, Bouten, BoutenKind, BoutenPosition,
    ContainerKind, Content, DoubleRuby, Gaiji, HeadingHint, Indent, Kaeriten, Ruby, Sashie,
    SectionKind, Segment, Span, TateChuYoko,
};

use crate::diagnostic::Diagnostic;
use crate::phase2_pair::{PairEvent, PairKind, PairOutput};
use crate::token::TriggerKind;

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
/// Phase 4 maps the variants to PUA sentinels as follows:
///
/// | variant        | sentinel              | `post_process` role |
/// |----------------|-----------------------|-------------------|
/// | `Plain`        | verbatim source bytes | — |
/// | `Newline`      | verbatim `\n`         | — |
/// | `Aozora(n)`    | `E001` if inline, `E002` if block-leaf | splice Aozora node into comrak AST |
/// | `BlockOpen`    | `E003`                | pair with matching `BlockClose` |
/// | `BlockClose`   | `E004`                | close nearest unclosed `BlockOpen` |
///
/// The `BlockOpen` / `BlockClose` split exists because paired
/// containers (`ここから字下げ` … `ここで字下げ終わり`) span arbitrary
/// content between the two markers. The lexer emits both markers as
/// independent spans and lets `post_process` walk the AST to wrap
/// sibling nodes in the container — see ADR-0008.
///
/// # Memory layout
///
/// The `Aozora` variant carries a [`Box<AozoraNode>`][AozoraNode]
/// rather than the `AozoraNode` inline. `AozoraNode` is the fattest
/// inhabitant of the enum (~56 bytes for variants like Ruby with two
/// `Content` fields), and the spans Vec is dominated by `Plain` /
/// `Newline` spans on real input. Boxing the rare-and-fat variant
/// pulls the enum down to ~16 bytes per span and cuts the spans Vec
/// by roughly 4× on a typical document, at the cost of one
/// indirection per Aozora dispatch in Phase 4 (a single deref that
/// the cache hierarchy amortises against the Aozora-variant clone
/// that follows immediately).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanKind {
    /// Source bytes that carry no Aozora construct. Emitted verbatim
    /// by Phase 4.
    Plain,
    /// Classified Aozora construct (inline span or block-leaf line).
    /// Phase 4 replaces the source span with an `E001` (inline) or
    /// `E002` (block-leaf) sentinel and records the node in the
    /// placeholder registry keyed at the sentinel's normalized
    /// position. Boxed because `AozoraNode` is the fattest variant
    /// of this enum (~56 bytes); the indirection shrinks the spans
    /// `Vec` by ~4× on real input — see the type-level docs above.
    Aozora(Box<AozoraNode>),
    /// Paired-container opener — `［＃ここから字下げ］`, `［＃罫囲み］`,
    /// etc. Phase 4 emits an `E003` sentinel line; `post_process`
    /// matches it to the corresponding `BlockClose` via a balanced
    /// stack walk of the comrak AST.
    BlockOpen(ContainerKind),
    /// Paired-container closer — `［＃ここで字下げ終わり］`,
    /// `［＃罫囲み終わり］`, etc. Phase 4 emits an `E004` sentinel
    /// line; the carried `ContainerKind` is a hint used by
    /// `post_process` to diagnose `［＃罫囲み終わり］` closing an
    /// `Indent` opener (kind mismatch).
    BlockClose(ContainerKind),
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
    let events = &pair_output.events;
    let mut driver = Driver::new(source);
    let mut i = 0;
    while i < events.len() {
        if let Some(consumed) = driver.try_recognize(events, i) {
            i += consumed;
        } else {
            driver.accept(&events[i]);
            i += 1;
        }
    }
    driver.finish(pair_output.diagnostics.clone())
}

/// Mutable state for the event-walk.
///
/// `pending_plain_start` is `Some(start_byte)` when the driver is in
/// the middle of accumulating a Plain span; `None` when the last span
/// emitted was a Newline or a classified Aozora span (or nothing yet).
/// Flushing the pending plain span is the only place Plain spans are
/// produced.
struct Driver<'s> {
    source_len: u32,
    source: &'s str,
    spans: Vec<ClassifiedSpan>,
    pending_plain_start: Option<u32>,
}

impl<'s> Driver<'s> {
    fn new(source: &'s str) -> Self {
        Self {
            source_len: u32::try_from(source.len()).expect("sanitize asserts fit in u32"),
            source,
            spans: Vec::new(),
            pending_plain_start: None,
        }
    }

    /// Attempt to recognize an Aozora construct at `i`. Returns
    /// `Some(consumed_event_count)` on a match (with the corresponding
    /// Aozora span emitted and pending plain truncated); `None` leaves
    /// the event for the fallback `accept` path.
    fn try_recognize(&mut self, events: &[PairEvent], i: usize) -> Option<usize> {
        match events[i] {
            PairEvent::PairOpen {
                kind: PairKind::Ruby,
                close_idx,
                ..
            } => self.try_ruby(events, i, close_idx),
            PairEvent::PairOpen {
                kind: PairKind::DoubleRuby,
                close_idx,
                ..
            } => self.try_double_ruby(events, i, close_idx),
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                close_idx,
                ..
            } => self.try_bracket_annotation(events, i, close_idx),
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } => self.try_gaiji(events, i, span),
            _ => None,
        }
    }

    /// Classify a `《《…》》` pair as an [`AozoraNode::DoubleRuby`]. The
    /// body events are walked by [`build_content_from_body`] so any
    /// nested gaiji / annotation fold into the payload `Content`
    /// rather than leaking to the top-level span list.
    ///
    /// Empty `《《》》` pairs are still consumed — otherwise the bare
    /// double brackets would leak to plain text and confuse a reader
    /// (they look like a missing body). The renderer emits `≪≫` in
    /// that case; the `afm-double-ruby` wrapper class is always
    /// applied so stylesheets can size the academic brackets
    /// correctly.
    fn try_double_ruby(
        &mut self,
        events: &[PairEvent],
        open_idx: usize,
        close_idx: usize,
    ) -> Option<usize> {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = events[open_idx]
        else {
            return None;
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = events[close_idx]
        else {
            return None;
        };
        let content = build_content_from_body(
            events,
            self.source,
            &BodyWindow {
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );
        self.flush_plain_up_to(open_span.start);
        self.spans.push(ClassifiedSpan {
            kind: SpanKind::Aozora(Box::new(AozoraNode::DoubleRuby(DoubleRuby { content }))),
            source_span: Span::new(open_span.start, close_span.end),
        });
        self.pending_plain_start = None;
        Some(close_idx - open_idx + 1)
    }

    fn try_ruby(
        &mut self,
        events: &[PairEvent],
        open_idx: usize,
        close_idx: usize,
    ) -> Option<usize> {
        let m = recognize_ruby(events, self.source, open_idx, close_idx)?;
        // Truncate any in-progress plain run to end exactly where the
        // ruby takes over. If `pending_plain_start >= consume_start`
        // the pending span is empty and dropped — common for explicit
        // ruby right after a newline.
        self.flush_plain_up_to(m.consume_start);
        self.spans.push(ClassifiedSpan {
            kind: SpanKind::Aozora(Box::new(AozoraNode::Ruby(Ruby {
                base: Content::from(m.base),
                reading: m.reading,
                delim_explicit: m.explicit,
            }))),
            source_span: Span::new(m.consume_start, m.consume_end),
        });
        self.pending_plain_start = None;
        Some(close_idx - open_idx + 1)
    }

    fn try_bracket_annotation(
        &mut self,
        events: &[PairEvent],
        open_idx: usize,
        close_idx: usize,
    ) -> Option<usize> {
        let m = recognize_annotation(events, self.source, open_idx, close_idx)?;
        self.flush_plain_up_to(m.consume_start);
        let kind = match m.emit {
            EmitKind::Aozora(node) => SpanKind::Aozora(Box::new(node)),
            EmitKind::BlockOpen(container) => SpanKind::BlockOpen(container),
            EmitKind::BlockClose(container) => SpanKind::BlockClose(container),
        };
        self.spans.push(ClassifiedSpan {
            kind,
            source_span: Span::new(m.consume_start, m.consume_end),
        });
        self.pending_plain_start = None;
        Some(close_idx - open_idx + 1)
    }

    fn try_gaiji(
        &mut self,
        events: &[PairEvent],
        refmark_idx: usize,
        refmark_span: Span,
    ) -> Option<usize> {
        let bracket_open_idx = refmark_idx + 1;
        let &PairEvent::PairOpen {
            kind: PairKind::Bracket,
            close_idx,
            ..
        } = events.get(bracket_open_idx)?
        else {
            return None;
        };
        let m = recognize_gaiji(events, self.source, refmark_span, bracket_open_idx)?;
        self.flush_plain_up_to(m.consume_start);
        self.spans.push(ClassifiedSpan {
            kind: SpanKind::Aozora(Box::new(m.node)),
            source_span: Span::new(m.consume_start, m.consume_end),
        });
        self.pending_plain_start = None;
        // RefMark + entire bracket pair events: 1 + (close_idx - bracket_open_idx + 1)
        Some(close_idx - refmark_idx + 1)
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

        // Un-classified: merge into the pending plain run. Every
        // non-Newline PairEvent carries a span. The end is implicitly
        // tracked by the next event's start or the end-of-stream
        // finish pass — the 1:1 token↔event invariant from Phase 2,
        // combined with Phase 1's contiguous byte coverage, means
        // sequential event spans meet end-to-start with no gaps.
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

/// Intermediate result of [`recognize_ruby`]. `base` stays borrowed
/// (the two forms we handle — explicit `｜X《Y》` and implicit
/// trailing-kanji — both come from a single [`PairEvent::Text`] event
/// with no nested structure). `reading`, on the other hand, can carry
/// embedded gaiji (`※［＃…］`) or annotations (`［＃ママ］`), so it is
/// already resolved into a [`Content`] via [`build_content_from_body`].
///
/// Collapsing inside the lexer (rather than leaving the splitting to
/// the renderer) keeps the [`AozoraNode`] payload self-contained:
/// Phase 4 stamps one PUA sentinel over the whole `｜…《…》` source
/// span, and the inner gaiji/annotation never reach the top-level
/// `spans` list or the comrak parse phase.
struct RubyMatch<'s> {
    base: &'s str,
    reading: Content,
    explicit: bool,
    consume_start: u32,
    consume_end: u32,
}

/// Try to recognize a Ruby span at `events[open_idx]`.
///
/// Two shapes per the Aozora annotation manual
/// (<https://www.aozora.gr.jp/annotation/ruby.html>):
///
/// * **Explicit** — `｜X《Y》`. A [`TriggerKind::Bar`] `Solo` two
///   events before the [`PairKind::Ruby`] open marks the full base.
///   Any Text, not just kanji, may be the base.
/// * **Implicit** — `…X《Y》` where the preceding Text event ends in
///   a run of ideographs. The base is the trailing kanji run of that
///   Text; any non-kanji prefix remains plain.
///
/// The `《…》` reading body is walked with [`build_content_from_body`]
/// so nested `※［＃…］` gaiji and `［＃…］` annotations fold into the
/// returned `Content` as `Segment::Gaiji` / `Segment::Annotation`.
/// Pure-text readings collapse back to [`Content::Plain`] via
/// [`Content::from_segments`].
///
/// Returns `None` if neither shape applies (empty reading, no
/// preceding Text, no kanji for implicit).
fn recognize_ruby<'s>(
    events: &[PairEvent],
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<RubyMatch<'s>> {
    let PairEvent::PairOpen {
        span: open_span, ..
    } = events[open_idx]
    else {
        return None;
    };
    let PairEvent::PairClose {
        span: close_span, ..
    } = events[close_idx]
    else {
        return None;
    };
    if open_span.end >= close_span.start {
        // Empty reading — the `《…》` body has no bytes.
        return None;
    }
    if open_idx == 0 {
        return None;
    }
    let PairEvent::Text {
        range: prev_range, ..
    } = events[open_idx - 1]
    else {
        return None;
    };
    let prev_text = &source[prev_range.start as usize..prev_range.end as usize];

    let reading = build_content_from_body(
        events,
        source,
        &BodyWindow {
            events: open_idx + 1..close_idx,
            bytes: open_span.end..close_span.start,
        },
    );

    // Explicit form: Solo(Bar) two events before the open, with the
    // Text between them acting as the base.
    if open_idx >= 2
        && let PairEvent::Solo {
            kind: TriggerKind::Bar,
            span: bar_span,
        } = events[open_idx - 2]
    {
        if prev_text.is_empty() {
            return None;
        }
        return Some(RubyMatch {
            base: prev_text,
            reading,
            explicit: true,
            consume_start: bar_span.start,
            consume_end: close_span.end,
        });
    }

    // Implicit form: trailing-kanji run of the preceding Text.
    let kanji_offset = trailing_kanji_start(prev_text);
    if kanji_offset == prev_text.len() {
        return None;
    }
    let consume_start =
        prev_range.start + u32::try_from(kanji_offset).expect("kanji offset fits in u32");
    Some(RubyMatch {
        base: &prev_text[kanji_offset..],
        reading,
        explicit: false,
        consume_start,
        consume_end: close_span.end,
    })
}

/// Half-open window into a [`PairEvent`] stream. Bundles the event-
/// index range with the matching byte-offset range so
/// [`build_content_from_body`] can flush text segments using source
/// byte slices without re-derefing event spans on every iteration.
///
/// The two ranges are redundant in principle — `bytes.start` always
/// equals `events[events.start]`'s leading edge — but caching them
/// avoids a branch when the range is empty and makes the helper
/// signature honest about what it needs.
struct BodyWindow {
    events: Range<usize>,
    bytes: Range<u32>,
}

/// Walk `window` over `events` and build the corresponding
/// [`Content`].
///
/// Each nested `※［＃description、mencode］` reduces to a
/// [`Segment::Gaiji`] via [`recognize_gaiji`]; each standalone
/// `［＃…］` reduces to a [`Segment::Annotation`] via
/// [`recognize_annotation`]. Every other byte (plain text, stray
/// triggers, unmatched delimiters) is captured into adjacent
/// [`Segment::Text`] runs by tracking a single "outstanding text
/// start" byte offset and flushing only when a recognisable construct
/// consumes the intervening bytes.
///
/// Non-Annotation Aozora emits (a paired-container opener, a block
/// leaf, etc.) are *not* first-class segments and are folded back
/// into `Annotation{Unknown}` with the raw bracket bytes — this keeps
/// the Tier-A canary intact inside a ruby body regardless of how
/// unusual the inner annotation shape is.
///
/// ## Fast path
///
/// [`has_nested_candidate`] first short-circuits the body scan: when
/// no `Solo(RefMark)` and no `PairOpen(Bracket)` appear, the body is
/// guaranteed to be plain text (possibly peppered with unrelated
/// triggers like `｜` or mismatched quotes, which we treat as text).
/// Returning `Content::from(&str)` in that branch skips the `Vec`
/// allocation and the `from_segments` collapse pass — a win for the
/// 99%+ of ruby readings that carry no embedded structure.
///
/// ## Slow path
///
/// The fallback is a single `O(body_events)` sweep. `text_start`
/// tracks the earliest byte that has not yet been committed to a Text
/// segment; flushing is strictly triggered by a *recognised* nested
/// construct, so unrelated events cost a single index increment. Each
/// recognition jumps to `close_idx + 1` using Phase 2's pre-linked
/// pair indices, keeping the sweep strictly forward-only regardless
/// of nesting depth.
///
/// The returned value is always normalised via
/// [`Content::from_segments`], so a slow-path body that turned out to
/// contain only text (for example because its brackets were malformed
/// and skipped) still collapses back to [`Content::Plain`].
fn build_content_from_body(events: &[PairEvent], source: &str, window: &BodyWindow) -> Content {
    debug_assert!(
        window.events.start <= window.events.end,
        "body window event range must be non-inverted",
    );
    debug_assert!(
        window.bytes.start <= window.bytes.end,
        "body window byte range must be non-inverted",
    );

    let body_events = &events[window.events.start..window.events.end];
    if !has_nested_candidate(body_events) {
        // Fast path: no `※` and no `［` in the body; bytes pass
        // through verbatim. `Content::from(&str)` maps empty input to
        // `Content::Segments([])` and otherwise to `Content::Plain`.
        let text = &source[window.bytes.start as usize..window.bytes.end as usize];
        return Content::from(text);
    }

    // Slow path: at least one potential nested construct exists.
    // Pre-size the segment vector: worst case is `ceil(n / 2)` runs of
    // `Text, Construct, Text, …` plus one trailing Text. Capping at
    // `body_events.len() + 1` is a safe upper bound that is small in
    // practice (ruby readings almost never reach double-digit events).
    let mut segments: Vec<Segment> = Vec::with_capacity(body_events.len() + 1);
    let mut text_start: u32 = window.bytes.start;
    let mut i = window.events.start;

    while i < window.events.end {
        // Shape 1: `※［＃…］` — Solo(RefMark) followed by PairOpen(Bracket).
        if let PairEvent::Solo {
            kind: TriggerKind::RefMark,
            span: refmark_span,
        } = events[i]
        {
            let bracket_idx = i + 1;
            if bracket_idx < window.events.end
                && let PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    close_idx,
                    ..
                } = events[bracket_idx]
                && close_idx < window.events.end
                && let Some(g) = recognize_gaiji(events, source, refmark_span, bracket_idx)
            {
                let AozoraNode::Gaiji(gaiji) = g.node else {
                    // `recognize_gaiji` always produces an `AozoraNode::Gaiji`; any
                    // other variant would be a bug in the recogniser itself.
                    unreachable!("recognize_gaiji returned non-Gaiji AozoraNode");
                };
                push_text_segment(&mut segments, source, text_start, g.consume_start);
                segments.push(Segment::Gaiji(gaiji));
                text_start = g.consume_end;
                i = close_idx + 1;
                continue;
            }
        }

        // Shape 2: `［＃…］` — a standalone bracket annotation. The
        // RefMark+Bracket combo above has already had its chance to
        // claim this event, so here we handle the remaining brackets.
        // `recognize_annotation` has an Unknown catch-all and only
        // returns `None` for malformed brackets (no `＃` sentinel);
        // those fall through to `i += 1` and the bracket bytes stay
        // inside the pending Text run.
        if let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            close_idx,
            span: open_span,
        } = events[i]
            && close_idx < window.events.end
            && let Some(a) = recognize_annotation(events, source, i, close_idx)
        {
            let PairEvent::PairClose {
                span: close_span, ..
            } = events[close_idx]
            else {
                // PairOpen's `close_idx` always targets a PairClose of the same
                // kind; anything else would be a Phase 2 invariant violation.
                unreachable!("PairOpen close_idx must target a PairClose");
            };
            // The emit may be a non-Annotation node (e.g. a nested
            // block leaf or container marker) which has no home in a
            // Segment. Downgrade those to `Annotation{Unknown}` so
            // the Tier-A canary (no bare `［＃` in HTML) still holds.
            let annotation = match a.emit {
                EmitKind::Aozora(AozoraNode::Annotation(ann)) => ann,
                _ => Annotation {
                    raw: source[open_span.start as usize..close_span.end as usize].into(),
                    kind: AnnotationKind::Unknown,
                },
            };
            push_text_segment(&mut segments, source, text_start, a.consume_start);
            segments.push(Segment::Annotation(annotation));
            text_start = a.consume_end;
            i = close_idx + 1;
            continue;
        }

        i += 1;
    }

    push_text_segment(&mut segments, source, text_start, window.bytes.end);
    Content::from_segments(segments)
}

/// Whether `body` could host a nested gaiji / annotation. The Phase 2
/// event model guarantees that:
///
/// * `※［＃…］` always emits a `Solo(RefMark)` event at its `※`.
/// * `［＃…］` always emits a `PairOpen(Bracket)` event at its `［`.
///
/// So the absence of both event shapes in the body is sufficient proof
/// that no nested construct can be recognised, allowing
/// [`build_content_from_body`] to take the allocation-free fast path.
fn has_nested_candidate(body: &[PairEvent]) -> bool {
    body.iter().any(|e| {
        matches!(
            e,
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                ..
            } | PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        )
    })
}

/// Append `source[start..end]` to `segments` as a `Segment::Text` if
/// the slice is non-empty. `start == end` occurs naturally when a
/// recognised construct sits at the very start of the body or
/// immediately follows a previous one; skipping those zero-length
/// flushes keeps the post-collapse invariant "no empty `Text` in a
/// `Segments` run" (see `Content::from_segments`) without a second
/// compaction pass.
#[inline]
fn push_text_segment(segments: &mut Vec<Segment>, source: &str, start: u32, end: u32) {
    if end > start {
        segments.push(Segment::Text(source[start as usize..end as usize].into()));
    }
}

/// Intermediate result of [`recognize_gaiji`].
struct GaijiMatch {
    node: AozoraNode,
    consume_start: u32,
    consume_end: u32,
}

/// Try to recognize a gaiji reference at `events[refmark_idx]`.
///
/// Shape: `※［＃<description>、<mencode>］` or `※［＃<description>］`.
/// The description may be wrapped in `「…」` (the common form) or
/// appear bare. `<mencode>` is the mencode reference (`第3水準1-85-54`,
/// `U+XXXX`, etc.) appearing after a `、` separator.
///
/// The UCS resolution column of [`Gaiji`] is populated by
/// `aozora_encoding::gaiji::lookup` before the recogniser returns, so
/// downstream consumers receive a resolved `Option<char>` without
/// having to re-probe the mencode table.
///
/// Event preconditions (checked):
/// * `events[refmark_idx]` is `Solo(RefMark)` [done by caller]
/// * `events[refmark_idx + 1]` is `PairOpen(Bracket)` [done by caller]
/// * `events[refmark_idx + 2]` is `Solo(Hash)` [checked here]
///
/// Consume range is from `refmark_span.start` to the bracket close's
/// end — i.e. the `※` and the entire following `［＃…］` fold into
/// one Aozora span.
fn recognize_gaiji(
    events: &[PairEvent],
    source: &str,
    refmark_span: Span,
    bracket_open_idx: usize,
) -> Option<GaijiMatch> {
    let &PairEvent::PairOpen {
        kind: PairKind::Bracket,
        close_idx: bracket_close_idx,
        ..
    } = events.get(bracket_open_idx)?
    else {
        return None;
    };
    let hash_end = match events.get(bracket_open_idx + 1)? {
        PairEvent::Solo {
            kind: TriggerKind::Hash,
            span,
        } => span.end,
        _ => return None,
    };
    let &PairEvent::PairClose {
        span: bracket_close_span,
        ..
    } = events.get(bracket_close_idx)?
    else {
        return None;
    };

    // Try the quoted-description form first: `「DESC」、MENCODE`. Two
    // events after open: PairOpen(Quote).
    let quote_open_idx = bracket_open_idx + 2;
    let quoted = events.get(quote_open_idx).and_then(|ev| match *ev {
        PairEvent::PairOpen {
            kind: PairKind::Quote,
            span: qos,
            close_idx: qci,
        } if qci < bracket_close_idx => {
            let PairEvent::PairClose { span: qcs, .. } = *events.get(qci)? else {
                return None;
            };
            let desc = &source[qos.end as usize..qcs.start as usize];
            if desc.is_empty() {
                return None;
            }
            let tail = source[qcs.end as usize..bracket_close_span.start as usize].trim();
            let mencode = tail.strip_prefix('、').map(str::trim);
            Some((desc.to_owned(), mencode.map(str::to_owned)))
        }
        _ => None,
    });

    let (description, mencode) = quoted.unwrap_or_else(|| {
        // Bare-description fallback: split body at the first `、`.
        // Whole body after `＃` becomes the description if there's no `、`.
        let body = source[hash_end as usize..bracket_close_span.start as usize].trim();
        if let Some((desc, men)) = body.split_once('、') {
            (desc.trim().to_owned(), Some(men.trim().to_owned()))
        } else {
            (body.to_owned(), None)
        }
    });

    if description.is_empty() {
        return None;
    }
    // Reject descriptions that carry structural quote characters.
    // The serializer wraps the description in `「…」` for round-trip,
    // so a stray `」` (e.g. from the malformed input `※［＃」］`) would
    // make `serialize ∘ parse` non-stable. Falling through to `None`
    // here lets the higher-level classifier wrap the raw bracket in
    // an `Annotation{Unknown}`, which round-trips byte-identical.
    if description.contains(['「', '」']) {
        return None;
    }

    // Resolve the Unicode scalar at lex time via the static table in
    // afm-encoding so the downstream AST / renderer never has to
    // re-probe. `None` stays `None` when the mencode has no mapping
    // entry and no `U+XXXX` shape matches — the renderer falls back
    // to escaping the raw `description`.
    let ucs = gaiji_resolve::lookup(None, mencode.as_deref(), &description);

    Some(GaijiMatch {
        node: AozoraNode::Gaiji(Gaiji {
            description: description.into_boxed_str(),
            ucs,
            mencode: mencode.map(String::into_boxed_str),
        }),
        consume_start: refmark_span.start,
        consume_end: bracket_close_span.end,
    })
}

/// Byte offset where the trailing kanji run in `text` begins.
///
/// Walks chars right-to-left, keeping track of the earliest byte
/// offset reached while every char is a ruby-base char. Returns
/// `text.len()` if the final char is not a ruby-base char (→ no
/// implicit base available).
fn trailing_kanji_start(text: &str) -> usize {
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        if is_ruby_base_char(ch) {
            start = idx;
        } else {
            break;
        }
    }
    start
}

/// Intermediate result of [`recognize_annotation`]. The `emit`
/// variant decides which [`SpanKind`] the driver pushes.
struct AnnotationMatch {
    emit: EmitKind,
    consume_start: u32,
    consume_end: u32,
}

/// What to emit for a matched annotation.
enum EmitKind {
    /// Inline or block-leaf — becomes [`SpanKind::Aozora`].
    Aozora(AozoraNode),
    /// Paired-container opener — becomes [`SpanKind::BlockOpen`].
    BlockOpen(ContainerKind),
    /// Paired-container closer — becomes [`SpanKind::BlockClose`].
    BlockClose(ContainerKind),
}

/// Try to recognize a `［＃keyword…］` annotation at
/// `events[open_idx]`.
///
/// Requires the immediately-next event to be a [`TriggerKind::Hash`]
/// [`PairEvent::Solo`] — the shape `［` `＃` `body` `］`. Bodies
/// without a hash (plain `［…］`) are not annotations; bodies with a
/// hash whose keyword no specialised recogniser matches fall through
/// to the `Annotation { Unknown }` catch-all so the bracket is
/// always consumed into some `AozoraNode`.
fn recognize_annotation(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    close_idx: usize,
) -> Option<AnnotationMatch> {
    let PairEvent::PairOpen {
        span: open_span, ..
    } = events[open_idx]
    else {
        return None;
    };
    let PairEvent::PairClose {
        span: close_span, ..
    } = events[close_idx]
    else {
        return None;
    };

    // The next event must be `＃`. `open_idx + 1 < close_idx` is
    // guaranteed whenever the hash exists, and `close_idx > open_idx`
    // always holds for a surviving PairOpen.
    let hash_end = match events.get(open_idx + 1)? {
        PairEvent::Solo {
            kind: TriggerKind::Hash,
            span,
        } => span.end,
        _ => return None,
    };

    // Body bytes are everything between `＃` and `］`. Trim leading /
    // trailing ASCII whitespace to be resilient to malformed input
    // like `［＃ 改ページ  ］`; Aozora spec does not officially allow
    // such whitespace but the corpus contains stragglers.
    let body = source[hash_end as usize..close_span.start as usize].trim();

    let emit = classify_fixed_keyword(body)
        .map(EmitKind::Aozora)
        .or_else(|| classify_kaeriten(body).map(EmitKind::Aozora))
        .or_else(|| classify_indent_or_align(body).map(EmitKind::Aozora))
        .or_else(|| classify_sashie(body).map(EmitKind::Aozora))
        .or_else(|| classify_inline_warichu(body).map(EmitKind::Aozora))
        .or_else(|| {
            classify_forward_bouten(events, source, open_idx, close_idx).map(EmitKind::Aozora)
        })
        .or_else(|| classify_forward_tcy(events, source, open_idx, close_idx).map(EmitKind::Aozora))
        .or_else(|| {
            classify_forward_heading(events, source, open_idx, close_idx).map(EmitKind::Aozora)
        })
        .or_else(|| classify_container_open(body))
        .or_else(|| classify_container_close(body))
        .or_else(|| {
            // Catch-all fallback for any well-formed `［＃…］` whose body
            // no specialised recogniser claimed — including empty
            // bodies (`［＃］`), which real Aozora corpora occasionally
            // use as illustrative glyphs inside explanatory prose.
            // Emitting `Annotation { Unknown }` with the raw source
            // slice keeps the Tier-A canary (no bare `［＃` in HTML
            // output) intact: the renderer wraps the raw bytes in an
            // `afm-annotation` hidden span regardless of body shape.
            // The lexer is the sole owner of this classification —
            // comrak's parse phase never sees `［＃…］`.
            let raw = &source[open_span.start as usize..close_span.end as usize];
            Some(EmitKind::Aozora(AozoraNode::Annotation(Annotation {
                raw: raw.into(),
                kind: AnnotationKind::Unknown,
            })))
        })?;

    Some(AnnotationMatch {
        emit,
        consume_start: open_span.start,
        consume_end: close_span.end,
    })
}

/// Fixed-string annotation keywords — no parameters, no body
/// variations. Each entry corresponds to a single constant
/// [`AozoraNode`].
fn classify_fixed_keyword(body: &str) -> Option<AozoraNode> {
    Some(match body {
        "改ページ" => AozoraNode::PageBreak,
        "改丁" => AozoraNode::SectionBreak(SectionKind::Choho),
        "改段" => AozoraNode::SectionBreak(SectionKind::Dan),
        "改見開き" => AozoraNode::SectionBreak(SectionKind::Spread),
        "地付き" => AozoraNode::AlignEnd(AlignEnd { offset: 0 }),
        _ => return None,
    })
}

/// Parameterized indent / end-alignment annotations:
///
/// * `N字下げ`       → `Indent { amount: N }`
/// * `地からN字上げ` → `AlignEnd { offset: N }`
///
/// The `N` prefix accepts ASCII digits (`0-9`) and full-width digits
/// (`０-９`); both conventions appear in Aozora corpora. 漢数字 is not
/// accepted here (rare for indent amounts, and ambiguous to parse
/// without a full reader). Invalid or unsupported shapes return
/// `None` so the body flows to the next recognizer or to Plain.
fn classify_indent_or_align(body: &str) -> Option<AozoraNode> {
    if let Some(rest) = body.strip_prefix("地から")
        && let Some((n, tail)) = parse_decimal_u8_prefix(rest)
        && tail == "字上げ"
        && n >= 1
    {
        return Some(AozoraNode::AlignEnd(AlignEnd { offset: n }));
    }
    let (n, tail) = parse_decimal_u8_prefix(body)?;
    // `N字下げ` requires N >= 1 per the Aozora annotation spec — a
    // zero-width indent is not meaningful. Reject and let the body fall
    // through to the generic Annotation classifier.
    if tail == "字下げ" && n >= 1 {
        return Some(AozoraNode::Indent(Indent { amount: n }));
    }
    None
}

/// Classify a `［＃「target」に<bouten-kind>］` forward-reference
/// bouten annotation.
///
/// Uses the event-stream layout to find the target quote pair,
/// avoiding the string-find-first-`」` pitfall when the target text
/// itself contains nested `「…」`. Phase 2 has already balanced the
/// quotes so the target's extent is unambiguous.
///
/// Expected event layout for a valid forward bouten:
///
/// ```text
/// open_idx         PairOpen(Bracket)
/// open_idx + 1     Solo(Hash)                [already verified]
/// open_idx + 2     PairOpen(Quote, close=Q)
/// …                body events               [usually just Text]
/// Q                PairClose(Quote)
/// Q+1..close_idx   suffix events             [usually Text("に…")]
/// close_idx        PairClose(Bracket)
/// ```
fn classify_forward_bouten(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    close_idx: usize,
) -> Option<AozoraNode> {
    let extracted = extract_forward_quote_targets(events, source, open_idx, close_idx)?;
    // Shape 1: `に<kind>` — default right-side placement.
    // Shape 2: `の左に<kind>` — left-side placement (position flipped).
    let (position, kind_suffix) = if let Some(rest) = extracted.suffix.strip_prefix("に") {
        (BoutenPosition::Right, rest)
    } else if let Some(rest) = extracted.suffix.strip_prefix("の左に") {
        (BoutenPosition::Left, rest)
    } else {
        return None;
    };
    let kind = bouten_kind_from_suffix(kind_suffix)?;
    // A forward-reference bouten only makes sense when every named
    // target actually appears in the preceding text. Otherwise it
    // has no referent and we fall through to the Annotation{Unknown}
    // catch-all so the reader sees the raw `［＃…］` rather than a
    // mysterious styling applied to nothing. Each target is checked
    // independently so a partially-valid multi-quote bracket (rare
    // but present in corpora) still fails cleanly.
    for target in &extracted.targets {
        if !forward_target_is_preceded(events, source, open_idx, target) {
            return None;
        }
    }
    Some(AozoraNode::Bouten(Bouten {
        kind,
        target: build_bouten_target(&extracted.targets),
        position,
    }))
}

/// Fold a list of forward-bouten target strings into a single
/// [`Content`]. A one-element list takes the `Content::from(&str)`
/// fast path (the overwhelmingly common case); multi-target lists
/// build a `Segments` run where inter-target separators are modelled
/// as `Segment::Text("、")` so the renderer emits
/// `<em>A、B</em>` in document order.
///
/// Using `、` as the glue is a deliberate, lossy choice: the raw
/// source shape `「A」「B」` does not have an explicit separator, but
/// inserting one in the rendered output makes the targets readable
/// without requiring a dedicated `Segment::Separator` variant (which
/// would ripple through every renderer / serializer). Callers that
/// need the per-target list can walk `Content::iter` and filter on
/// `SegmentRef::Text`.
fn build_bouten_target(targets: &[&str]) -> Content {
    match targets {
        [] => Content::default(),
        [only] => Content::from(*only),
        many => {
            let mut segs: Vec<Segment> = Vec::with_capacity(many.len() * 2 - 1);
            for (i, t) in many.iter().enumerate() {
                if i > 0 {
                    segs.push(Segment::Text("、".into()));
                }
                segs.push(Segment::Text((*t).into()));
            }
            Content::from_segments(segs)
        }
    }
}

/// Classify a `［＃「target」は縦中横］` forward-reference
/// tate-chu-yoko (horizontal-in-vertical) annotation.
///
/// Same event-layout expectations as forward bouten, except the
/// suffix uses the particle `は` and the keyword `縦中横`. Paired
/// form (`［＃縦中横］…［＃縦中横終わり］`) is handled by the
/// paired-container classifier and not matched here.
///
/// Multi-quote `［＃「A」「B」は縦中横］` bodies are not standard Aozora
/// spec; we accept the first target's text and ignore the rest for
/// robustness rather than failing, so the bracket still consumes via
/// [`classify_forward_tcy`] instead of leaking to `Annotation{Unknown}`.
fn classify_forward_tcy(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    close_idx: usize,
) -> Option<AozoraNode> {
    let extracted = extract_forward_quote_targets(events, source, open_idx, close_idx)?;
    if extracted.suffix != "は縦中横" {
        return None;
    }
    let first = extracted.targets.first()?;
    // Same rationale as `classify_forward_bouten` — the styling has no
    // meaning without a preceding target literal.
    if !forward_target_is_preceded(events, source, open_idx, first) {
        return None;
    }
    Some(AozoraNode::TateChuYoko(TateChuYoko {
        text: Content::from(*first),
    }))
}

/// Check whether `target` appears somewhere in the source preceding the
/// `［` event at `open_idx`. Used by forward-reference recognisers to
/// suppress `［＃「X」…］` spans whose target has no referent.
///
/// Returns `false` if the event shape isn't the expected `PairOpen`
/// (defensive — the caller is responsible for having picked a valid
/// bracket, so this only fails if invariants drift).
fn forward_target_is_preceded(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> bool {
    let Some(PairEvent::PairOpen { span, .. }) = events.get(open_idx) else {
        return false;
    };
    let preceding = &source[..span.start as usize];
    preceding.contains(target)
}

/// Result of walking the `［＃「…」「…」…<particle><keyword>］`
/// shape. `targets` holds each non-empty quote body in document order
/// (length `>= 1` when `Some(_)` is returned) and `suffix` is the
/// trimmed source between the last quote's `」` and the bracket's `］`,
/// ready for particle + keyword matching.
struct ForwardQuoteExtract<'s> {
    targets: Vec<&'s str>,
    suffix: &'s str,
}

/// Shared helper for the `［＃「X」…<particle><keyword>］` shape.
///
/// Walks consecutive quote pairs immediately after the `＃` and
/// stops when the next event is *not* another `PairOpen(Quote)`.
/// Returns the collected target list together with the trimmed
/// suffix so callers can match on the particle + keyword portion.
///
/// Returns `None` if any shape assumption fails: no adjacent quote
/// pair, first quote empty, or the initial quote crossing out of the
/// bracket. Subsequent empty quote bodies are silently skipped
/// (defensive against `「」` placeholders in real corpora) rather
/// than aborting the recognition.
fn extract_forward_quote_targets<'s>(
    events: &[PairEvent],
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<ForwardQuoteExtract<'s>> {
    let &PairEvent::PairClose {
        span: bracket_close_span,
        ..
    } = events.get(close_idx)?
    else {
        return None;
    };

    let mut targets: Vec<&'s str> = Vec::new();
    let mut cursor = open_idx + 2; // skip `［` and `＃`
    let mut last_quote_end: u32 = 0;

    while let Some(&PairEvent::PairOpen {
        kind: PairKind::Quote,
        span: quote_open_span,
        close_idx: quote_close_idx,
    }) = events.get(cursor)
    {
        // The quote must close *before* the bracket — a cross-boundary
        // close would mean the quote is not nested inside the bracket.
        if quote_close_idx >= close_idx {
            return None;
        }
        let Some(&PairEvent::PairClose {
            span: quote_close_span,
            ..
        }) = events.get(quote_close_idx)
        else {
            return None;
        };
        // Empty quotes are tolerated in-position but not added to the
        // target list — they carry no semantic content.
        let body = &source[quote_open_span.end as usize..quote_close_span.start as usize];
        if !body.is_empty() {
            targets.push(body);
        }
        last_quote_end = quote_close_span.end;
        cursor = quote_close_idx + 1;
    }

    if targets.is_empty() {
        return None;
    }
    let suffix = source[last_quote_end as usize..bracket_close_span.start as usize].trim();
    Some(ForwardQuoteExtract { targets, suffix })
}

/// Classify a paired-container opener annotation.
///
/// Accepted shapes:
///
/// * `ここから字下げ`      → `Indent { amount: 1 }` (default)
/// * `ここからN字下げ`     → `Indent { amount: N }` (N is ASCII or 全角)
/// * `ここから地付き`       → `AlignEnd { offset: 0 }`
/// * `ここから地からN字上げ` → `AlignEnd { offset: N }`
/// * `罫囲み`               → `Keigakomi`
///
/// `割り注` is claimed earlier by [`classify_inline_warichu`] and
/// emitted as an inline annotation pair rather than a block container —
/// see the Aozora spec note at
/// <https://www.aozora.gr.jp/annotation/etc.html#warichu> deprecating
/// the `ここから割り注` / `ここで割り注終わり` block form.
///
/// Returns `None` for closers or unknown bodies; those go to
/// [`classify_container_close`] or fall through to Plain.
fn classify_container_open(body: &str) -> Option<EmitKind> {
    if let Some(rest) = body.strip_prefix("ここから") {
        if rest == "字下げ" {
            return Some(EmitKind::BlockOpen(ContainerKind::Indent { amount: 1 }));
        }
        if rest == "地付き" {
            return Some(EmitKind::BlockOpen(ContainerKind::AlignEnd { offset: 0 }));
        }
        if let Some(inner) = rest.strip_prefix("地から")
            && let Some((n, tail)) = parse_decimal_u8_prefix(inner)
            && tail == "字上げ"
        {
            return Some(EmitKind::BlockOpen(ContainerKind::AlignEnd { offset: n }));
        }
        if let Some((n, tail)) = parse_decimal_u8_prefix(rest)
            && tail == "字下げ"
        {
            return Some(EmitKind::BlockOpen(ContainerKind::Indent { amount: n }));
        }
        return None;
    }
    match body {
        "罫囲み" => Some(EmitKind::BlockOpen(ContainerKind::Keigakomi)),
        _ => None,
    }
}

/// Classify a paired-container closer annotation.
///
/// Accepted shapes (corresponding to the opener list above):
///
/// * `ここで字下げ終わり` → `Indent { amount: 0 }` (amount is a placeholder)
/// * `ここで地付き終わり` → `AlignEnd { offset: 0 }`
/// * `罫囲み終わり`         → `Keigakomi`
///
/// `割り注終わり` is claimed by [`classify_inline_warichu`]; see that
/// function's docs for the rationale.
///
/// The carried [`ContainerKind`] only conveys the *variant* — the
/// numeric field (`amount` / `offset`) is a placeholder because the
/// closer does not restate it. `post_process` compares open and close
/// by variant, not by field value.
fn classify_container_close(body: &str) -> Option<EmitKind> {
    let rest = body.strip_suffix("終わり")?;
    if rest == "ここで字下げ" {
        return Some(EmitKind::BlockClose(ContainerKind::Indent { amount: 0 }));
    }
    if rest == "ここで地付き" {
        return Some(EmitKind::BlockClose(ContainerKind::AlignEnd { offset: 0 }));
    }
    match rest {
        "罫囲み" => Some(EmitKind::BlockClose(ContainerKind::Keigakomi)),
        _ => None,
    }
}

/// Classify `［＃割り注］` / `［＃割り注終わり］` as paired inline
/// annotations.
///
/// The Aozora annotation spec
/// (<https://www.aozora.gr.jp/annotation/etc.html#warichu>) says:
/// > かつては［＃ここから割り注］〜［＃ここで割り注終わり］と書いていましたが、
/// > 行中の注記の書式をそろえるために、変更しました。
///
/// So `割り注` is always inline. Emitting each open / close as a
/// separate [`AnnotationKind::WarichuOpen`] / [`AnnotationKind::WarichuClose`]
/// annotation keeps them in the inline token stream — the renderer
/// emits `<span class="afm-warichu">` on open and `</span>` on close,
/// producing the canonical `<span class="afm-warichu">…</span>` shape
/// without splitting the host paragraph the way a block container
/// would.
///
/// Returns `None` for bodies other than the two warichu markers.
fn classify_inline_warichu(body: &str) -> Option<AozoraNode> {
    match body {
        "割り注" => Some(AozoraNode::Annotation(Annotation {
            raw: "［＃割り注］".into(),
            kind: AnnotationKind::WarichuOpen,
        })),
        "割り注終わり" => Some(AozoraNode::Annotation(Annotation {
            raw: "［＃割り注終わり］".into(),
            kind: AnnotationKind::WarichuClose,
        })),
        _ => None,
    }
}

/// Classify a `［＃<mark>］` kaeriten (Chinese-reading order mark).
///
/// Three shapes recognised — see
/// <https://www.aozora.gr.jp/annotation/kunten.html>:
///
/// 1. **Canonical single-char marks** — `一` / `二` / `三` / `四` /
///    `上` / `中` / `下` / `レ` / `甲` / `乙` / `丙` / `丁`. Binary-
///    searched against a sorted `&[&str]` table for O(log n) lookup.
/// 2. **Compound marks** — `一レ`, `二レ`, `三レ`, `上レ`, `中レ`,
///    `下レ`. These pair an order mark with the reversal mark; they
///    render identically to their canonical counterparts (the CSS
///    theme can differentiate via content-based selectors).
/// 3. **送り仮名 (okurigana)** — `（X）` where X is 1–6 CJK characters
///    (hiragana / katakana / kanji). Kept verbatim as the Kaeriten
///    mark so the renderer can emit `<sup>（X）</sup>`. The canonical
///    use is supplying a Japanese particle reading for a Chinese
///    character where a full ruby run would be overkill.
///
/// Other single-character bodies fall through to other classifiers.
/// The shared [`AozoraNode::Kaeriten`] payload keeps the renderer
/// schema-uniform; per-shape styling is a CSS concern via attribute
/// selectors on `afm-kaeriten` + content.
fn classify_kaeriten(body: &str) -> Option<AozoraNode> {
    // Kept sorted so a future migration to `binary_search` is a
    // one-liner; today's 18 entries fit comfortably in a linear
    // `contains` scan.
    const CANONICAL: &[&str] = &[
        "一", "丁", "三", "上", "下", "中", "丙", "乙", "二", "四", "甲", "レ",
    ];
    const COMPOUND: &[&str] = &["一レ", "上レ", "下レ", "中レ", "二レ", "三レ"];
    if CANONICAL.contains(&body) || COMPOUND.contains(&body) {
        return Some(AozoraNode::Kaeriten(Kaeriten { mark: body.into() }));
    }
    if is_okurigana_body(body) {
        return Some(AozoraNode::Kaeriten(Kaeriten { mark: body.into() }));
    }
    None
}

/// Whether `body` is the okurigana shape `（X）` where X is a short
/// run of Japanese characters.
///
/// The length bound guards against accidentally claiming long
/// parenthesised glosses (which belong to the generic annotation
/// catch-all). 6 characters is the ~99th-percentile okurigana length
/// in Aozora corpora; anything longer is practically always editorial
/// prose rather than an inflection marker.
fn is_okurigana_body(body: &str) -> bool {
    let Some(inner) = body.strip_prefix('（').and_then(|s| s.strip_suffix('）')) else {
        return false;
    };
    // Empty parens are not meaningful okurigana.
    let char_count = inner.chars().count();
    if !(1..=6).contains(&char_count) {
        return false;
    }
    inner.chars().all(is_okurigana_char)
}

/// Character class accepted inside okurigana parens: hiragana,
/// katakana (incl. half-width), CJK unified ideographs. Deliberately
/// narrower than "any non-whitespace" so editorial `（注）` or
/// punctuation-rich glosses fall through to the annotation path.
const fn is_okurigana_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3041}'..='\u{309F}'      // hiragana
        | '\u{30A0}'..='\u{30FF}'    // katakana
        | '\u{FF66}'..='\u{FF9F}'    // half-width katakana
        | '\u{4E00}'..='\u{9FFF}'    // CJK unified
        | '\u{3400}'..='\u{4DBF}'    // CJK ext A
        | '\u{F900}'..='\u{FAFF}'    // CJK compat
    )
}

/// Classify a `［＃挿絵（file）入る］` sashie (illustration insert).
///
/// Captures the filename between `（` and `）`; the rest of the body
/// must be exactly `入る`. The captioned form
/// (`［＃挿絵（file）「caption」入る］`) needs an event-level caption
/// recogniser that this pass does not yet perform; the no-caption
/// shape accounts for the vast majority of corpus occurrences.
fn classify_sashie(body: &str) -> Option<AozoraNode> {
    let rest = body.strip_prefix("挿絵（")?;
    // `）` is a full-width right parenthesis (U+FF09). Find its first
    // occurrence — corpus rarely nests `（）` inside a filename.
    let close_off = rest.find('）')?;
    let file = &rest[..close_off];
    if file.is_empty() {
        return None;
    }
    let tail = &rest[close_off + '）'.len_utf8()..];
    if tail != "入る" {
        return None;
    }
    Some(AozoraNode::Sashie(Sashie {
        file: file.into(),
        caption: None,
    }))
}

/// Classify a `［＃「target」は(大|中|小)見出し］` forward-reference
/// heading annotation.
///
/// Shares the event-stream extraction helper with [`classify_forward_bouten`]
/// — the quote-delimited target and the trailing keyword live in the same
/// `［＃「X」…］` shape. The suffix after the target must start with `は`
/// (unlike bouten's `に`), and the keyword selects the Markdown heading
/// level: `大見出し` → 1, `中見出し` → 2, `小見出し` → 3.
///
/// The docs in [`crate`] and ADR-0008 call out that 大/中/小 headings are
/// promoted to `comrak::NodeValue::Heading` by `afm-parser::post_process`;
/// this classifier only marks the position. 窓見出し / 副見出し remain
/// first-class on [`AozoraNode::AozoraHeading`] via a separate path.
///
/// Same `forward_target_is_preceded` gate as forward bouten: a heading
/// hint that names a target which does not appear in the preceding
/// source text is rejected — the annotation has no referent and the
/// paragraph would promote to an empty heading. Falling through lets
/// the catch-all emit `Annotation { Unknown }` so the reader at least
/// sees the raw bracket text in diagnostics.
fn classify_forward_heading(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    close_idx: usize,
) -> Option<AozoraNode> {
    let extracted = extract_forward_quote_targets(events, source, open_idx, close_idx)?;
    let rest = extracted.suffix.strip_prefix("は")?;
    let level = heading_level_from_suffix(rest)?;

    // Reject hints whose targets are not preceded by matching text.
    // See `classify_forward_bouten` for the same rationale.
    for target in &extracted.targets {
        if target.is_empty() {
            continue;
        }
        if !forward_target_is_preceded(events, source, open_idx, target) {
            return None;
        }
    }

    // Concatenate targets in the (rare) multi-quote case so the full
    // named run drives the heading content. For the 17 k-work corpus
    // this is always a single quote, but the concat keeps the shape
    // parallel to forward bouten.
    let combined: String = extracted.targets.iter().copied().collect();
    if combined.is_empty() {
        return None;
    }

    Some(AozoraNode::HeadingHint(HeadingHint {
        level,
        target: combined.into_boxed_str(),
    }))
}

/// Map the keyword after `は` to a Markdown heading level per the
/// Aozora annotation manual
/// (<https://www.aozora.gr.jp/annotation/heading.html>). Only the three
/// first-class levels are recognised; 窓見出し / 副見出し remain on
/// `AozoraHeading`.
fn heading_level_from_suffix(s: &str) -> Option<u8> {
    Some(match s {
        "大見出し" => 1,
        "中見出し" => 2,
        "小見出し" => 3,
        _ => return None,
    })
}

/// Map the trailing keyword (after `に`) to a [`BoutenKind`].
///
/// Covers the eleven bouten kinds catalogued at
/// <https://www.aozora.gr.jp/annotation/bouten.html> plus the common
/// emphasis-page variants (`白ゴマ` / `ばつ` / `白三角` / `二重傍線`).
/// Unknown suffixes return `None`, letting the annotation fall through
/// to the `Annotation{Unknown}` catch-all.
///
/// The dispatch is a straight `match` rather than a PHF table: 11
/// entries, each a short literal, lookup cost is dominated by hash
/// overhead either way. The exhaustive test in
/// `bouten_kind_from_suffix_recognises_all_spec_keywords` catches
/// typos before they silence recognition.
fn bouten_kind_from_suffix(s: &str) -> Option<BoutenKind> {
    Some(match s {
        "傍点" => BoutenKind::Goma,
        "白ゴマ傍点" => BoutenKind::WhiteSesame,
        "丸傍点" => BoutenKind::Circle,
        "白丸傍点" => BoutenKind::WhiteCircle,
        "二重丸傍点" => BoutenKind::DoubleCircle,
        "蛇の目傍点" => BoutenKind::Janome,
        "ばつ傍点" => BoutenKind::Cross,
        "白三角傍点" => BoutenKind::WhiteTriangle,
        "波線" => BoutenKind::WavyLine,
        "傍線" => BoutenKind::UnderLine,
        "二重傍線" => BoutenKind::DoubleUnderLine,
        _ => return None,
    })
}

/// Parse a leading run of ASCII / full-width decimal digits into a
/// [`u8`] and return the remainder slice.
///
/// Returns `None` if the leading char is not a digit, or if the value
/// overflows `u8` (> 255). `saturating_mul` / `saturating_add` during
/// accumulation keep the `u32` intermediate bounded, but the final
/// `try_from` enforces the `u8` range — a body like `300字下げ` fails
/// cleanly rather than wrapping to 44.
fn parse_decimal_u8_prefix(s: &str) -> Option<(u8, &str)> {
    let mut value: u32 = 0;
    let mut consumed = 0;
    for (idx, ch) in s.char_indices() {
        let digit = match ch {
            '0'..='9' => Some(u32::from(ch) - u32::from('0')),
            '０'..='９' => Some(u32::from(ch) - u32::from('０')),
            _ => None,
        };
        let Some(d) = digit else { break };
        value = value.saturating_mul(10).saturating_add(d);
        consumed = idx + ch.len_utf8();
    }
    if consumed == 0 {
        return None;
    }
    let value_u8 = u8::try_from(value).ok()?;
    Some((value_u8, &s[consumed..]))
}

/// Characters eligible as an implicit-ruby base. Covers:
///
/// * CJK Unified Ideographs (main block + Extension A)
/// * CJK Compatibility Ideographs
/// * CJK Unified Ideographs Extension B..F (supplementary plane)
/// * `々` (U+3005) ideographic iteration mark — usually kanji-like
/// * `〆` (U+3006) ideographic closing mark — sometimes used as kanji
const fn is_ruby_base_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFF}'
        | '々'
        | '〆'
    )
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

    /// Test-only helper: deref the boxed `Aozora` variant so tests can
    /// pattern-match on `AozoraNode` without spelling out the
    /// `Box<...>` indirection at every call site.
    fn aozora_node(span: &ClassifiedSpan) -> Option<&AozoraNode> {
        match &span.kind {
            SpanKind::Aozora(boxed) => Some(boxed.as_ref()),
            _ => None,
        }
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
    fn explicit_ruby_produces_single_aozora_span() {
        let src = "｜青梅《おうめ》";
        let out = run(src);
        assert_eq!(out.spans.len(), 1);
        let SpanKind::Aozora(ref boxed) = out.spans[0].kind else {
            panic!("expected Aozora span, got {:?}", out.spans[0].kind);
        };
        let AozoraNode::Ruby(ref ruby) = **boxed else {
            panic!("expected Ruby variant, got {boxed:?}");
        };
        assert_eq!(ruby.base.as_plain(), Some("青梅"));
        assert_eq!(ruby.reading.as_plain(), Some("おうめ"));
        assert!(ruby.delim_explicit);
        assert_eq!(out.spans[0].source_span.end as usize, src.len());
    }

    #[test]
    fn implicit_ruby_consumes_trailing_kanji_only() {
        // "あいう" (kana) + "漢字" (kanji) + ruby → base is "漢字",
        // leading kana stays Plain.
        let src = "あいう漢字《かんじ》";
        let out = run(src);
        assert_eq!(out.spans.len(), 2);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        let SpanKind::Aozora(ref boxed) = out.spans[1].kind else {
            panic!("expected Aozora span, got {:?}", out.spans[1].kind);
        };
        let AozoraNode::Ruby(ref ruby) = **boxed else {
            panic!("expected Ruby variant, got {boxed:?}");
        };
        assert_eq!(ruby.base.as_plain(), Some("漢字"));
        assert_eq!(ruby.reading.as_plain(), Some("かんじ"));
        assert!(!ruby.delim_explicit);
        // Plain covers "あいう"; ruby covers "漢字《かんじ》".
        assert_eq!(out.spans[0].source_span.slice(src), "あいう");
    }

    #[test]
    fn implicit_ruby_without_leading_kanji_leaves_ruby_unrecognized() {
        // No kanji before 《 → ruby can't bind. Ruby remains plain.
        let src = "あいう《かんじ》";
        let out = run(src);
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn explicit_ruby_with_empty_reading_is_not_recognized() {
        let src = "｜漢字《》";
        let out = run(src);
        // Empty reading fails recognition; whole source stays plain.
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn ruby_after_newline_keeps_newline_as_its_own_span() {
        let src = "line1\n｜漢《かん》";
        let out = run(src);
        // Plain("line1"), Newline, Aozora(Ruby)
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        let is_ruby = matches!(
            &out.spans[2].kind,
            SpanKind::Aozora(boxed) if matches!(**boxed, AozoraNode::Ruby(_))
        );
        assert!(is_ruby, "expected Aozora(Ruby), got {:?}", out.spans[2].kind);
    }

    #[test]
    fn implicit_ruby_after_non_text_event_is_not_recognized() {
        // A close-bracket between `」` and `《` means the preceding
        // event is PairClose, not Text. Implicit ruby can't bind.
        let src = "「台詞」《かんじ》";
        let out = run(src);
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    // ---------------------------------------------------------------
    // Ruby reading Content::Segments — nested gaiji / annotation
    // inside the `《reading》` body.
    // ---------------------------------------------------------------

    /// Pull the sole `SpanKind::Aozora(Ruby(...))` out of a
    /// [`ClassifyOutput`] so tests can assert on the Ruby payload
    /// without repeating the shape-match boilerplate.
    fn only_ruby(out: &ClassifyOutput) -> &Ruby {
        let mut found = None;
        for span in &out.spans {
            if let SpanKind::Aozora(ref boxed) = span.kind
                && let AozoraNode::Ruby(ref r) = **boxed
            {
                assert!(found.is_none(), "more than one Ruby span: {:?}", out.spans);
                found = Some(r);
            }
        }
        found.unwrap_or_else(|| panic!("no Ruby span in {:?}", out.spans))
    }

    #[test]
    fn ruby_plain_reading_still_collapses_to_plain_content() {
        // The Segments lift must not regress the plain-text ruby case:
        // when the body holds only text, `Content::from_segments` is
        // obliged to collapse back to `Content::Plain` so `.as_plain()`
        // returns `Some(&str)` for downstream consumers (renderer fast
        // path, property tests that assert the textual shape).
        let out = run("｜青梅《おうめ》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("青梅"));
        assert_eq!(r.reading.as_plain(), Some("おうめ"));
    }

    #[test]
    fn ruby_reading_with_embedded_gaiji_produces_segments() {
        // `※［＃「ほ」、第3水準1-85-54］` inside the reading must fold
        // into a `Segment::Gaiji` between Text segments so the renderer
        // can wrap it in `<span class="afm-gaiji">` without leaking the
        // bare `［＃` marker (Tier A).
        let out = run("｜日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("日本"));
        let Content::Segments(ref segs) = r.reading else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 3);
        assert!(
            matches!(&segs[0], Segment::Text(t) if &**t == "に"),
            "segment 0: {:?}",
            segs[0]
        );
        let Segment::Gaiji(ref g) = segs[1] else {
            panic!("segment 1 should be Gaiji, got {:?}", segs[1]);
        };
        assert_eq!(&*g.description, "ほ");
        assert_eq!(g.mencode.as_deref(), Some("第3水準1-85-54"));
        assert!(
            matches!(&segs[2], Segment::Text(t) if &**t == "ん"),
            "segment 2: {:?}",
            segs[2]
        );
    }

    #[test]
    fn ruby_reading_wholly_gaiji_produces_single_gaiji_segment() {
        // No surrounding text; the reading is exactly one gaiji
        // marker. The Segments run must be a single Gaiji (not a
        // trailing empty Text on either side).
        let out = run("｜日本《※［＃「にほん」、第3水準1-85-54］》");
        let r = only_ruby(&out);
        let Content::Segments(ref segs) = r.reading else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 1);
        let Segment::Gaiji(ref g) = segs[0] else {
            panic!("expected Gaiji, got {:?}", segs[0]);
        };
        assert_eq!(&*g.description, "にほん");
    }

    #[test]
    fn ruby_reading_with_trailing_annotation_produces_annotation_segment() {
        // `［＃ママ］` inside a reading indicates editorial "sic" —
        // must fold as `Segment::Annotation` so the renderer wraps it
        // in the hidden `afm-annotation` span (Tier A compliance).
        let out = run("｜日本《にほん［＃ママ］》");
        let r = only_ruby(&out);
        let Content::Segments(ref segs) = r.reading else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 2);
        assert!(
            matches!(&segs[0], Segment::Text(t) if &**t == "にほん"),
            "segment 0: {:?}",
            segs[0]
        );
        let Segment::Annotation(ref a) = segs[1] else {
            panic!("segment 1 should be Annotation, got {:?}", segs[1]);
        };
        assert_eq!(&*a.raw, "［＃ママ］");
    }

    #[test]
    fn ruby_reading_with_gaiji_and_annotation_interleaved() {
        // Exercises the general Segments shape: Text, Gaiji, Text,
        // Annotation. Proves the flusher preserves ordering and the
        // `text_start` advancement correctly spans each gap.
        let out = run("｜日本《に※［＃「ほ」、第3水準1-85-54］ん［＃ママ］》");
        let r = only_ruby(&out);
        let Content::Segments(ref segs) = r.reading else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 4);
        assert!(matches!(&segs[0], Segment::Text(t) if &**t == "に"));
        assert!(matches!(&segs[1], Segment::Gaiji(_)));
        assert!(matches!(&segs[2], Segment::Text(t) if &**t == "ん"));
        assert!(matches!(&segs[3], Segment::Annotation(_)));
    }

    #[test]
    fn implicit_ruby_reading_with_embedded_gaiji_also_produces_segments() {
        // Implicit form must use the same body walker; only the base
        // extraction differs (trailing-kanji run instead of explicit
        // `｜`-delimited Text event).
        let out = run("日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("日本"));
        assert!(!r.delim_explicit);
        let Content::Segments(ref segs) = r.reading else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 3);
        assert!(matches!(&segs[0], Segment::Text(t) if &**t == "に"));
        assert!(matches!(&segs[1], Segment::Gaiji(_)));
        assert!(matches!(&segs[2], Segment::Text(t) if &**t == "ん"));
    }

    #[test]
    fn ruby_reading_consume_span_still_covers_outer_source_bytes() {
        // The Segments lift must not disturb the outer `source_span`
        // of the classified span: Phase 4 still needs to replace the
        // full `｜…《…》` bytes with a single PUA sentinel, and the
        // inner gaiji/annotation source bytes are folded into the
        // Ruby payload — not re-exposed to the outer classifier.
        let src = "｜日本《に※［＃「ほ」、第3水準1-85-54］ん》";
        let out = run(src);
        let aozora_spans: Vec<_> = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .collect();
        assert_eq!(
            aozora_spans.len(),
            1,
            "nested gaiji must stay inside the Ruby payload, not leak into a \
             sibling span at the top level: {:?}",
            out.spans
        );
        assert_eq!(
            aozora_spans[0].source_span.end as usize,
            src.len(),
            "ruby span must cover through the final `》`"
        );
        assert_eq!(aozora_spans[0].source_span.start, 0);
    }

    #[test]
    fn ruby_reading_preserves_tier_a_even_for_nested_block_leaf() {
        // `［＃改ページ］` inside a ruby reading is nonsensical, but
        // real corpora have been known to carry freak shapes. The
        // non-Annotation emit path in `build_content_from_body` must
        // downgrade such shapes into `Annotation{Unknown}` so the
        // bare `［＃` never reaches the rendered HTML through a
        // `Segment::Text` channel (Tier A canary).
        let out = run("｜日本《にほん［＃改ページ］》");
        let r = only_ruby(&out);
        let Content::Segments(ref segs) = r.reading else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        // Last segment must be an Annotation carrying the raw bytes.
        let last = segs.last().expect("non-empty segments");
        let Segment::Annotation(a) = last else {
            panic!("final segment should be Annotation, got {last:?}");
        };
        assert_eq!(&*a.raw, "［＃改ページ］");
        assert_eq!(a.kind, AnnotationKind::Unknown);
    }

    #[test]
    fn page_break_annotation_becomes_single_page_break_span() {
        let src = "前\n［＃改ページ］\n後";
        let out = run(src);
        // Plain("前"), Newline, Aozora(PageBreak), Newline, Plain("後")
        assert_eq!(out.spans.len(), 5);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert!(matches!(
            aozora_node(&out.spans[2]),
            Some(AozoraNode::PageBreak)
        ));
        assert_eq!(out.spans[2].source_span.slice(src), "［＃改ページ］");
        assert_eq!(out.spans[3].kind, SpanKind::Newline);
        assert_eq!(out.spans[4].kind, SpanKind::Plain);
    }

    #[test]
    fn section_break_choho_recognized() {
        let out = run("［＃改丁］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::SectionBreak(SectionKind::Choho))
        ));
    }

    #[test]
    fn section_break_dan_recognized() {
        let out = run("［＃改段］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::SectionBreak(SectionKind::Dan))
        ));
    }

    #[test]
    fn section_break_spread_recognized() {
        let out = run("［＃改見開き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::SectionBreak(SectionKind::Spread))
        ));
    }

    #[test]
    fn bracket_without_hash_is_not_an_annotation() {
        // `［普通］` (no `＃`) is plain literal text, not an annotation.
        let out = run("［普通］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn unknown_annotation_keyword_is_promoted_to_annotation_unknown() {
        // The lexer claims every well-formed `［＃…］`: if no specialised
        // recogniser matches, the `Annotation{Unknown}` fallback wraps
        // the raw source so the renderer can emit an `afm-annotation`
        // hidden span instead of leaking the brackets as plain text.
        let out = run("［＃未知のキーワード］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("unknown keyword must promote to Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(&*ann.raw, "［＃未知のキーワード］");
    }

    #[test]
    fn annotation_with_whitespace_padding_still_matches() {
        // Corpus occasionally has `［＃ 改ページ ］` with spaces. We
        // trim the body to be lenient.
        let out = run("［＃ 改ページ ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::PageBreak)
        ));
    }

    #[test]
    fn empty_bracket_with_hash_is_wrapped_as_annotation_unknown() {
        // Real Aozora corpora use `［＃］` as an illustrative glyph
        // inside explanatory prose (e.g. "［＃］：入力者注…"). The
        // Tier-A canary (no bare `［＃` in HTML output) requires that
        // the bracket not leak even for empty-body forms, so the
        // catch-all fallback wraps it as Annotation{Unknown} with the
        // raw `［＃］` bytes preserved for round-trip.
        let out = run("［＃］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("empty body must still wrap as Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(&*ann.raw, "［＃］");
    }

    #[test]
    fn indent_with_full_width_digit() {
        let out = run("［＃２字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::Indent(Indent { amount: 2 }))
        ));
    }

    #[test]
    fn indent_with_ascii_digit() {
        let out = run("［＃10字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::Indent(Indent { amount: 10 }))
        ));
    }

    #[test]
    fn indent_overflow_falls_back_to_annotation_unknown() {
        // 300 > 255, doesn't fit in u8 — the `N字下げ` recogniser
        // declines. The `Annotation { Unknown }` catch-all then
        // claims the bracket so the renderer wraps the body in an
        // afm-annotation span instead of leaking raw brackets.
        let out = run("［＃300字下げ］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("overflow should fall back to Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(&*ann.raw, "［＃300字下げ］");
        // The specialised Indent recogniser MUST NOT claim it.
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_digit_falls_through() {
        // N=0 is meaningless for 字下げ (a zero-width indent is not
        // a thing). Fullwidth-digit variant.
        let out = run("［＃０字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_ascii_digit_falls_through() {
        // ASCII-digit variant of the N=0 reject.
        let out = run("［＃0字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn align_end_zero_digit_falls_through() {
        // 地から0字上げ is redundant with 地付き and not spec-sanctioned —
        // reject so the text falls through to a generic Annotation.
        let out = run("［＃地から0字上げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::AlignEnd(_)))),
        );
    }

    #[test]
    fn chitsuki_zero_offset_recognized() {
        let out = run("［＃地付き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::AlignEnd(AlignEnd { offset: 0 }))
        ));
    }

    #[test]
    fn chi_kara_n_ji_age_recognized() {
        let out = run("［＃地から３字上げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::AlignEnd(AlignEnd { offset: 3 }))
        ));
    }

    #[test]
    fn indent_without_digits_falls_through() {
        // "ここから字下げ" is a paired-container opener, not a leaf
        // indent — the leaf classifier must not grab it, and the
        // paired-container recogniser claims it instead.
        let out = run("［＃ここから字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn forward_bouten_goma_recognized() {
        // Preceding text "前置き" plus "青空" before the bracket — the
        // target literal must appear in the preceding source for the
        // forward-reference classifier to promote.
        let out = run("前置きの青空［＃「青空」に傍点］後ろ");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.kind, BoutenKind::Goma);
        assert_eq!(bouten.target.as_plain(), Some("青空"));
    }

    #[test]
    fn forward_bouten_circle_recognized() {
        let out = run("X［＃「X」に丸傍点］");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.kind, BoutenKind::Circle);
        assert_eq!(bouten.target.as_plain(), Some("X"));
    }

    #[test]
    fn forward_bouten_all_eleven_kinds() {
        // All eleven bouten kinds — the seven core shapes plus
        // 白ゴマ / ばつ / 白三角 / 二重傍線. Each suffix must promote
        // the bracket into a `Bouten` node rather than fall through
        // to `Annotation{Unknown}`, lowering the sweep leak rate.
        let cases = [
            ("傍点", BoutenKind::Goma),
            ("白ゴマ傍点", BoutenKind::WhiteSesame),
            ("丸傍点", BoutenKind::Circle),
            ("白丸傍点", BoutenKind::WhiteCircle),
            ("二重丸傍点", BoutenKind::DoubleCircle),
            ("蛇の目傍点", BoutenKind::Janome),
            ("ばつ傍点", BoutenKind::Cross),
            ("白三角傍点", BoutenKind::WhiteTriangle),
            ("波線", BoutenKind::WavyLine),
            ("傍線", BoutenKind::UnderLine),
            ("二重傍線", BoutenKind::DoubleUnderLine),
        ];
        for (suffix, expected_kind) in cases {
            let src = format!("t［＃「t」に{suffix}］");
            let out = run(&src);
            let Some(b) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            }) else {
                panic!("no Bouten span for suffix {suffix:?}");
            };
            assert_eq!(b.kind, expected_kind, "suffix {suffix:?}");
            // All default `に` shapes produce right-side position.
            assert_eq!(b.position, BoutenPosition::Right, "suffix {suffix:?}");
        }
    }

    #[test]
    fn forward_bouten_left_side_flips_position() {
        // `の左に傍点` sets BoutenPosition::Left. The same forward-
        // reference validation (target appears in preceding text) still
        // applies so we prepend a matching target.
        let out = run("X［＃「X」の左に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.kind, BoutenKind::Goma);
        assert_eq!(b.position, BoutenPosition::Left);
        assert_eq!(b.target.as_plain(), Some("X"));
    }

    #[test]
    fn forward_bouten_left_side_pairs_with_every_kind() {
        // 左 + every kind must work (same suffix grammar).
        let cases = [
            ("傍点", BoutenKind::Goma),
            ("白ゴマ傍点", BoutenKind::WhiteSesame),
            ("丸傍点", BoutenKind::Circle),
            ("二重傍線", BoutenKind::DoubleUnderLine),
            ("傍線", BoutenKind::UnderLine),
        ];
        for (suffix, expected_kind) in cases {
            let src = format!("t［＃「t」の左に{suffix}］");
            let out = run(&src);
            let Some(b) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            }) else {
                panic!("no Bouten span for left-side suffix {suffix:?}");
            };
            assert_eq!(b.kind, expected_kind);
            assert_eq!(b.position, BoutenPosition::Left);
        }
    }

    #[test]
    fn forward_bouten_multi_quote_concatenates_targets() {
        // `［＃「A」「B」に傍点］` walks consecutive PairOpen(Quote)
        // events after the `＃` and folds their bodies into a single
        // Bouten target joined with `、`. Both A and B must appear in
        // the preceding text for the classifier to promote — this
        // keeps the forward-reference semantic intact.
        let out = run("AとB［＃「A」「B」に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("multi-quote Bouten expected");
        assert_eq!(b.kind, BoutenKind::Goma);
        // Targets collapse to `A、B` through `Content::from_segments`
        // (all-Text segments → `Plain`).
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_multi_quote_without_all_targets_preceded_falls_through() {
        // Only "A" appears before the bracket; "B" does not. The
        // classifier refuses to promote — the bracket is consumed as
        // `Annotation{Unknown}` by the catch-all instead, preserving
        // Tier-A without inventing a bouten target.
        let out = run("A［＃「A」「B」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
            "Bouten must not promote when any target is unreferenced"
        );
    }

    #[test]
    fn forward_bouten_empty_inner_quotes_are_skipped() {
        // `「」` placeholders in the middle of a multi-quote body do
        // not contribute to the target list. This guards against
        // corpus stragglers like `［＃「A」「」「B」に傍点］`.
        let out = run("AB［＃「A」「」「B」に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_position_slug_and_segments_render_together() {
        // Regression: the position modifier must be propagated even
        // when the target is a Segments (multi-quote) value.
        let out = run("AB［＃「A」「B」の左に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.position, BoutenPosition::Left);
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_empty_target_falls_through() {
        let out = run("［＃「」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_unknown_suffix_falls_through() {
        let out = run("［＃「X」に未知］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_missing_ni_particle_falls_through() {
        let out = run("［＃「X」傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_without_preceding_target_falls_through() {
        // Target 可哀想 never appears before the bracket — refusing to
        // promote to Bouten lets the generic Annotation classifier
        // wrap the raw `［＃…］` in an afm-annotation span instead of
        // styling a non-existent referent.
        let out = run("［＃「可哀想」に傍点］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_target_in_preceding_paragraph_still_promotes() {
        // The classifier currently scans the entire preceding source
        // (not just the current paragraph). Preserving that lenient
        // behaviour keeps real Aozora corpora working — authors
        // sometimes refer backwards across paragraph boundaries.
        let out = run("青空\n\n改行後［＃「青空」に傍点］");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_tcy_without_preceding_target_falls_through() {
        let out = run("［＃「29」は縦中横］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::TateChuYoko(_)))),
        );
    }

    #[test]
    fn forward_bouten_with_nested_quote_in_target_uses_outer_quote() {
        // Phase 2 balances 「「」」 correctly. The target is the full
        // outer-quote contents including the inner 「inner」 — not
        // truncated at the first 」. The preceding copy of the target
        // is required so the classifier's target-exists check passes.
        let out = run("A「inner」B［＃「A「inner」B」に傍点］");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.target.as_plain(), Some("A「inner」B"));
    }

    #[test]
    fn forward_tcy_single_recognized() {
        let out = run("20［＃「20」は縦中横］");
        let tcy = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::TateChuYoko(t)) => Some(t),
                _ => None,
            })
            .expect("expected a TateChuYoko span");
        assert_eq!(tcy.text.as_plain(), Some("20"));
    }

    #[test]
    fn forward_tcy_wrong_particle_falls_through() {
        // Using に instead of は — not a TCY shape.
        let out = run("［＃「20」に縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::TateChuYoko(_)))),
        );
    }

    #[test]
    fn forward_tcy_empty_target_falls_through() {
        let out = run("［＃「」は縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::TateChuYoko(_)))),
        );
    }

    // ---------------------------------------------------------------
    // Forward-reference heading hints — `［＃「X」は(大|中|小)見出し］`.
    // These tests pin the lexer contract that drives post-process
    // paragraph promotion (docs/plan.md §M2): the classifier emits a
    // `HeadingHint { level: 1..=3 }` when the target is preceded by a
    // matching run in the source, otherwise falls through so the
    // catch-all emits `Annotation { Unknown }` and the Tier-A canary
    // ([# never leaks) still holds.
    // ---------------------------------------------------------------

    fn find_heading_hint(out: &ClassifyOutput) -> Option<HeadingHint> {
        out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(AozoraNode::HeadingHint(h)) => Some(h.clone()),
            _ => None,
        })
    }

    #[test]
    fn forward_heading_large_recognized() {
        // Spec: 大見出し → Markdown H1 (level 1). The preceding
        // occurrence of the target literal is required — same gate as
        // forward-bouten.
        let out = run("第一篇［＃「第一篇」は大見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 1);
        assert_eq!(&*h.target, "第一篇");
    }

    #[test]
    fn forward_heading_medium_recognized() {
        // 中見出し → H2.
        let out = run("一［＃「一」は中見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 2);
        assert_eq!(&*h.target, "一");
    }

    #[test]
    fn forward_heading_small_recognized() {
        // 小見出し → H3.
        let out = run("小題［＃「小題」は小見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 3);
        assert_eq!(&*h.target, "小題");
    }

    #[test]
    fn forward_heading_without_preceding_target_falls_through() {
        // No 「第一篇」 run in the preceding source — hint has no
        // referent; classifier must reject so the paragraph isn't
        // promoted to an empty heading. The catch-all then emits
        // `Annotation { Unknown }` to preserve Tier-A.
        let out = run("［＃「第一篇」は大見出し］後");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_unknown_keyword_falls_through() {
        // `大見出し` and friends are the only supported heading
        // keywords; anything else (包括的, 飾り見出し, …) should not
        // promote.
        let out = run("X［＃「X」は飾り見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_wrong_particle_falls_through() {
        // The Aozora annotation spec's heading shape uses `は` as the
        // particle. Using `に` (the bouten particle) must not promote
        // to HeadingHint — otherwise we'd clobber the bouten path.
        let out = run("X［＃「X」に大見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_empty_target_falls_through() {
        let out = run("［＃「」は大見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_all_three_levels_exercised_in_one_paragraph() {
        // A single paragraph could conceivably carry multiple heading
        // hints — the lexer emits one HeadingHint per bracket and
        // post-process handles the first. This test locks the per-
        // bracket classification rather than the post_process policy.
        let out = run("A［＃「A」は大見出し］B［＃「B」は中見出し］C［＃「C」は小見出し］");
        let levels: Vec<u8> = out
            .spans
            .iter()
            .filter_map(|s| match aozora_node(s) {
                Some(AozoraNode::HeadingHint(h)) => Some(h.level),
                _ => None,
            })
            .collect();
        assert_eq!(levels, vec![1, 2, 3]);
    }

    #[test]
    fn sashie_without_caption_recognized() {
        let out = run("［＃挿絵（fig01.png）入る］");
        let sashie = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Sashie(s)) => Some(s),
                _ => None,
            })
            .expect("expected a Sashie span");
        assert_eq!(&*sashie.file, "fig01.png");
        assert!(sashie.caption.is_none());
    }

    #[test]
    fn sashie_with_caption_form_not_matched() {
        // Captioned sashie needs a dedicated caption recogniser;
        // the no-caption matcher must reject the captioned form
        // cleanly so the bracket falls through to the catch-all.
        let out = run("［＃挿絵（fig01.png）「キャプション」入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn sashie_empty_filename_falls_through() {
        let out = run("［＃挿絵（）入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn sashie_missing_iru_suffix_falls_through() {
        let out = run("［＃挿絵（fig01.png）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn gaiji_quoted_description_with_mencode() {
        let out = run("※［＃「木＋吶のつくり」、第3水準1-85-54］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(&*gaiji.description, "木＋吶のつくり");
        assert_eq!(gaiji.mencode.as_deref(), Some("第3水準1-85-54"));
        // The mencode table resolves 第3水準1-85-54 → 榁 (U+6903).
        assert_eq!(gaiji.ucs, Some('\u{6903}'));
    }

    #[test]
    fn gaiji_quoted_description_without_mencode() {
        let out = run("※［＃「試」］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(&*gaiji.description, "試");
        assert!(gaiji.mencode.is_none());
    }

    #[test]
    fn gaiji_bare_description_with_mencode() {
        let out = run("※［＃二の字点、1-2-23］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(&*gaiji.description, "二の字点");
        assert_eq!(gaiji.mencode.as_deref(), Some("1-2-23"));
    }

    #[test]
    fn gaiji_consumes_refmark_and_bracket_as_one_span() {
        let src = "a※［＃「X」、m］b";
        let out = run(src);
        let gaiji_span = out
            .spans
            .iter()
            .find(|s| matches!(aozora_node(s), Some(AozoraNode::Gaiji(_))))
            .expect("expected a Gaiji span");
        // span must start at the ※ (after "a"), not at ［.
        assert_eq!(gaiji_span.source_span.slice(src), "※［＃「X」、m］");
    }

    #[test]
    fn refmark_without_following_bracket_stays_plain() {
        // Bare ※ without ［＃...］ — not a gaiji, emit as Plain.
        let out = run("a※b");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Gaiji(_)))),
        );
    }

    #[test]
    fn gaiji_without_hash_is_not_recognized() {
        // ※ followed by ［ but no ＃ inside — not a gaiji shape.
        let out = run("※［普通］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Gaiji(_)))),
        );
    }

    #[test]
    fn kaeriten_ichi_recognized() {
        let out = run("之［＃一］");
        let kaeriten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Kaeriten(k)) => Some(k),
                _ => None,
            })
            .expect("expected a Kaeriten span");
        assert_eq!(&*kaeriten.mark, "一");
    }

    #[test]
    fn kaeriten_all_twelve_marks_recognized() {
        for mark in [
            "一", "二", "三", "四", "上", "中", "下", "レ", "甲", "乙", "丙", "丁",
        ] {
            let src = format!("［＃{mark}］");
            let out = run(&src);
            let Some(k) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Kaeriten(k)) => Some(k),
                _ => None,
            }) else {
                panic!("no Kaeriten span for mark {mark:?}");
            };
            assert_eq!(&*k.mark, mark);
        }
    }

    #[test]
    fn kaeriten_unknown_mark_falls_through() {
        let out = run("［＃甬］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
        );
    }

    #[test]
    fn kaeriten_compound_marks_recognized() {
        // Compound kaeriten pair an order mark with the reversal mark
        // (`レ`). Six combinations are canonical per the Aozora
        // kunten spec. Each must produce a Kaeriten with the combo
        // string preserved verbatim.
        let cases = ["一レ", "二レ", "三レ", "上レ", "中レ", "下レ"];
        for mark in cases {
            let src = format!("［＃{mark}］");
            let out = run(&src);
            let k = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(AozoraNode::Kaeriten(k)) => Some(k),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no Kaeriten span for mark {mark:?}"));
            assert_eq!(&*k.mark, mark, "mark={mark:?}");
        }
    }

    #[test]
    fn kaeriten_okurigana_shape_recognized() {
        // `［＃（X）］` where X is 1–6 Japanese chars is treated as an
        // okurigana marker — same AozoraNode::Kaeriten with the
        // parenthesised payload kept verbatim for the renderer.
        let cases = [
            "（カ）",
            "（ダ）",
            "（シクシテ）",
            "（弖）",       // kanji payload
            "（テニヲハ）", // 4-char katakana
        ];
        for mark in cases {
            let src = format!("［＃{mark}］");
            let out = run(&src);
            let k = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(AozoraNode::Kaeriten(k)) => Some(k),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no Kaeriten for okurigana {mark:?}"));
            assert_eq!(&*k.mark, mark, "mark={mark:?}");
        }
    }

    #[test]
    fn kaeriten_okurigana_with_long_body_falls_through() {
        // 7+ character parenthesised content is almost always an
        // editorial gloss, not okurigana. Must fall through to
        // Annotation{Unknown} so we don't mislabel it as kaeriten.
        let out = run("［＃（これはおくりがなではない）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
            "long parenthesised bodies must not be Kaeriten: {:?}",
            out.spans
        );
    }

    #[test]
    fn kaeriten_okurigana_with_latin_body_falls_through() {
        // Okurigana payload must be hiragana/katakana/kanji. ASCII
        // inside parens is probably an editorial note, not kaeriten.
        let out = run("［＃（abc）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
        );
    }

    #[test]
    fn kaeriten_okurigana_empty_parens_fall_through() {
        let out = run("［＃（）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
        );
    }

    // ---------------------------------------------------------------
    // Double angle-bracket `《《X》》`.
    // ---------------------------------------------------------------

    #[test]
    fn double_ruby_plain_body_produces_double_ruby_span() {
        let out = run("前《《強調》》後");
        let aozora = out
            .spans
            .iter()
            .find_map(aozora_node)
            .expect("DoubleRuby expected");
        let AozoraNode::DoubleRuby(d) = aozora else {
            panic!("expected DoubleRuby, got {aozora:?}");
        };
        assert_eq!(d.content.as_plain(), Some("強調"));
    }

    #[test]
    fn double_ruby_consumes_entire_source_span() {
        // Source `《《X》》` must fold into ONE Aozora span that covers
        // the double brackets AND the body. No `《` characters may
        // leak to the outer `spans` list.
        let src = "《《ABC》》";
        let out = run(src);
        let aozora_count = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .count();
        assert_eq!(
            aozora_count, 1,
            "one DoubleRuby span expected: {:?}",
            out.spans
        );
        let aozora = out
            .spans
            .iter()
            .find(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .expect("Aozora span");
        assert_eq!(aozora.source_span.start, 0);
        assert_eq!(aozora.source_span.end as usize, src.len());
    }

    #[test]
    fn double_ruby_with_nested_gaiji_folds_into_segments() {
        // The helper reuses `build_content_from_body`, so a `※［＃…］`
        // inside the double brackets must surface as `Segment::Gaiji`
        // in the content — same invariant as nested gaiji in ruby.
        let out = run("《《※［＃「ほ」、第3水準1-85-54］》》");
        let aozora = out
            .spans
            .iter()
            .find_map(aozora_node)
            .expect("Aozora expected");
        let AozoraNode::DoubleRuby(d) = aozora else {
            panic!("expected DoubleRuby, got {aozora:?}");
        };
        let Content::Segments(segs) = &d.content else {
            panic!("expected Segments, got {:?}", d.content);
        };
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], Segment::Gaiji(_)));
    }

    #[test]
    fn double_ruby_empty_body_still_consumed() {
        // `《《》》` with no body: we still consume the double brackets
        // into a DoubleRuby span so no stray `《` leaks as plain text.
        // The content is empty `Content::Segments([])`.
        let out = run("A《《》》B");
        let aozora_count = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .count();
        assert_eq!(
            aozora_count, 1,
            "empty double-ruby must still emit one span"
        );
    }

    #[test]
    fn container_open_indent_default_amount_one() {
        let out = run("［＃ここから字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent { amount: 1 })
        ));
    }

    #[test]
    fn container_open_indent_with_amount() {
        let out = run("［＃ここから３字下げ］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent { amount: 3 })
        ));
    }

    #[test]
    fn container_close_indent_matches_open_by_variant() {
        let out = run("［＃ここから字下げ］本文［＃ここで字下げ終わり］");
        // Spans: BlockOpen(Indent{1}), Plain("本文"), BlockClose(Indent{0})
        assert_eq!(out.spans.len(), 3);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent { .. })
        ));
        assert_eq!(out.spans[1].kind, SpanKind::Plain);
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::Indent { .. })
        ));
    }

    #[test]
    fn container_open_chitsuki_and_chi_kara_n() {
        let out = run("［＃ここから地付き］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::AlignEnd { offset: 0 })
        ));
        let out2 = run("［＃ここから地から2字上げ］");
        assert!(matches!(
            out2.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::AlignEnd { offset: 2 })
        ));
    }

    #[test]
    fn container_open_close_keigakomi() {
        let out = run("［＃罫囲み］内部［＃罫囲み終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Keigakomi)
        ));
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::Keigakomi)
        ));
    }

    #[test]
    fn warichu_open_close_are_inline_annotations() {
        // Aozora spec: `［＃割り注］…［＃割り注終わり］` is inline
        // (`<span class="afm-warichu">…</span>`). The legacy block
        // form (`ここから割り注` / `ここで割り注終わり`) is deprecated
        // and not classified here.
        use aozora_syntax::AnnotationKind;
        let out = run("［＃割り注］内部［＃割り注終わり］");
        let Some(AozoraNode::Annotation(open)) = aozora_node(&out.spans[0]) else {
            panic!(
                "expected Aozora(Annotation) for ［＃割り注］, got {:?}",
                out.spans[0].kind,
            );
        };
        assert_eq!(open.kind, AnnotationKind::WarichuOpen);
        assert_eq!(&*open.raw, "［＃割り注］");

        let Some(AozoraNode::Annotation(close)) = aozora_node(&out.spans[2]) else {
            panic!(
                "expected Aozora(Annotation) for ［＃割り注終わり］, got {:?}",
                out.spans[2].kind,
            );
        };
        assert_eq!(close.kind, AnnotationKind::WarichuClose);
        assert_eq!(&*close.raw, "［＃割り注終わり］");
    }

    #[test]
    fn container_close_without_matching_open_still_emits_close() {
        // Phase 3 does not pair opens with closes — that's `post_process`.
        // A bare `［＃罫囲み終わり］` is still classified.
        let out = run("［＃罫囲み終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockClose(ContainerKind::Keigakomi)
        ));
    }

    #[test]
    fn container_unknown_here_from_keyword_falls_through() {
        let out = run("［＃ここから未知］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::BlockOpen(_) | SpanKind::BlockClose(_))),
            "expected no block container spans, got {:?}",
            out.spans
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
