#![expect(
    clippy::expect_used,
    reason = "classifier state transitions and the source-size cap establish these invariants"
)]

// Forced inlining reduced measured throughput, so the compiler chooses inlining.

use core::mem;
use core::ops::Range;
use std::collections::VecDeque;

use crate::syntax::alloc::Allocator;
use crate::syntax::ast::{Content, Directive, Gaiji, Node, Segment, node_is_content_segment};
use crate::syntax::{DirectiveKind, RegionClose, RegionFormat, Span, ruby_base_class};

use super::pair::{PairEvent, PairKind};
use super::token::TriggerKind;
use crate::spec::Diagnostic;

mod directive;
mod forward;
mod gaiji;
mod kaeriten;
pub(crate) use directive::prewarm;
use forward::install_forward_target_index_from_source;
use kaeriten::{KaeritenObs, classify_kaeriten_mark, family_index, looks_like_kana_prose};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassifiedSpan {
    pub kind: SpanKind,
    pub source_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum SpanKind {
    Plain,
    Aozora(Node),
    BlockOpen(RegionFormat),
    BlockClose(RegionClose),
    Newline,
}

#[must_use]
#[cfg(test)]
pub(crate) fn classify<'src, 'al, I>(
    events: I,
    source: &'src str,
    alloc: &'al mut Allocator,
) -> ClassifyStream<'src, 'al, I::IntoIter>
where
    I: IntoIterator<Item = PairEvent>,
{
    classify_range(
        events,
        source,
        u32::try_from(source.len()).unwrap_or(u32::MAX),
        alloc,
    )
}

#[must_use]
pub(crate) fn classify_range<'src, 'al, I>(
    events: I,
    source: &'src str,
    source_end: u32,
    alloc: &'al mut Allocator,
) -> ClassifyStream<'src, 'al, I::IntoIter>
where
    I: IntoIterator<Item = PairEvent>,
{
    install_forward_target_index_from_source(source);
    ClassifyStream::new(events.into_iter(), source, source_end, alloc)
}

pub(crate) struct ClassifyStream<'src, 'al, I>
where
    I: Iterator<Item = PairEvent>,
{
    events: I,
    source: &'src str,
    source_len: u32,
    alloc: &'al mut Allocator,
    // Document 49178 in the corpus queues about 16k spans, making repeated front removal from a SmallVec quadratic.
    pending_outputs: VecDeque<ClassifiedSpan>,
    frame: Option<Frame>,
    // Streaming avoids buffering and replaying about 24k quote-body events in corpus document 49178.
    streaming: Option<StreamingFrame>,
    pending_plain_start: Option<u32>,
    pending_refmark: Option<Span>,
    pending_ruby_bar: Option<Span>,
    pending_ruby_base: Option<PendingRubyBase>,
    diagnostics: Vec<Diagnostic>,
    kaeriten_obs: Vec<KaeritenObs>,
    finished: bool,
}

#[derive(Debug, Clone, Copy)]
struct StreamingFrame {
    kind: PairKind,
    depth: u32,
}

struct PendingGaiji {
    span: ClassifiedSpan,
    payload: Gaiji,
}

struct PendingRubyBase {
    segs: smallvec::SmallVec<[PendingGaiji; 2]>,
    bar: Option<Span>,
}

impl PendingRubyBase {
    fn single(gaiji: PendingGaiji, bar: Option<Span>) -> Self {
        Self {
            segs: smallvec::smallvec![gaiji],
            bar,
        }
    }

    fn start(&self) -> u32 {
        self.bar
            .map_or(self.segs[0].span.source_span.start, |b| b.start)
    }

    fn end(&self) -> u32 {
        self.segs
            .last()
            .expect("a PendingRubyBase always holds ≥1 gaiji")
            .span
            .source_span
            .end
    }
}

enum GaijiBaseRuby {
    NotApplicable,
    Emitted(ClassifiedSpan),
    Declined,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyView<'b> {
    pub events: &'b [PairEvent],
    pub links: &'b [u32],
}

#[derive(Clone, Copy)]
struct StructuredRubyInput<'b> {
    body: BodyView<'b>,
    open_idx: usize,
    close_idx: usize,
    bar: Option<Span>,
}

pub(crate) struct RecogniseCtx<'al, 's> {
    pub alloc: &'al mut Allocator,
    pub source: &'s str,
    pub diagnostics: Vec<Diagnostic>,
    pub pending_plain_start: Option<u32>,
    pub pending_decoration: Option<(Node, Span)>,
}

struct Frame {
    body: smallvec::SmallVec<[PairEvent; 16]>,
    links: smallvec::SmallVec<[u32; 16]>,
    inner_stack: smallvec::SmallVec<[(PairKind, usize); 8]>,
    gaiji_refmark: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameOutcome {
    Open,
    Closed,
    Abandoned,
}

fn first_nested_ruby_open(events: &[PairEvent], lo: usize, hi: usize) -> Option<Span> {
    events[lo..hi].iter().find_map(|e| match e {
        PairEvent::PairOpen {
            kind: PairKind::Ruby,
            span,
        } => Some(*span),
        _ => None,
    })
}

fn empty_explicit_ruby_span(
    bar_byte_offset: Option<usize>,
    preceding_start: u32,
    open_span: Span,
    close_span: Span,
) -> Option<Span> {
    let bar_off = bar_byte_offset?;
    if open_span.end.cmp(&close_span.start).is_ne() {
        return None;
    }
    let bar_pos = preceding_start.checked_add(u32::try_from(bar_off).ok()?)?;
    Some(Span::new(bar_pos, close_span.end))
}

type SynthRubyView = (
    smallvec::SmallVec<[PairEvent; 16]>,
    smallvec::SmallVec<[u32; 16]>,
    usize,
);

fn build_synth_ruby_view(
    body: BodyView<'_>,
    prev_text_range: Span,
    bar_byte_offset: Option<usize>,
) -> Option<SynthRubyView> {
    let mut synth: smallvec::SmallVec<[PairEvent; 16]> =
        smallvec::SmallVec::with_capacity(body.events.len().saturating_add(2));
    let mut synth_links: smallvec::SmallVec<[u32; 16]> =
        smallvec::SmallVec::with_capacity(body.events.len().saturating_add(2));
    let synth_open_idx = if let Some(bar_off) = bar_byte_offset {
        let bar_pos = prev_text_range
            .start
            .saturating_add(u32::try_from(bar_off).unwrap_or(u32::MAX));
        let bar_span = Span::new(
            bar_pos,
            bar_pos.saturating_add(u32::try_from('｜'.len_utf8()).unwrap_or(u32::MAX)),
        );
        synth.push(PairEvent::Solo {
            kind: TriggerKind::Bar,
            span: bar_span,
        });
        synth_links.push(u32::MAX);
        if bar_span.end >= prev_text_range.end {
            return None;
        }
        synth.push(PairEvent::Text {
            range: Span::new(bar_span.end, prev_text_range.end),
        });
        synth_links.push(u32::MAX);
        2
    } else {
        synth.push(PairEvent::Text {
            range: prev_text_range,
        });
        synth_links.push(u32::MAX);
        1
    };
    let shift = u32::try_from(synth.len()).expect("synth prefix fits u32");
    synth.extend(body.events.iter().cloned());
    synth_links.extend(body.links.iter().map(|&l| {
        if l == u32::MAX {
            u32::MAX
        } else {
            l.saturating_add(shift)
        }
    }));
    Some((synth, synth_links, synth_open_idx))
}

impl<'src, 'al, I> ClassifyStream<'src, 'al, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn new(events: I, source: &'src str, source_len: u32, alloc: &'al mut Allocator) -> Self {
        Self {
            events,
            source,
            source_len,
            alloc,
            pending_outputs: VecDeque::new(),
            frame: None,
            streaming: None,
            pending_plain_start: None,
            pending_refmark: None,
            pending_ruby_bar: None,
            pending_ruby_base: None,
            diagnostics: Vec::new(),
            kaeriten_obs: Vec::new(),
            finished: false,
        }
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        self.finalize_kaeriten();
        mem::take(&mut self.diagnostics)
    }

