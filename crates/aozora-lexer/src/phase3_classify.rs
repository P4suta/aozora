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
//! Recognizers land incrementally on top of the same driver:
//!
//! * ✅ C4a — scaffolding (Plain + Newline only).
//! * ✅ C4b — ruby (explicit `｜base《reading》` and implicit-kanji).
//! * C4c — bracket-annotation keyword dispatch (leaf blocks, bouten
//!   keyword table).
//! * C4d — inline: forward-ref bouten, tcy, gaiji, kaeriten.
//! * C4e — paired containers (字下げ / 地付き / 罫囲み / 割り注 /
//!   小書き / 大中小見出し).
//!
//! Each recognizer is a narrow function that inspects a
//! `&[PairEvent]` slice (often one pair's `body_events`) plus the
//! sanitized source. The driver loop stays the same — only the
//! `try_recognize` dispatch grows.

use afm_syntax::{
    AlignEnd, Annotation, AnnotationKind, AozoraNode, Bouten, BoutenKind, ContainerKind, Content,
    Gaiji, Indent, Kaeriten, Ruby, Sashie, SectionKind, Span, TateChuYoko,
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
    /// position.
    Aozora(AozoraNode),
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
            kind: SpanKind::Aozora(AozoraNode::Ruby(Ruby {
                base: Content::from(m.base),
                reading: Content::from(m.reading),
                delim_explicit: m.explicit,
            })),
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
            EmitKind::Aozora(node) => SpanKind::Aozora(node),
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
            kind: SpanKind::Aozora(m.node),
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

/// Intermediate result of [`recognize_ruby`]. Strings are borrowed
/// from the sanitized source; the driver owns them into the final
/// `Content` values.
struct RubyMatch<'s> {
    base: &'s str,
    reading: &'s str,
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
    let reading = &source[open_span.end as usize..close_span.start as usize];
    if reading.is_empty() {
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
/// The UCS resolution column of [`Gaiji`] is left `None` here — G1
/// (the gaiji UCS table) fills it in post-classification.
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

    Some(GaijiMatch {
        node: AozoraNode::Gaiji(Gaiji {
            description: description.into_boxed_str(),
            ucs: None,
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
/// hash whose keyword is unrecognized also fall through to Plain
/// (they will be picked up by later C4c/d/e commits or emitted as
/// `Annotation { Unknown }` once F4/F5 land).
///
/// C4c1 recognizes the four no-body block leaf keywords:
/// `改ページ` / `改丁` / `改段` / `改見開き`.
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
        .or_else(|| {
            classify_forward_bouten(events, source, open_idx, close_idx).map(EmitKind::Aozora)
        })
        .or_else(|| classify_forward_tcy(events, source, open_idx, close_idx).map(EmitKind::Aozora))
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
            //
            // Pre-D1 this was the adapter's inline parse hook; post-D1
            // the lexer is the sole owner of this classification.
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
    let (target, suffix) = extract_forward_quote_target(events, source, open_idx, close_idx)?;
    let suffix = suffix.strip_prefix("に")?;
    let kind = bouten_kind_from_suffix(suffix)?;
    // A forward-reference bouten only makes sense when its target
    // literal actually appears in the preceding text. Otherwise it has
    // no referent and we fall through to the generic Annotation path
    // so the reader sees the raw `［＃…］` rather than a mysterious
    // styling applied to nothing.
    if !forward_target_is_preceded(events, source, open_idx, target) {
        return None;
    }
    Some(AozoraNode::Bouten(Bouten {
        kind,
        target: Content::from(target),
    }))
}

/// Classify a `［＃「target」は縦中横］` forward-reference
/// tate-chu-yoko (horizontal-in-vertical) annotation.
///
/// Same event-layout expectations as forward bouten, except the
/// suffix uses the particle `は` and the keyword `縦中横`. Paired
/// form (`［＃縦中横］…［＃縦中横終わり］`) is a C4e / C4d concern
/// and is not matched here.
fn classify_forward_tcy(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    close_idx: usize,
) -> Option<AozoraNode> {
    let (target, suffix) = extract_forward_quote_target(events, source, open_idx, close_idx)?;
    if suffix != "は縦中横" {
        return None;
    }
    // Same rationale as `classify_forward_bouten` — the styling has no
    // meaning without a preceding target literal.
    if !forward_target_is_preceded(events, source, open_idx, target) {
        return None;
    }
    Some(AozoraNode::TateChuYoko(TateChuYoko {
        text: Content::from(target),
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

/// Shared helper for the `［＃「X」<particle><keyword>］` shape.
///
/// Returns `(target, suffix)` where:
/// * `target` is the raw source bytes strictly inside the `「…」`
///   quote pair that lives immediately after the `＃`.
/// * `suffix` is the trimmed source bytes between the closing `」`
///   and the bracket's `］`. Callers then match on the particle +
///   keyword.
///
/// Returns `None` if any shape assumption fails: no adjacent quote
/// pair, empty target, or quote crossing out of the bracket.
fn extract_forward_quote_target<'s>(
    events: &[PairEvent],
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<(&'s str, &'s str)> {
    let quote_open_idx = open_idx + 2;
    let &PairEvent::PairOpen {
        kind: PairKind::Quote,
        span: quote_open_span,
        close_idx: quote_close_idx,
    } = events.get(quote_open_idx)?
    else {
        return None;
    };
    // The quote must close *before* the bracket — a cross-boundary
    // close would mean the quote is not nested inside the bracket.
    if quote_close_idx >= close_idx {
        return None;
    }
    let &PairEvent::PairClose {
        span: quote_close_span,
        ..
    } = events.get(quote_close_idx)?
    else {
        return None;
    };
    let target = &source[quote_open_span.end as usize..quote_close_span.start as usize];
    if target.is_empty() {
        return None;
    }
    let &PairEvent::PairClose {
        span: bracket_close_span,
        ..
    } = events.get(close_idx)?
    else {
        return None;
    };
    let suffix = source[quote_close_span.end as usize..bracket_close_span.start as usize].trim();
    Some((target, suffix))
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
/// * `割り注`               → `Warichu`
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
        "割り注" => Some(EmitKind::BlockOpen(ContainerKind::Warichu)),
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
/// * `割り注終わり`         → `Warichu`
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
        "割り注" => Some(EmitKind::BlockClose(ContainerKind::Warichu)),
        _ => None,
    }
}

/// Classify a `［＃<mark>］` kaeriten (Chinese-reading order mark).
///
/// The body must be exactly one of the 12 canonical marks:
/// 一 / 二 / 三 / 四 / 上 / 中 / 下 / レ / 甲 / 乙 / 丙 / 丁. Other
/// single-character annotation bodies are left for other classifiers
/// or fall through to Plain.
fn classify_kaeriten(body: &str) -> Option<AozoraNode> {
    const MARKS: &[&str] = &[
        "一", "二", "三", "四", "上", "中", "下", "レ", "甲", "乙", "丙", "丁",
    ];
    if MARKS.contains(&body) {
        return Some(AozoraNode::Kaeriten(Kaeriten { mark: body.into() }));
    }
    None
}

/// Classify a `［＃挿絵（file）入る］` sashie (illustration insert).
///
/// Captures the filename between `（` and `）`; the rest of the body
/// must be exactly `入る`. Captioned form
/// (`［＃挿絵（file）「caption」入る］`) is left to F5 where the
/// extra quote pair gets event-level handling — the simple no-caption
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

/// Map the trailing keyword (after `に`) to a [`BoutenKind`].
///
/// Covers the seven shapes catalogued at
/// <https://www.aozora.gr.jp/annotation/bouten.html>. Unknown
/// suffixes return `None`, letting the annotation fall through to
/// Plain (or to a more specific classifier in a later commit).
fn bouten_kind_from_suffix(s: &str) -> Option<BoutenKind> {
    Some(match s {
        "傍点" => BoutenKind::Goma,
        "丸傍点" => BoutenKind::Circle,
        "白丸傍点" => BoutenKind::WhiteCircle,
        "二重丸傍点" => BoutenKind::DoubleCircle,
        "蛇の目傍点" => BoutenKind::Janome,
        "波線" => BoutenKind::WavyLine,
        "傍線" => BoutenKind::UnderLine,
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

/// Characters eligible as an implicit-ruby base.
///
/// Mirrors `afm-parser::aozora::ruby::is_ruby_base_char` so the
/// corpus behavior stays consistent across the E2 cutover. Covers:
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
        let SpanKind::Aozora(AozoraNode::Ruby(ref ruby)) = out.spans[0].kind else {
            panic!("expected Aozora(Ruby) span, got {:?}", out.spans[0].kind);
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
        let SpanKind::Aozora(AozoraNode::Ruby(ref ruby)) = out.spans[1].kind else {
            panic!("expected Aozora(Ruby) span, got {:?}", out.spans[1].kind);
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
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::Aozora(AozoraNode::Ruby(_))
        ));
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

    #[test]
    fn page_break_annotation_becomes_single_page_break_span() {
        let src = "前\n［＃改ページ］\n後";
        let out = run(src);
        // Plain("前"), Newline, Aozora(PageBreak), Newline, Plain("後")
        assert_eq!(out.spans.len(), 5);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::Aozora(AozoraNode::PageBreak)
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
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::SectionBreak(SectionKind::Choho))
        ));
    }

    #[test]
    fn section_break_dan_recognized() {
        let out = run("［＃改段］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::SectionBreak(SectionKind::Dan))
        ));
    }

    #[test]
    fn section_break_spread_recognized() {
        let out = run("［＃改見開き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::SectionBreak(SectionKind::Spread))
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
        // Post-D1 the lexer classifies every well-formed `［＃…］` with a
        // non-empty body: if no specialised recogniser claims it, the
        // Annotation{Unknown} fallback wraps the raw source so the
        // renderer can emit an `afm-annotation` hidden span instead of
        // leaking the brackets as plain text.
        let out = run("［＃未知のキーワード］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Annotation(a)) => Some(a),
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
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::PageBreak)
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
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Annotation(a)) => Some(a),
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
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::Indent(Indent { amount: 2 }))
        ));
    }

    #[test]
    fn indent_with_ascii_digit() {
        let out = run("［＃10字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::Indent(Indent { amount: 10 }))
        ));
    }

    #[test]
    fn indent_overflow_falls_back_to_annotation_unknown() {
        // 300 > 255, doesn't fit in u8 — the `N字下げ` recogniser
        // declines. After the D1 fallback we don't leak the raw
        // bracket; instead it becomes `Annotation { Unknown }` so
        // the renderer wraps the body in an afm-annotation span.
        let out = run("［＃300字下げ］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("overflow should fall back to Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(&*ann.raw, "［＃300字下げ］");
        // The specialised Indent recogniser MUST NOT claim it.
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Indent(_)))),
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_ascii_digit_falls_through() {
        // ASCII-digit variant of the N=0 reject.
        let out = run("［＃0字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Indent(_)))),
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::AlignEnd(_)))),
        );
    }

    #[test]
    fn chitsuki_zero_offset_recognized() {
        let out = run("［＃地付き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::AlignEnd(AlignEnd { offset: 0 }))
        ));
    }

    #[test]
    fn chi_kara_n_ji_age_recognized() {
        let out = run("［＃地から３字上げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::Aozora(AozoraNode::AlignEnd(AlignEnd { offset: 3 }))
        ));
    }

    #[test]
    fn indent_without_digits_falls_through() {
        // "ここから字下げ" is a paired-container opener, not a leaf.
        // C4c2 must not grab it — C4e will.
        let out = run("［＃ここから字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Indent(_)))),
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
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Bouten(b)) => Some(b),
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
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.kind, BoutenKind::Circle);
        assert_eq!(bouten.target.as_plain(), Some("X"));
    }

    #[test]
    fn forward_bouten_all_seven_kinds() {
        let cases = [
            ("傍点", BoutenKind::Goma),
            ("丸傍点", BoutenKind::Circle),
            ("白丸傍点", BoutenKind::WhiteCircle),
            ("二重丸傍点", BoutenKind::DoubleCircle),
            ("蛇の目傍点", BoutenKind::Janome),
            ("波線", BoutenKind::WavyLine),
            ("傍線", BoutenKind::UnderLine),
        ];
        for (suffix, expected_kind) in cases {
            let src = format!("t［＃「t」に{suffix}］");
            let out = run(&src);
            let Some(kind) = out.spans.iter().find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Bouten(b)) => Some(b.kind),
                _ => None,
            }) else {
                panic!("no Bouten span for suffix {suffix:?}");
            };
            assert_eq!(kind, expected_kind, "suffix {suffix:?}");
        }
    }

    #[test]
    fn forward_bouten_empty_target_falls_through() {
        let out = run("［＃「」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_unknown_suffix_falls_through() {
        let out = run("［＃「X」に未知］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_missing_ni_particle_falls_through() {
        let out = run("［＃「X」傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Bouten(_)))),
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Bouten(_)))),
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_tcy_without_preceding_target_falls_through() {
        let out = run("［＃「29」は縦中横］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::TateChuYoko(_)))),
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
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Bouten(b)) => Some(b),
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
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::TateChuYoko(t)) => Some(t),
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::TateChuYoko(_)))),
        );
    }

    #[test]
    fn forward_tcy_empty_target_falls_through() {
        let out = run("［＃「」は縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::TateChuYoko(_)))),
        );
    }

    #[test]
    fn sashie_without_caption_recognized() {
        let out = run("［＃挿絵（fig01.png）入る］");
        let sashie = out
            .spans
            .iter()
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Sashie(s)) => Some(s),
                _ => None,
            })
            .expect("expected a Sashie span");
        assert_eq!(&*sashie.file, "fig01.png");
        assert!(sashie.caption.is_none());
    }

    #[test]
    fn sashie_with_caption_form_not_matched_in_c4c4() {
        // Captioned sashie is F5 territory; the no-caption matcher
        // must reject the captioned form cleanly.
        let out = run("［＃挿絵（fig01.png）「キャプション」入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn sashie_empty_filename_falls_through() {
        let out = run("［＃挿絵（）入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn sashie_missing_iru_suffix_falls_through() {
        let out = run("［＃挿絵（fig01.png）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn gaiji_quoted_description_with_mencode() {
        let out = run("※［＃「木＋吶のつくり」、第3水準1-85-54］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(&*gaiji.description, "木＋吶のつくり");
        assert_eq!(gaiji.mencode.as_deref(), Some("第3水準1-85-54"));
        assert!(gaiji.ucs.is_none(), "C4d does not resolve UCS — G1 does");
    }

    #[test]
    fn gaiji_quoted_description_without_mencode() {
        let out = run("※［＃「試」］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Gaiji(g)) => Some(g),
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
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Gaiji(g)) => Some(g),
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
            .find(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Gaiji(_))))
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Gaiji(_)))),
        );
    }

    #[test]
    fn gaiji_without_hash_is_not_recognized() {
        // ※ followed by ［ but no ＃ inside — not a gaiji shape.
        let out = run("※［普通］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Gaiji(_)))),
        );
    }

    #[test]
    fn kaeriten_ichi_recognized() {
        let out = run("之［＃一］");
        let kaeriten = out
            .spans
            .iter()
            .find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Kaeriten(k)) => Some(k),
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
            let Some(k) = out.spans.iter().find_map(|s| match &s.kind {
                SpanKind::Aozora(AozoraNode::Kaeriten(k)) => Some(k),
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
                .any(|s| matches!(s.kind, SpanKind::Aozora(AozoraNode::Kaeriten(_)))),
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
    fn container_open_close_warichu() {
        let out = run("［＃割り注］内部［＃割り注終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Warichu)
        ));
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::Warichu)
        ));
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