    // Document-wide kaeriten base checks reduced corpus false positives from 586 across 337 works to 2.
    fn finalize_kaeriten(&mut self) {
        let obs = mem::take(&mut self.kaeriten_obs);
        if obs.is_empty() {
            return;
        }
        if let [only] = obs.as_slice()
            && looks_like_kana_prose(self.source, only.span)
        {
            self.diagnostics
                .push(Diagnostic::kaeriten_outside_kanbun(only.span));
        }
        let mut has_base = [false; 3];
        for o in obs
            .iter()
            .filter(|o| o.is_ladder)
            .filter(|o| matches!(o.rank, 1))
        {
            has_base[family_index(o.family)] = true;
        }
        for o in obs
            .iter()
            .filter(|o| o.is_ladder)
            .filter(|o| matches!(o.rank, 2..=u8::MAX))
        {
            if !has_base[family_index(o.family)] {
                self.diagnostics
                    .push(Diagnostic::bracketed_kaeriten_no_pair(o.span));
            }
        }
    }

    fn push_output(&mut self, span: ClassifiedSpan) {
        self.pending_outputs.push_back(span);
    }

    fn flush_plain_up_to(&mut self, end: u32) {
        if let Some(rm) = self.pending_refmark.take()
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        if let Some(start) = self.pending_plain_start.take()
            && end > start
        {
            self.push_output(ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(start, end),
            });
        }
    }

    fn splice_plain_around(&mut self, deco: Node, deco_span: Span) {
        self.flush_plain_up_to(deco_span.start);
        self.push_output(ClassifiedSpan {
            kind: SpanKind::Aozora(deco),
            source_span: deco_span,
        });
        self.pending_plain_start = Some(deco_span.end);
    }

    fn open_frame(&mut self, open_event: PairEvent, gaiji_refmark: Option<Span>) {
        let mut body: smallvec::SmallVec<[PairEvent; 16]> = smallvec::SmallVec::new();
        let mut links: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
        let mut inner_stack = smallvec::SmallVec::new();
        let &PairEvent::PairOpen { kind, .. } = &open_event else {
            unreachable!("open_frame called with non-PairOpen event");
        };
        inner_stack.push((kind, 0_usize));
        body.push(open_event);
        links.push(u32::MAX);
        self.frame = Some(Frame {
            body,
            links,
            inner_stack,
            gaiji_refmark,
        });
    }

    fn append_to_frame(&mut self, event: PairEvent) -> FrameOutcome {
        let frame = self
            .frame
            .as_mut()
            .expect("append_to_frame requires an active frame");
        let body_idx = frame.body.len();

        match &event {
            PairEvent::PairOpen { kind, .. } => {
                frame.inner_stack.push((*kind, body_idx));
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
            PairEvent::PairClose { kind, .. } => {
                if let Some(pos) = frame.inner_stack.iter().rposition(|&(k, _)| k == *kind) {
                    let (_, open_body_idx) = frame.inner_stack.remove(pos);
                    frame.body.push(event);
                    let body_idx_u32 = u32::try_from(body_idx)
                        .expect("body_idx fits u32 (corpus body lengths are bounded)");
                    let open_body_idx_u32 = u32::try_from(open_body_idx)
                        .expect("body_idx fits u32 (corpus body lengths are bounded)");
                    frame.links.push(open_body_idx_u32);
                    frame.links[open_body_idx] = body_idx_u32;
                } else {
                    frame.body.push(event);
                    frame.links.push(u32::MAX);
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                if let Some(pos) = frame.inner_stack.iter().rposition(|&(k, _)| k == *kind) {
                    match pos {
                        0 => return FrameOutcome::Abandoned,
                        _ => {
                            frame.inner_stack.remove(pos);
                        }
                    }
                }
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
            _ => {
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
        }

        if frame.inner_stack.is_empty() {
            FrameOutcome::Closed
        } else {
            FrameOutcome::Open
        }
    }

    fn recognize_and_emit(&mut self) {
        let frame = self
            .frame
            .take()
            .expect("recognize_and_emit requires an active frame");
        let body = frame.body;
        let links = frame.links;
        debug_assert!(body.len() >= 2, "frame body must contain open + close");
        debug_assert_eq!(body.len(), links.len(), "links must parallel body");

        let open_idx = 0usize;
        let close_idx = body.len() - 1;

        let PairEvent::PairOpen {
            kind: open_kind, ..
        } = body[open_idx]
        else {
            unreachable!("frame body[0] must be PairOpen");
        };

        let view = BodyView {
            events: &body,
            links: &links,
        };

        match open_kind {
            PairKind::Ruby => {
                if let Some(span) = self.try_ruby_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            PairKind::AngleQuote => {
                if let Some(span) = self.try_angle_quote_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            PairKind::Bracket => {
                let refmark = frame.gaiji_refmark;
                if let Some(rm_span) = refmark {
                    // Reusing the body avoids copies measured in corpus documents 50685 and 49178.
                    if let Some((gaiji, bar)) = self.try_gaiji_emit(view, open_idx, rm_span) {
                        let gaiji_start = gaiji.span.source_span.start;
                        let adjacent = self
                            .pending_ruby_base
                            .as_ref()
                            .is_some_and(|p| p.end() == gaiji_start);
                        if adjacent {
                            self.pending_ruby_base
                                .as_mut()
                                .expect("adjacent implies Some")
                                .segs
                                .push(gaiji);
                        } else {
                            if let Some(old) = self.pending_ruby_base.take() {
                                self.emit_pending_gaiji(old);
                            }
                            self.pending_ruby_base = Some(PendingRubyBase::single(gaiji, bar));
                        }
                        return;
                    }
                    if let Some(pending) = self.pending_ruby_base.take() {
                        self.emit_pending_gaiji(pending);
                    }
                    if self.pending_plain_start.is_none() {
                        self.pending_plain_start = Some(rm_span.start);
                    }
                    if let Some(span) = self.try_bracket_emit(view, open_idx, close_idx) {
                        self.push_output(span);
                        return;
                    }
                    self.replay_unrecognised_body(body, None);
                    return;
                }
                if let Some(span) = self.try_bracket_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            _ => {}
        }

        self.replay_unrecognised_body(body, frame.gaiji_refmark);
    }

    fn replay_unrecognised_body(
        &mut self,
        body: smallvec::SmallVec<[PairEvent; 16]>,
        refmark: Option<Span>,
    ) {
        if let Some(rm) = refmark
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        let mut events = body.into_iter();
        let Some(first) = events.next() else {
            return;
        };
        if let Some(span) = first.span()
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(span.start);
        }
        let mut trailing = None;
        for event in events {
            if let Some(inner) = trailing.replace(event)
                && !matches!(inner, PairEvent::Unclosed { .. })
            {
                self.process_event(inner);
            }
        }
        if let Some(last) = trailing {
            if matches!(last, PairEvent::Unclosed { .. }) {
                if let Some(refmark) = self.pending_refmark.take()
                    && self.pending_plain_start.is_none()
                {
                    self.pending_plain_start = Some(refmark.start);
                }
            } else {
                self.process_event(last);
            }
        }
    }

    fn handle_top_level(&mut self, event: PairEvent) {
        match event {
            PairEvent::Newline { pos } => {
                self.pending_ruby_bar = None;
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } => {
                self.pending_refmark = Some(span);
            }
            PairEvent::Solo {
                kind: TriggerKind::Bar,
                span,
            } => {
                self.remember_ruby_bar(span);
            }
            PairEvent::PairOpen { kind, span, .. } => {
                if matches!(kind, PairKind::Quote | PairKind::Tortoise) {
                    let pre_open = self
                        .pending_refmark
                        .take()
                        .map_or(span.start, |rm| rm.start);
                    self.flush_plain_up_to(pre_open);
                    if self.pending_plain_start.is_none() {
                        self.pending_plain_start = Some(span.start);
                    }
                    self.streaming = Some(StreamingFrame { kind, depth: 1 });
                    return;
                }
                let gaiji_refmark = if matches!(kind, PairKind::Bracket) {
                    self.pending_refmark.take()
                } else {
                    None
                };
                let preserve_pending_plain = matches!(
                    kind,
                    PairKind::Ruby | PairKind::AngleQuote | PairKind::Bracket
                );
                if !preserve_pending_plain {
                    let truncate_to = gaiji_refmark.map_or(span.start, |rm| rm.start);
                    self.flush_plain_up_to(truncate_to);
                }
                self.open_frame(PairEvent::PairOpen { kind, span }, gaiji_refmark);
            }
            other => {
                let Some(span) = other.span() else {
                    return;
                };
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
        }
    }

    fn fold_held_refmark(&mut self, event: &PairEvent) {
        if self.pending_refmark.is_some()
            && !matches!(
                event,
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        {
            let rm = self.pending_refmark.take().expect("checked Some");
            if self.pending_plain_start.is_none() {
                self.pending_plain_start = Some(rm.start);
            }
        }
    }

    fn remember_ruby_bar(&mut self, span: Span) {
        self.pending_ruby_bar = Some(span);
        if self.pending_plain_start.is_none() {
            self.pending_plain_start = Some(span.start);
        }
    }

    fn handle_stream_event(&mut self, event: PairEvent) {
        self.fold_held_refmark(&event);

        if let PairEvent::PairOpen { kind, span } = &event
            && matches!(
                kind,
                PairKind::Ruby | PairKind::AngleQuote | PairKind::Bracket
            )
        {
            let gaiji_refmark = if matches!(kind, PairKind::Bracket) {
                self.pending_refmark.take()
            } else {
                None
            };
            self.open_frame(
                PairEvent::PairOpen {
                    kind: *kind,
                    span: *span,
                },
                gaiji_refmark,
            );
            return;
        }
        let stream = self
            .streaming
            .as_mut()
            .expect("handle_stream_event without streaming state");
        match event {
            PairEvent::Newline { pos } => {
                self.pending_ruby_bar = None;
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::PairOpen { kind, span } if kind.eq(&stream.kind) => {
                stream.depth = stream.depth.saturating_add(1);
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
            PairEvent::PairClose { kind, span } if kind.eq(&stream.kind) => {
                stream.depth = stream.depth.saturating_sub(1);
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
                if matches!(stream.depth, 0) {
                    self.streaming = None;
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                if kind.eq(&stream.kind) {
                    stream.depth = stream.depth.saturating_sub(1);
                    if matches!(stream.depth, 0) {
                        self.streaming = None;
                    }
                }
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } => {
                self.pending_refmark = Some(span);
            }
            PairEvent::Solo {
                kind: TriggerKind::Bar,
                span,
            } => {
                self.remember_ruby_bar(span);
            }
            other => {
                let Some(span) = other.span() else {
                    return;
                };
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
        }
    }

    fn try_ruby_over_gaiji_base(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> GaijiBaseRuby {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return GaijiBaseRuby::NotApplicable;
        };
        let gaiji_base = self.pending_plain_start.is_none()
            && self
                .pending_ruby_base
                .as_ref()
                .is_some_and(|p| p.end() == open_span.start);
        if !gaiji_base {
            return GaijiBaseRuby::NotApplicable;
        }
        let pending = self.pending_ruby_base.take().expect("checked Some");
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            self.emit_pending_gaiji(pending);
            return GaijiBaseRuby::Declined;
        };
        if open_span.end >= close_span.start {
            self.emit_pending_gaiji(pending);
            return GaijiBaseRuby::Declined;
        }
        let reading = {
            let mut ctx = RecogniseCtx {
                alloc: self.alloc,
                source: self.source,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            let reading = ctx.build_content_from_body(
                body,
                &BodyWindow {
                    events: open_idx.saturating_add(1)..close_idx,
                    bytes: open_span.end..close_span.start,
                },
            );
            self.diagnostics.append(&mut ctx.diagnostics);
            reading
        };
        let base_start = pending.start();
        let segs: smallvec::SmallVec<[Segment; 2]> = pending
            .segs
            .iter()
            .map(|g| self.alloc.seg_gaiji(g.payload))
            .collect();
        let base = self.alloc.content_segments(&segs);
        let node = self.alloc.ruby(base, reading);
        GaijiBaseRuby::Emitted(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(base_start, close_span.end),
        })
    }

    fn try_ruby_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan> {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return None;
        };
        let explicit_bar = self.pending_ruby_bar.take();

        let reading_start = open_idx.saturating_add(1);
        if let Some(inner_open) = first_nested_ruby_open(body.events, reading_start, close_idx) {
            self.diagnostics.push(Diagnostic::nested_ruby(inner_open));
        }

        match self.try_ruby_over_gaiji_base(body, open_idx, close_idx) {
            GaijiBaseRuby::Emitted(span) => return Some(span),
            GaijiBaseRuby::Declined => return None,
            GaijiBaseRuby::NotApplicable => {}
        }

        let preceding_start = self.pending_plain_start.unwrap_or(open_span.start);
        if preceding_start >= open_span.start {
            return self.try_structured_explicit_ruby(StructuredRubyInput {
                body,
                open_idx,
                close_idx,
                bar: explicit_bar,
            });
        }
        let prev_text_range = Span::new(preceding_start, open_span.start);
        let bar_byte_offset = explicit_bar
            .filter(|bar| preceding_start <= bar.start && bar.end <= open_span.start)
            .map(|bar| {
                usize::try_from(bar.start.saturating_sub(preceding_start))
                    .expect("source span offset fits usize")
            });

        let (synth, synth_links, synth_open_idx) =
            build_synth_ruby_view(body, prev_text_range, bar_byte_offset)?;
        let synth_close_idx = synth_open_idx.saturating_add(close_idx.saturating_sub(open_idx));
        let synth_view = BodyView {
            events: synth.as_slice(),
            links: synth_links.as_slice(),
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decoration: None,
        };
        let Some(m) = ctx.recognize_ruby(synth_view, synth_open_idx, synth_close_idx) else {
            if let PairEvent::PairClose {
                span: close_span, ..
            } = body.events[close_idx]
                && let Some(span) = empty_explicit_ruby_span(
                    bar_byte_offset,
                    preceding_start,
                    open_span,
                    close_span,
                )
            {
                self.diagnostics.push(Diagnostic::empty_ruby_reading(span));
            }
            self.diagnostics.append(&mut ctx.diagnostics);
            return self.try_structured_explicit_ruby(StructuredRubyInput {
                body,
                open_idx,
                close_idx,
                bar: explicit_bar,
            });
        };
        self.diagnostics.append(&mut ctx.diagnostics);
        self.flush_plain_up_to(m.consume_start);
        let base_content = self.alloc.content_plain(m.base);
        let node = self.alloc.ruby(base_content, m.reading);
        self.pending_plain_start = None;
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    fn try_structured_explicit_ruby(
        &mut self,
        input: StructuredRubyInput<'_>,
    ) -> Option<ClassifiedSpan> {
        let StructuredRubyInput {
            body,
            open_idx,
            close_idx,
            bar,
        } = input;
        let bar = bar?;
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return None;
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            return None;
        };
        if bar.end >= open_span.start || open_span.end >= close_span.start {
            return None;
        }
        let reading = {
            let mut ctx = RecogniseCtx {
                alloc: self.alloc,
                source: self.source,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            let reading = ctx.build_content_from_body(
                body,
                &BodyWindow {
                    events: open_idx.saturating_add(1)..close_idx,
                    bytes: open_span.end..close_span.start,
                },
            );
            self.diagnostics.append(&mut ctx.diagnostics);
            reading
        };
        let base = self
            .alloc
            .content_plain(&self.source[bar.end as usize..open_span.start as usize]);
        let node = self.alloc.ruby(base, reading);
        self.flush_plain_up_to(open_span.start);
        self.pending_plain_start = None;
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(open_span.start, close_span.end),
        })
    }

    fn try_angle_quote_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan> {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            unreachable!("body[open_idx] must be PairOpen");
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            unreachable!("body[close_idx] must be PairClose");
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decoration: None,
        };
        let content = ctx.build_content_from_body(
            body,
            &BodyWindow {
                events: open_idx.saturating_add(1)..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );
        self.diagnostics.append(&mut ctx.diagnostics);
        let content_is_empty = match content {
            Content::Plain(s) => self.alloc.store().resolve_str(s).is_empty(),
            Content::Segments(segs) => segs.len == 0,
        };
        if content_is_empty {
            return None;
        }
        self.flush_plain_up_to(open_span.start);
        let node = self.alloc.angle_quote(content);
        self.pending_plain_start = None;
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(open_span.start, close_span.end),
        })
    }

    fn try_bracket_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan> {
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: self.pending_plain_start,
            pending_decoration: None,
        };
        let m = ctx.recognize_annotation(body, open_idx, close_idx)?;
        self.diagnostics.append(&mut ctx.diagnostics);
        let decoration = ctx.pending_decoration.take();
        if let Some((deco, deco_span)) = decoration
            && self
                .pending_plain_start
                .is_some_and(|ps| ps <= deco_span.start)
            && deco_span.end <= m.consume_start
        {
            self.splice_plain_around(deco, deco_span);
        }
        self.flush_plain_up_to(m.consume_start);
        let kind = match m.emit {
            EmitKind::Aozora(node) => SpanKind::Aozora(node),
            EmitKind::BlockOpen(container) => SpanKind::BlockOpen(container),
            EmitKind::BlockClose(container) => SpanKind::BlockClose(container),
        };
        if matches!(
            kind,
            SpanKind::BlockOpen(_)
                | SpanKind::BlockClose(_)
                | SpanKind::Aozora(
                    Node::Line(_)
                        | Node::PageBreak
                        | Node::SectionBreak(_)
                        | Node::BodyEnd
                        | Node::ForcedBreak
                        | Node::Heading(_)
                )
        ) {
            self.pending_ruby_bar = None;
        }
        self.pending_plain_start = None;
        if let Some(diag) = m.pending_diagnostic {
            self.diagnostics.push(diag);
        }
        if let SpanKind::Aozora(Node::Kaeriten(k)) = kind {
            let span = Span::new(m.consume_start, m.consume_end);
            let (family, rank, is_ladder) =
                classify_kaeriten_mark(self.alloc.store().resolve_str(k.mark));
            self.kaeriten_obs.push(KaeritenObs {
                family,
                rank,
                is_ladder,
                span,
            });
        }
        Some(ClassifiedSpan {
            kind,
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    fn try_gaiji_emit(
        &mut self,
        body: BodyView<'_>,
        bracket_open_idx: usize,
        refmark_span: Span,
    ) -> Option<(PendingGaiji, Option<Span>)> {
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decoration: None,
        };
        let m = ctx.recognize_gaiji(body, refmark_span, bracket_open_idx)?;
        let before = &self.source[..m.consume_start as usize];
        let bar = before.ends_with('｜').then(|| {
            let bar_start = m
                .consume_start
                .checked_sub(u32::try_from('｜'.len_utf8()).expect("bar length fits u32"))
                .expect("matched bar precedes gaiji");
            Span::new(bar_start, m.consume_start)
        });
        self.flush_plain_up_to(bar.map_or(m.consume_start, |b| b.start));
        let node = self.alloc.gaiji(m.payload);
        self.pending_plain_start = None;
        if m.payload.resolve(self.alloc.store()).is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    m.consume_start,
                    m.consume_end,
                )));
        }
        Some((
            PendingGaiji {
                span: ClassifiedSpan {
                    kind: SpanKind::Aozora(node),
                    source_span: Span::new(m.consume_start, m.consume_end),
                },
                payload: m.payload,
            },
            bar,
        ))
    }

    fn emit_pending_gaiji(&mut self, pending: PendingRubyBase) {
        if let Some(bar) = pending.bar {
            self.push_output(ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: bar,
            });
        }
        for gaiji in pending.segs {
            self.push_output(gaiji.span);
        }
    }

    fn finalize(&mut self) {
        if let Some(rm) = self.pending_refmark.take()
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        let end = self.source_len;
        self.flush_plain_up_to(end);
    }
}

impl<I> Iterator for ClassifyStream<'_, '_, I>
where
    I: Iterator<Item = PairEvent>,
{
    type Item = ClassifiedSpan;

    fn next(&mut self) -> Option<ClassifiedSpan> {
        loop {
            if let Some(span) = self.pending_outputs_pop_front() {
                return Some(span);
            }
            if self.finished {
                return None;
            }
            let next_event = self.events.next();
            if let Some(event) = next_event {
                self.process_event(event);
            } else {
                if let Some(pending) = self.pending_ruby_base.take() {
                    self.emit_pending_gaiji(pending);
                }
                if let Some(frame) = self.frame.take() {
                    let refmark = frame.gaiji_refmark;
                    self.replay_unrecognised_body(frame.body, refmark);
                }
                self.finalize();
                self.finished = true;
            }
        }
    }
}

impl<I> ClassifyStream<'_, '_, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn pending_outputs_pop_front(&mut self) -> Option<ClassifiedSpan> {
        self.pending_outputs.pop_front()
    }

    fn process_event(&mut self, event: PairEvent) {
        if self.frame.is_some() {
            debug_assert!(
                self.pending_refmark.is_none(),
                "a pending refmark should have been absorbed or flushed before frame entry"
            );
            match self.append_to_frame(event) {
                FrameOutcome::Closed => self.recognize_and_emit(),
                FrameOutcome::Abandoned => {
                    let Some(frame) = self.frame.take() else {
                        unreachable!("abandoned outcome requires an active frame");
                    };
                    self.replay_unrecognised_body(frame.body, frame.gaiji_refmark);
                }
                FrameOutcome::Open => {}
            }
            return;
        }

        let flush_gaiji = if let Some(pending) = self.pending_ruby_base.as_ref() {
            let end = pending.end();
            let continues = match &event {
                PairEvent::PairOpen {
                    kind: PairKind::Ruby,
                    span,
                }
                | PairEvent::Solo {
                    kind: TriggerKind::RefMark,
                    span,
                } => span.start == end,
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                } => self.pending_refmark.is_some_and(|rm| rm.start == end),
                _ => false,
            };
            !continues
        } else {
            false
        };
        if flush_gaiji {
            let pending = self.pending_ruby_base.take().expect("checked Some");
            self.emit_pending_gaiji(pending);
        }

        if self.streaming.is_some() {
            self.handle_stream_event(event);
            return;
        }

        if self.pending_refmark.is_some()
            && !matches!(
                event,
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        {
            let rm = self.pending_refmark.take().expect("checked Some");
            if self.pending_plain_start.is_none() {
                self.pending_plain_start = Some(rm.start);
            }
        }

        self.handle_top_level(event);
    }
}

struct RubyMatch<'s> {
    base: &'s str,
    reading: Content,
    consume_start: u32,
    consume_end: u32,
}

impl<'s> RecogniseCtx<'_, 's> {
    fn recognize_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<RubyMatch<'s>> {
        let events = view.events;
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
            return None;
        }
        let previous_idx = open_idx.checked_sub(1)?;
        let PairEvent::Text {
            range: prev_range, ..
        } = events[previous_idx]
        else {
            return None;
        };
        let prev_text = &self.source[prev_range.start as usize..prev_range.end as usize];

        let reading = self.build_content_from_body(
            view,
            &BodyWindow {
                events: open_idx.saturating_add(1)..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );

        if let Some(bar_idx) = open_idx.checked_sub(2)
            && let PairEvent::Solo {
                kind: TriggerKind::Bar,
                span: bar_span,
            } = events[bar_idx]
        {
            if prev_text.is_empty() {
                return None;
            }
            return Some(RubyMatch {
                base: prev_text,
                reading,
                consume_start: bar_span.start,
                consume_end: close_span.end,
            });
        }

        let base_offset = trailing_ruby_base_start(prev_text);
        if base_offset == prev_text.len() {
            return None;
        }
        let consume_start = prev_range
            .start
            .saturating_add(u32::try_from(base_offset).unwrap_or(u32::MAX));
        Some(RubyMatch {
            base: &prev_text[base_offset..],
            reading,
            consume_start,
            consume_end: close_span.end,
        })
    }
}

struct BodyWindow {
    events: Range<usize>,
    bytes: Range<u32>,
}

#[derive(Clone, Copy)]
struct BodyWalkCtx<'b> {
    view: BodyView<'b>,
    window: &'b BodyWindow,
}

struct ContentBuild {
    segments: Vec<Segment>,
    text_start: u32,
}

impl RecogniseCtx<'_, '_> {
    fn build_content_from_source_slice(&mut self, view: BodyView<'_>, text: &str) -> Content {
        let source_start = self.source.as_ptr() as usize;
        let text_start = (text.as_ptr() as usize)
            .checked_sub(source_start)
            .expect("nested content slice starts inside the source");
        let text_end = text_start.saturating_add(text.len());
        assert!(
            text_end <= self.source.len(),
            "nested content slice ends inside the source"
        );
        let byte_start = u32::try_from(text_start).expect("source length fits u32");
        let byte_end = u32::try_from(text_end).expect("source length fits u32");
        let event_window = source_slice_event_window(view.events, byte_start..byte_end);
        self.build_content_from_body(
            view,
            &BodyWindow {
                events: event_window,
                bytes: byte_start..byte_end,
            },
        )
    }

    fn build_content_from_body(&mut self, view: BodyView<'_>, window: &BodyWindow) -> Content {
        debug_assert!(
            window.events.start <= window.events.end,
            "body window event range must be non-inverted",
        );
        debug_assert!(
            window.bytes.start <= window.bytes.end,
            "body window byte range must be non-inverted",
        );
        debug_assert_eq!(
            view.events.len(),
            view.links.len(),
            "BodyView events/links must be parallel",
        );

        let body_events = &view.events[window.events.start..window.events.end];
        let nested_candidate = body_events.iter().find(|event| {
            matches!(
                event,
                PairEvent::Solo {
                    kind: TriggerKind::RefMark,
                    ..
                } | PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        });
        if nested_candidate.is_none() {
            let text = &self.source[window.bytes.start as usize..window.bytes.end as usize];
            return self.alloc.content_plain(text);
        }

        let body = BodyWalkCtx { view, window };
        let mut build = ContentBuild {
            segments: Vec::with_capacity(body_events.len().saturating_add(1)),
            text_start: window.bytes.start,
        };
        let mut i = window.events.start;
        while window.events.contains(&i) {
            if let Some(next_i) = self.try_emit_gaiji_at(body, &mut build, i) {
                assert!(
                    next_i.cmp(&i).is_gt(),
                    "nested gaiji recognition must advance the event cursor"
                );
                i = next_i;
                continue;
            }
            if let Some(next_i) = self.try_emit_annotation_at(body, &mut build, i) {
                assert!(
                    next_i.cmp(&i).is_gt(),
                    "nested annotation recognition must advance the event cursor"
                );
                i = next_i;
                continue;
            }
            let previous = i;
            i = i.saturating_add(1);
            assert!(i > previous, "content walk must advance the event cursor");
        }
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..window.bytes.end,
            self.alloc,
        );
        self.alloc.content_segments(&build.segments)
    }

    fn try_emit_gaiji_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::Solo {
            kind: TriggerKind::RefMark,
            span: refmark_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let bracket_idx = i.checked_add(1)?;
        if !body.window.events.contains(&bracket_idx) {
            return None;
        }
        let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            ..
        } = body.view.events[bracket_idx]
        else {
            return None;
        };
        let close_link = body.view.links[bracket_idx];
        if close_link == u32::MAX {
            return None;
        }
        let close_idx = close_link as usize;
        if !body.window.events.contains(&close_idx) {
            return None;
        }
        let g = self.recognize_gaiji(body.view, refmark_span, bracket_idx)?;
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..g.consume_start,
            self.alloc,
        );
        build.segments.push(self.alloc.seg_gaiji(g.payload));
        build.text_start = g.consume_end;
        if g.payload.resolve(self.alloc.store()).is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    g.consume_start,
                    g.consume_end,
                )));
        }
        close_idx.checked_add(1)
    }

    fn try_emit_annotation_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            span: open_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let close_link = body.view.links[i];
        if close_link == u32::MAX {
            return None;
        }
        let close_idx = close_link as usize;
        if !body.window.events.contains(&close_idx) {
            return None;
        }
        let a = self.recognize_annotation(body.view, i, close_idx)?;
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.view.events[close_idx]
        else {
            unreachable!("PairOpen link must target a PairClose");
        };
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..a.consume_start,
            self.alloc,
        );
        match a.emit {
            EmitKind::Aozora(Node::Gaiji(g)) => {
                build.segments.push(self.alloc.seg_gaiji(g));
            }
            EmitKind::Aozora(Node::Directive(directive)) => {
                build.segments.push(self.alloc.seg_annotation(directive));
            }
            EmitKind::Aozora(node) if node_is_content_segment(node) => {
                build.segments.push(self.alloc.seg_node(node));
            }
            EmitKind::Aozora(_) | EmitKind::BlockOpen(_) | EmitKind::BlockClose(_) => {
                let payload = if let Some(p) = a.annotation_payload {
                    p
                } else {
                    let raw = &self.source[open_span.start as usize..close_span.end as usize];
                    self.alloc.make_directive(raw, DirectiveKind::Editorial)
                };
                build.segments.push(self.alloc.seg_annotation(payload));
            }
        }
        if let Some(diagnostic) = a.pending_diagnostic {
            self.diagnostics.push(diagnostic);
        }
        build.text_start = a.consume_end;
        close_idx.checked_add(1)
    }
}

fn source_slice_event_window(events: &[PairEvent], bytes: Range<u32>) -> Range<usize> {
    let start = events
        .iter()
        .position(|event| event.span().is_some_and(|span| span.end > bytes.start))
        .unwrap_or(events.len());
    let end = events
        .iter()
        .rposition(|event| event.span().is_some_and(|span| span.start < bytes.end))
        .map_or(start, |idx| idx.saturating_add(1));
    start..end
}

#[inline]
fn push_text_segment(
    segments: &mut Vec<Segment>,
    source: &str,
    bytes: Range<u32>,
    alloc: &mut Allocator,
) {
    if !bytes.is_empty() {
        segments.push(alloc.seg_text(&source[bytes.start as usize..bytes.end as usize]));
    }
}

fn trailing_ruby_base_start(text: &str) -> usize {
    let Some(last) = text.chars().next_back() else {
        return text.len();
    };
    let Some(base_class) = ruby_base_class(last) else {
        return text.len();
    };
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        if ruby_base_class(ch) == Some(base_class) {
            start = idx;
        } else {
            break;
        }
    }
    start
}

struct AnnotationMatch {
    emit: EmitKind,
    annotation_payload: Option<Directive>,
    consume_start: u32,
    consume_end: u32,
    pending_diagnostic: Option<Diagnostic>,
}

enum EmitKind {
    Aozora(Node),
    BlockOpen(RegionFormat),
    BlockClose(RegionClose),
}

#[cfg(test)]
mod tests {

    use core::iter;

    use super::*;
    use crate::syntax::ast::{Content, ContentRange, Node, NodeStore, Segment, StrId};
    use crate::syntax::{BoutenKind, BoutenPosition, ForwardAttr, ForwardOrigin, SectionKind};

    use crate::pipeline::lexer::pair::pair;
    use crate::pipeline::lexer::tokenize::tokenize;

    #[derive(Debug)]
    struct TestClassifyOutput {
        spans: Vec<ClassifiedSpan>,
        diagnostics: Vec<Diagnostic>,
        store: NodeStore,
    }

    impl TestClassifyOutput {
        fn plain(&self, range: ContentRange) -> Option<&str> {
            self.store.content_range_as_plain(range)
        }

        fn s(&self, id: StrId) -> &str {
            self.store.resolve_str(id)
        }

        fn contents(&self, range: ContentRange) -> Vec<Content> {
            self.store.resolve_content_range(range).to_vec()
        }

        fn only_aozora(&self) -> Node {
            let mut found = None;
            for span in &self.spans {
                if let SpanKind::Aozora(node) = span.kind {
                    assert!(
                        found.is_none(),
                        "more than one Aozora span: {:?}",
                        self.spans
                    );
                    found = Some(node);
                }
            }
            found.unwrap_or_else(|| panic!("no Aozora span in {:?}", self.spans))
        }
    }

    macro_rules! run {
        ($name:ident, $src:expr) => {
            let mut alloc = Allocator::new();
            let mut pair_stream = pair(tokenize($src));
            let mut spans: Vec<ClassifiedSpan> = Vec::new();
            let classify_diagnostics: Vec<Diagnostic> = {
                let mut stream = classify(&mut pair_stream, $src, &mut alloc);
                for span in &mut stream {
                    spans.push(span);
                }
                stream.take_diagnostics()
            };
            let mut diagnostics = pair_stream.take_diagnostics();
            diagnostics.extend(classify_diagnostics);
            let $name = TestClassifyOutput {
                spans,
                diagnostics,
                store: alloc.into_store(),
            };
        };
    }

    #[test]
    fn empty_input_produces_empty_span_vector() {
        run!(out, "");
        assert_eq!(out.spans, Vec::<ClassifiedSpan>::new());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn plain_ascii_becomes_single_plain_span() {
        run!(out, "hello");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[0].source_span, Span::new(0, 5));
    }

    #[test]
    fn source_slice_event_window_excludes_adjacent_events() {
        let events = [
            PairEvent::Text {
                range: Span::new(0, 3),
            },
            PairEvent::Text {
                range: Span::new(3, 6),
            },
            PairEvent::Text {
                range: Span::new(6, 9),
            },
        ];

        assert_eq!(source_slice_event_window(&events, 3..6), 1..2);
    }

    #[test]
    fn newline_in_middle_splits_into_three_spans() {
        run!(out, "a\nb");
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert_eq!(out.spans[2].kind, SpanKind::Plain);
    }

    #[test]
    fn explicit_ruby_collapses_to_plain_base_and_reading() {
        run!(out, "｜青梅《おうめ》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("青梅"));
        assert_eq!(out.plain(r.reading), Some("おうめ"));
    }

    #[test]
    fn stray_bracket_does_not_sink_following_ruby() {
        run!(out, "［\n｜漢字《かんじ》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("post-stray ruby must survive, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("漢字"));
        assert_eq!(out.plain(r.reading), Some("かんじ"));
        assert!(
            out.spans
                .iter()
                .any(|s| s.kind == SpanKind::Plain && s.source_span == Span::new(0, 3)),
            "stray `［` must fold to plain: {:?}",
            out.spans
        );
        assert!(out.diagnostics.iter().any(|d| matches!(
            d,
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        )));
    }

    #[test]
    fn ruby_reading_with_embedded_gaiji_produces_segments() {
        run!(out, "｜日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby");
        };
        assert_eq!(out.plain(r.base), Some("日本"));
        let reading = out.contents(r.reading);
        let [Content::Segments(seg_range)] = reading[..] else {
            panic!("expected a single Segments reading, got {reading:?}");
        };
        let segs = out.store.resolve_seg_range(seg_range).to_vec();
        assert_eq!(segs.len(), 3);
        assert!(matches!(segs[0], Segment::Text(t) if out.s(t) == "に"));
        assert!(matches!(segs[1], Segment::Gaiji(_)));
        assert!(matches!(segs[2], Segment::Text(t) if out.s(t) == "ん"));
    }

    #[test]
    fn nested_standalone_gaiji_reports_one_unresolved_diagnostic() {
        run!(out, "漢《［＃「架空」、未知コード］》");
        assert!(matches!(
            out.diagnostics.as_slice(),
            [Diagnostic::UnresolvedGaiji { .. }]
        ));
    }

    #[test]
    fn top_level_gaiji_resolves() {
        run!(out, "※［＃「木＋吶のつくり」、第3水準1-85-54］");
        let Node::Gaiji(g) = out.only_aozora() else {
            panic!("expected Gaiji");
        };
        assert_eq!(out.s(g.hint), "木＋吶のつくり");
        assert!(g.resolve(&out.store).is_some());
    }

    #[test]
    fn forward_bouten_reclaims_adjacent_literal() {
        run!(out, "青空［＃「青空」に傍点］");
        let Node::Format(f) = out.only_aozora() else {
            panic!("expected Format(Bouten)");
        };
        assert_eq!(
            f.attr,
            ForwardAttr::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
        );
        assert_eq!(out.plain(f.target), Some("青空"));
        assert_eq!(f.origin, ForwardOrigin::Reclaimed);
    }

    #[test]
    fn paired_container_emits_open_and_close() {
        run!(out, "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::BlockOpen(_)))
        );
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::BlockClose(_)))
        );
    }

    #[test]
    fn kaeriten_is_classified() {
        run!(out, "天［＃（レ）］");
        let has_kaeriten = out
            .spans
            .iter()
            .any(|s| matches!(s.kind, SpanKind::Aozora(Node::Kaeriten(_))));
        assert!(has_kaeriten, "expected a Kaeriten span: {:?}", out.spans);
    }

    #[test]
    fn kaeriten_base_suppresses_unpaired_diagnostic() {
        run!(out, "漢［＃一］字［＃二］");
        assert!(
            !out.diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic, Diagnostic::BracketedKaeritenNoPair { .. })),
            "paired ladder diagnostics: {:?}",
            out.diagnostics,
        );
    }

    #[test]
    fn unclosed_nested_pair_removes_its_matching_frame_entry() {
        let mut alloc = Allocator::new();
        let mut stream = ClassifyStream::new(iter::empty(), "", 0, &mut alloc);
        stream.frame = Some(Frame {
            body: smallvec::smallvec![
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    span: Span::new(0, 1),
                },
                PairEvent::PairOpen {
                    kind: PairKind::Ruby,
                    span: Span::new(1, 2),
                },
            ],
            links: smallvec::smallvec![u32::MAX, u32::MAX],
            inner_stack: smallvec::smallvec![(PairKind::Bracket, 0), (PairKind::Ruby, 1)],
            gaiji_refmark: None,
        });

        let outcome = stream.append_to_frame(PairEvent::Unclosed {
            kind: PairKind::Ruby,
            span: Span::new(1, 2),
        });

        assert_eq!(outcome, FrameOutcome::Open);
        assert_eq!(
            stream
                .frame
                .as_ref()
                .expect("active frame")
                .inner_stack
                .as_slice(),
            &[(PairKind::Bracket, 0)],
        );
    }

    #[test]
    fn unknown_annotation_is_directive_not_bare_bracket() {
        run!(out, "［＃まったく未知の注記です］");
        let Node::Directive(d) = out.only_aozora() else {
            panic!("expected a Directive, got {:?}", out.only_aozora());
        };
        assert_eq!(d.kind, DirectiveKind::Editorial);
        assert!(out.s(d.raw).starts_with("［＃"));
    }

    #[test]
    fn page_break_and_section_break_classified() {
        run!(out, "［＃改ページ］");
        assert!(matches!(out.only_aozora(), Node::PageBreak));
        run!(out2, "［＃改丁］");
        assert!(matches!(
            out2.only_aozora(),
            Node::SectionBreak(SectionKind::Kaicho)
        ));
    }

    #[test]
    fn angle_quote_classified() {
        run!(out, "≪重要≫");
        let Node::AngleQuote(d) = out.only_aozora() else {
            panic!("expected AngleQuote, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(d.content), Some("重要"));
    }

    #[test]
    fn unresolved_gaiji_emits_diagnostic() {
        run!(out, "※［＃「謎の字」、未知の注記］");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnresolvedGaiji { .. })),
            "expected an UnresolvedGaiji diagnostic, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn spans_tile_source_contiguously() {
        run!(out, "前｜青梅《おうめ》後");
        let mut cursor = 0u32;
        for span in &out.spans {
            assert_eq!(span.source_span.start, cursor, "gap before {span:?}");
            cursor = span.source_span.end;
        }
    }

    #[test]
    fn ruby_helper_boundaries_are_exact() {
        let inner = Span::new(3, 6);
        let events = [
            PairEvent::Text {
                range: Span::new(0, 3),
            },
            PairEvent::PairOpen {
                kind: PairKind::Ruby,
                span: inner,
            },
            PairEvent::PairClose {
                kind: PairKind::Ruby,
                span: Span::new(9, 12),
            },
        ];
        assert_eq!(
            first_nested_ruby_open(&events, 0, events.len()),
            Some(inner)
        );
        assert_eq!(first_nested_ruby_open(&events, 0, 1), None);

        assert_eq!(
            empty_explicit_ruby_span(Some(3), 10, Span::new(20, 23), Span::new(23, 26),),
            Some(Span::new(13, 26)),
        );
        assert_eq!(
            empty_explicit_ruby_span(Some(3), 10, Span::new(20, 23), Span::new(24, 27),),
            None,
        );
        assert_eq!(
            empty_explicit_ruby_span(None, 10, Span::new(20, 23), Span::new(23, 26)),
            None,
        );
    }

    impl TestClassifyOutput {
        fn plain_count(&self) -> usize {
            self.spans
                .iter()
                .filter(|s| s.kind == SpanKind::Plain)
                .count()
        }

        fn only_aozora_span(&self) -> &ClassifiedSpan {
            let mut found: Option<&ClassifiedSpan> = None;
            for span in &self.spans {
                if matches!(span.kind, SpanKind::Aozora(_)) {
                    assert!(
                        found.is_none(),
                        "more than one Aozora span: {:?}",
                        self.spans
                    );
                    found = Some(span);
                }
            }
            found.unwrap_or_else(|| panic!("no Aozora span in {:?}", self.spans))
        }

        fn aozora_count(&self) -> usize {
            self.spans
                .iter()
                .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
                .count()
        }
    }

    #[test]
    fn gaiji_base_ruby_pins_span_and_reading() {
        run!(out, "あ※［＃「ほ」、第3水準1-85-54］《みは》");
        let span = out.only_aozora_span();
        assert_eq!(
            span.source_span.start, 3,
            "gaiji-base ruby must start at the `※` (byte 3): {:?}",
            out.spans
        );
        let SpanKind::Aozora(node) = span.kind else {
            unreachable!("filtered to Aozora above");
        };
        let Node::Ruby(r) = node else {
            panic!("expected a gaiji-base Ruby, got {node:?}");
        };
        assert_eq!(out.plain(r.reading), Some("みは"));
    }

    #[test]
    fn adjacent_gaiji_run_forms_single_ruby() {
        run!(
            out,
            "※［＃「ほ」、第3水準1-85-54］※［＃「ほ」、第3水準1-85-54］《かい》"
        );
        assert_eq!(
            out.aozora_count(),
            1,
            "the two-gaiji ateji base must fold into a single ruby span: {:?}",
            out.spans
        );
        assert!(
            matches!(out.only_aozora(), Node::Ruby(_)),
            "expected a Ruby, got {:?}",
            out.only_aozora()
        );
    }

    #[test]
    fn explicit_ruby_is_a_single_span() {
        run!(out, "｜青梅《おうめ》");
        assert_eq!(
            out.spans.len(),
            1,
            "explicit ruby must be one span (no stray `｜` plain / empty span): {:?}",
            out.spans
        );
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("青梅"));
        assert_eq!(out.plain(r.reading), Some("おうめ"));
    }

    #[test]
    fn literal_bracket_preserves_nested_ruby() {
        run!(out, "［注｜糠栗《コウリツ》］");
        assert!(
            out.spans
                .iter()
                .any(|span| matches!(span.kind, SpanKind::Aozora(Node::Ruby(_)))),
            "{:?}",
            out.spans
        );
    }

    #[test]
    fn interior_forward_bouten_splices_detached_decoration() {
        run!(out, "青空だ。［＃「青空」に傍点］");
        let deco = out
            .spans
            .iter()
            .find(|s| s.source_span == Span::new(0, 6))
            .unwrap_or_else(|| panic!("no span at [0,6): {:?}", out.spans));
        let SpanKind::Aozora(Node::Format(f)) = deco.kind else {
            panic!(
                "expected a spliced Format decoration at [0,6): {:?}",
                deco.kind
            );
        };
        assert_eq!(f.origin, ForwardOrigin::Detached);
        assert_eq!(out.plain(f.target), Some("青空"));
    }

    #[test]
    fn top_level_newline_span_covers_one_byte() {
        run!(out, "a\nb");
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].source_span, Span::new(1, 2));
    }

    #[test]
    fn streamed_newline_span_covers_one_byte() {
        run!(out, "「a\nb」");
        let nl = out
            .spans
            .iter()
            .find(|s| s.kind == SpanKind::Newline)
            .unwrap_or_else(|| panic!("no Newline span: {:?}", out.spans));
        assert_eq!(nl.source_span, Span::new(4, 5));
    }

    #[test]
    fn nested_quote_depth_keeps_single_plain_run() {
        run!(out, "「A「B」「C」」");
        assert_eq!(
            out.plain_count(),
            1,
            "nested quotes must stream as one plain run: {:?}",
            out.spans
        );
    }

    #[test]
    fn separate_quotes_split_the_plain_run() {
        run!(out, "「A」「B」");
        assert_eq!(
            out.plain_count(),
            2,
            "two separate quotes must produce two plain runs: {:?}",
            out.spans
        );
    }

    #[test]
    fn tortoise_close_does_not_end_quote_stream() {
        run!(out, "「〔A〕「B」C」");
        assert_eq!(
            out.plain_count(),
            1,
            "a nested tortoise close must not end the quote stream: {:?}",
            out.spans
        );
    }

    #[test]
    fn tortoise_open_does_not_bump_quote_depth() {
        run!(out, "「A〔B〕」「C」");
        assert_eq!(
            out.plain_count(),
            2,
            "a nested tortoise open must not bump quote depth: {:?}",
            out.spans
        );
    }

    #[test]
    fn held_bar_before_gaiji_is_its_own_plain_span() {
        run!(out, "元｜※［＃「ほ」、第3水準1-85-54］");
        assert!(
            out.spans
                .iter()
                .any(|s| s.kind == SpanKind::Plain && s.source_span == Span::new(3, 6)),
            "the held `｜` must re-emit as its own Plain [3,6): {:?}",
            out.spans
        );
    }

    #[test]
    fn no_bar_before_gaiji_emits_no_empty_span() {
        run!(out, "元※［＃「ほ」、第3水準1-85-54］");
        assert!(
            out.spans
                .iter()
                .all(|s| s.source_span.start != s.source_span.end),
            "no zero-length span should be emitted: {:?}",
            out.spans
        );
    }

    #[test]
    fn nested_annotation_in_reading_is_a_directive_segment() {
        run!(out, "｜日本《に［＃ママ］ん》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby, got {:?}", out.only_aozora());
        };
        let reading = out.contents(r.reading);
        let [Content::Segments(seg_range)] = reading[..] else {
            panic!("expected a Segments reading, got {reading:?}");
        };
        let segs = out.store.resolve_seg_range(seg_range).to_vec();
        assert_eq!(segs.len(), 3, "expected Text/Directive/Text: {segs:?}");
        assert!(
            matches!(segs[1], Segment::Directive(_)),
            "middle segment must be the nested annotation: {segs:?}"
        );
    }

    #[test]
    fn implicit_ruby_base_is_trailing_same_class_run() {
        run!(out, "お漢字《かんじ》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("漢字"));
        assert_eq!(out.plain(r.reading), Some("かんじ"));
    }

    #[test]
    fn orphan_refmark_in_quote_keeps_tiling() {
        run!(out, "「駄目《だめ》※あ」");
        let mut cursor = 0u32;
        for span in &out.spans {
            assert_eq!(span.source_span.start, cursor, "tiling gap before {span:?}");
            cursor = span.source_span.end;
        }
    }

    #[test]
    fn gaiji_in_quote_span_covers_refmark() {
        run!(out, "「※［＃「ほ」、第3水準1-85-54］」");
        let span = out.only_aozora_span();
        assert!(
            matches!(span.kind, SpanKind::Aozora(Node::Gaiji(_))),
            "expected a Gaiji, got {:?}",
            span.kind
        );
        assert_eq!(
            span.source_span.start, 3,
            "the gaiji must consume the held `※` (byte 3): {:?}",
            out.spans
        );
    }

    #[test]
    fn top_level_gaiji_is_a_single_span() {
        run!(out, "※［＃「木＋吶のつくり」、第3水準1-85-54］");
        assert_eq!(
            out.spans.len(),
            1,
            "top-level gaiji must consume its `※` into one span: {:?}",
            out.spans
        );
        assert!(matches!(out.only_aozora(), Node::Gaiji(_)));
    }

    #[test]
    fn replayed_refmark_is_not_held_for_next_bracket() {
        run!(out, "［※］［＃「ほ」、第3水準1-85-54］");
        let span = out.only_aozora_span();
        assert!(
            matches!(span.kind, SpanKind::Aozora(Node::Gaiji(_))),
            "expected a standalone Gaiji, got {:?}",
            span.kind
        );
        assert_eq!(
            span.source_span.start, 9,
            "the second bracket's gaiji must start at byte 9, not absorb the \
             earlier replayed `※`: {:?}",
            out.spans
        );
    }
}
