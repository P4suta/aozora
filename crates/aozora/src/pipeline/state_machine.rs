#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "parser state and sanitized source bounds are internal invariants"
)]

//! Type-state lex pipeline.
//!
//! `Pipeline<'src, S>` makes the lex stage order enforceable at compile time.
//! The state markers [`Source`], [`Sanitized`], [`Tokenized`], [`Paired`] track
//! which stages have run; methods consume `self` and return the next state.
//! Calling `.pair()` on a `Source` is a type error; calling `.tokenize()` twice
//! is a type error; etc.
//!
//! # Two entry shapes
//!
//! - [`Pipeline::run_to_completion`] — one-shot, equivalent to [`crate::pipeline::lex`].
//!   Used by `Document::snapshot` and the FFI / WASM / Python drivers.
//! - [`Pipeline::new`] → `.sanitize()` → `.tokenize()` → `.pair()` →
//!   `.build()` — explicit chain. Use for inspection / instrumentation: each
//!   intermediate state exposes accessors (`.sanitized_text()`, `.tokens()`,
//!   `.events()`, `.diagnostics()`) so callers can probe the partial output
//!   without re-running the pipeline.
//!
//! # State carries its own payload
//!
//! Each state marker is a field-bound struct holding exactly the stage outputs
//! it has produced (`Sanitized` carries the sanitized `String`; `Tokenized`
//! adds the token `Vec`; …). Reading `.sanitized_text()` from
//! `Pipeline<'_, Sanitized>` is a field projection on the state struct — no
//! `Option::expect` lives in production code.
//!
//! # Owned, arena-free
//!
//! Every stage operates on owned data: the sanitized text is an owned `String`,
//! the token / event lists are `Vec`s, and the classify stage builds the owned
//! AST directly into an
//! [`Allocator`]'s
//! [`NodeStore`], which threads straight into
//! the returned [`LexOutput`]. There is no bumpalo arena.
//!
//! # Why `build` is the terminal transition
//!
//! The classify stage requires `&mut Allocator`. We collapse the classify
//! stage + the normalize fold into a single terminal `.build()` call —
//! inspection up through `Paired` works freely; the final pass is atomic.

use core::ops::Range;
use smallvec::SmallVec;
use std::sync::Arc;

use crate::encoding::gaiji::Resolved;
use crate::pipeline::lexer::{
    ClassifiedSpan, PairEvent, SpanKind, Token, classify_range, pair, sanitize, tokenize,
};
use crate::spec::{Diagnostic, PairLink};

use crate::syntax::alloc::Allocator;
use crate::syntax::ast::{
    ContainerPair, Content, ContentRange, ForwardFormat, ForwardPayload, Gaiji, LexOutput, Node,
    NodeStore, RegionOutput, Registry, Ruby, SanitizedText, Segment, content_ruby_base_class,
    gaiji_ruby_base_class, node_is_content_segment,
};
use crate::syntax::format::ForwardOrigin;
use crate::syntax::{
    ForwardAttr, RegionClose, RegionFormat, RubyBaseClass, RubySide, Span, ruby_base_class,
};

use crate::pipeline::fold::{Normalizer, Recorder};

#[derive(Clone, Copy)]
enum PlannedSegment {
    Text(Span),
    Ready(Segment),
}

type SegmentPlan = SmallVec<[PlannedSegment; 8]>;
type MaterializedSegments = SmallVec<[Segment; 8]>;

#[derive(Clone, Copy)]
struct StructuredForwardCandidate {
    idx: usize,
    format: ForwardFormat,
}

#[derive(Clone, Copy)]
struct StructuredRubyCandidate {
    idx: usize,
    ruby: Ruby,
}

// =====================================================================
// State markers (field-bound — each state carries the stage output it is
// responsible for).
// =====================================================================

/// Initial state — no stage has run yet.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source;

/// The sanitize stage has run; the sanitized text is owned.
#[derive(Debug, Clone)]
pub(crate) struct Sanitized {
    sanitized_text: SanitizedText,
    source_unchanged: bool,
}

/// The tokenize stage has run; the token list is materialised.
#[derive(Debug)]
pub(crate) struct Tokenized {
    sanitized_text: SanitizedText,
    source_unchanged: bool,
    tokens: Vec<Token>,
}

/// The pair stage has run; the event list and the resolved (open, close) link
/// side-table are materialised.
#[derive(Debug)]
pub(crate) struct Paired {
    sanitized_text: SanitizedText,
    source_unchanged: bool,
    events: Vec<PairEvent>,
    links: Vec<PairLink>,
}

// =====================================================================
// Pipeline
// =====================================================================

/// Type-state lex pipeline. Each state's transition method consumes `self`,
/// materialises its stage output into the next state struct, and returns a new
/// pipeline in the next state.
#[derive(Debug)]
pub(crate) struct Pipeline<S> {
    source: Arc<str>,
    diagnostics: Vec<Diagnostic>,
    state: S,
}

// ---------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------

impl Pipeline<Source> {
    /// Wrap a source string for type-state-driven lex. The sanitize stage has
    /// not yet run; only `source` is set.
    #[must_use]
    pub(crate) fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            diagnostics: Vec::new(),
            state: Source,
        }
    }

    /// One-shot driver: run every stage and return the final [`LexOutput`].
    /// Equivalent to [`crate::pipeline::lex`].
    #[must_use]
    pub(crate) fn run_to_completion(source: impl Into<Arc<str>>) -> LexOutput {
        Self::new(source).sanitize().tokenize().pair().build()
    }

    pub(crate) fn run_region(source: &str, range: Range<usize>) -> Option<RegionOutput> {
        let range_start = range.start;
        let range_end = range.end;
        let region = source.get(range)?;
        let start = u32::try_from(range_start).ok()?;
        let end = u32::try_from(range_end).ok()?;
        let tokens = tokenize(region)
            .map(|token| shift_token(&token, start))
            .collect::<Option<Vec<_>>>()?;
        let mut pair_stream = pair(tokens.into_iter());
        let events = (&mut pair_stream).collect();
        let diagnostics = pair_stream.take_diagnostics();
        let links = pair_stream.take_links();
        build_paired(region, Some(source), events, links, diagnostics, start, end)
            .map(BuildOutput::into_region)
    }

    /// Borrow the original source text.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    /// Run the sanitize stage, materialising the sanitized text as an owned
    /// `String`.
    #[must_use]
    pub(crate) fn sanitize(mut self) -> Pipeline<Sanitized> {
        let out = sanitize(&self.source);
        self.diagnostics.extend(out.diagnostics);
        let source_unchanged = out.source_unchanged;
        let sanitized_text = if source_unchanged {
            SanitizedText::shared(Arc::clone(&self.source))
        } else {
            SanitizedText::owned(out.text.into_owned())
        };
        Pipeline {
            source: self.source,
            diagnostics: self.diagnostics,
            state: Sanitized {
                sanitized_text,
                source_unchanged,
            },
        }
    }
}

// ---------------------------------------------------------------------
// Sanitized
// ---------------------------------------------------------------------

impl Pipeline<Sanitized> {
    /// Sanitized text.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn sanitized_text(&self) -> &str {
        &self.state.sanitized_text
    }

    /// Diagnostics accumulated through the sanitize stage.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run the tokenize stage, materialising the token list.
    #[must_use]
    pub(crate) fn tokenize(self) -> Pipeline<Tokenized> {
        let tokens: Vec<Token> = tokenize(&self.state.sanitized_text).collect();
        Pipeline {
            source: self.source,
            diagnostics: self.diagnostics,
            state: Tokenized {
                sanitized_text: self.state.sanitized_text,
                source_unchanged: self.state.source_unchanged,
                tokens,
            },
        }
    }
}

// ---------------------------------------------------------------------
// Tokenized
// ---------------------------------------------------------------------

impl Pipeline<Tokenized> {
    /// Borrow the materialised token list. Useful for instrumentation.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn tokens(&self) -> &[Token] {
        &self.state.tokens
    }

    /// Run the pair stage, materialising a paired-event stream and the resolved
    /// link side-table. The pair stage's diagnostics are drained into the
    /// pipeline's diagnostic accumulator immediately.
    #[must_use]
    pub(crate) fn pair(mut self) -> Pipeline<Paired> {
        let Tokenized {
            sanitized_text,
            source_unchanged,
            tokens,
        } = self.state;
        let mut pair_stream = pair(tokens.into_iter());
        let events: Vec<PairEvent> = (&mut pair_stream).collect();
        self.diagnostics.extend(pair_stream.take_diagnostics());
        let links = pair_stream.take_links();
        Pipeline {
            source: self.source,
            diagnostics: self.diagnostics,
            state: Paired {
                sanitized_text,
                source_unchanged,
                events,
                links,
            },
        }
    }
}

// ---------------------------------------------------------------------
// Paired (terminal)
// ---------------------------------------------------------------------

impl Pipeline<Paired> {
    /// Borrow the materialised pair-event list. Useful for inspection before
    /// `.build()`.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn events(&self) -> &[PairEvent] {
        &self.state.events
    }

    /// Borrow the resolved (open, close) pair side-table.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn links(&self) -> &[PairLink] {
        &self.state.links
    }

    /// Drive the classify stage + the owned normalizer fold and return the final
    /// [`LexOutput`]. Terminal transition.
    ///
    /// Diagnostics are returned in source-span order.
    ///
    /// # Panics
    ///
    /// Panics if the sanitized source exceeds `u32::MAX` bytes (the lexer's
    /// `Span` width contract). In practice unreachable.
    #[must_use]
    pub(crate) fn build(self) -> LexOutput {
        let Paired {
            sanitized_text,
            source_unchanged,
            events,
            links,
        } = self.state;
        let Ok(end) = u32::try_from(sanitized_text.len()) else {
            panic!("sanitized source exceeds the span width");
        };
        let Some(output) = build_paired(
            &sanitized_text,
            None,
            events,
            links,
            self.diagnostics,
            0,
            end,
        ) else {
            unreachable!("full parse spans stay inside the source");
        };
        output.into_lex(sanitized_text, source_unchanged)
    }
}

fn shift_token(token: &Token, by: u32) -> Option<Token> {
    let by = i64::from(by);
    Some(match *token {
        Token::Text { range } => Token::Text {
            range: range.shifted(by),
        },
        Token::Trigger { kind, span } => Token::Trigger {
            kind,
            span: span.shifted(by),
        },
        Token::Newline { pos } => Token::Newline {
            pos: u32::try_from(i64::from(pos) + by).ok()?,
        },
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the parsed region and its already-materialized stage products are independent inputs"
)]
fn build_paired(
    sanitized_text: &str,
    source_context: Option<&str>,
    events: Vec<PairEvent>,
    links: Vec<PairLink>,
    mut diagnostics: Vec<Diagnostic>,
    region_start: u32,
    region_end: u32,
) -> Option<BuildOutput> {
    let full_source = source_context.is_none();
    let source = source_context.unwrap_or(sanitized_text);
    let mut alloc = Allocator::new();
    let (normalized, recorder, container_pairs, classify_diagnostics, norm_diagnostics, store) = {
        let mut normalizer = Normalizer::new(source, events.len());
        let mut events_iter = events.into_iter();
        let mut classify_stream = classify_range(&mut events_iter, source, region_end, &mut alloc);
        let spans: Vec<ClassifiedSpan> = (&mut classify_stream).collect();
        let mut classify_diagnostics: Vec<Diagnostic> = classify_stream.take_diagnostics();
        drop(classify_stream);
        let (lowered, decorated_forward) = lower_spans(spans, source, &mut alloc);
        if !decorated_forward.is_empty() {
            classify_diagnostics.retain(|diagnostic| {
                !(matches!(diagnostic, Diagnostic::ForwardReferentNotStylable { .. })
                    && decorated_forward.contains(&diagnostic.span()))
            });
        }
        for span in &lowered {
            normalizer.emit(span);
        }
        let Normalizer {
            out,
            recorder,
            container_pairs,
            diagnostics: norm_diagnostics,
            ..
        } = normalizer;
        let store = alloc.into_store();
        (
            out,
            recorder,
            container_pairs,
            classify_diagnostics,
            norm_diagnostics,
            store,
        )
    };

    diagnostics.extend(classify_diagnostics);
    diagnostics.extend(norm_diagnostics);
    let output = BuildOutput {
        normalized,
        recorder,
        container_pairs,
        store,
        links,
        diagnostics,
    };
    if full_source {
        return Some(output);
    }
    build_region_output(output, region_start, region_end)
}

struct BuildOutput {
    normalized: String,
    recorder: Recorder,
    container_pairs: Vec<ContainerPair>,
    store: NodeStore,
    links: Vec<PairLink>,
    diagnostics: Vec<Diagnostic>,
}

impl BuildOutput {
    fn into_lex(self, sanitized_text: SanitizedText, source_unchanged: bool) -> LexOutput {
        let registry = Registry::from_source_nodes(&self.recorder.source_nodes);
        LexOutput::new(
            self.normalized,
            sanitized_text,
            source_unchanged,
            registry,
            self.diagnostics,
            self.links,
            self.recorder.source_nodes,
            self.container_pairs,
            self.store,
        )
    }

    fn into_region(self) -> RegionOutput {
        RegionOutput {
            normalized: self.normalized,
            diagnostics: self.diagnostics,
            pairs: self.links,
            source_nodes: self.recorder.source_nodes,
            container_pairs: self.container_pairs,
            store: self.store,
        }
    }
}

fn build_region_output(
    mut build: BuildOutput,
    region_start: u32,
    region_end: u32,
) -> Option<BuildOutput> {
    if build
        .recorder
        .source_nodes
        .iter()
        .any(|entry| entry.source_span.start < region_start || entry.source_span.end > region_end)
        || build
            .links
            .iter()
            .any(|pair| pair.open.start < region_start || pair.close.end > region_end)
        || build.diagnostics.iter().any(|diagnostic| {
            diagnostic.span().start < region_start || diagnostic.span().end > region_end
        })
    {
        return None;
    }

    if region_start != 0 {
        let shift = -i64::from(region_start);
        for entry in &mut build.recorder.source_nodes {
            entry.source_span = entry.source_span.shifted(shift);
        }
        for pair in &mut build.links {
            *pair = PairLink::new(
                pair.kind,
                pair.open.shifted(shift),
                pair.close.shifted(shift),
            );
        }
        build.diagnostics = build
            .diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.shifted(shift))
            .collect();
    }
    Some(build)
}

/// NORMALIZE (lowering) pass over the materialized classified-span list.
///
/// This is the seam the normalization waist is built on. It performs the
/// source-byte **drop-superset** the streaming window did: when a later span's
/// source span is a proper superset of an earlier one — a backward pull-back,
/// e.g. a promoted 大/中/小 heading reclaiming its referent line `序章\n`, or a
/// forward node reclaiming its predecessor literal — the subsumed earlier span
/// is dropped, so the normalizer (which appends in source order) does not emit
/// the reclaimed text twice. This overlap-truncate cured the #180 round-trip
/// pathology; the surviving [`ForwardOrigin`] on each forward leaf is necessary
/// provenance (#202).
///
/// Returns the lowered spans and the set of forward-directive spans decorated
/// during lowering, so the builder can suppress their stale
/// `forward_referent_not_stylable` warnings.
fn lower_spans(
    spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> (Vec<ClassifiedSpan>, Vec<Span>) {
    // Phase 0: resolve forward heading hints whose referent is the bare line
    // directly above the directive into promoted `Heading` nodes.
    let mut spans = promote_headings(spans, source, alloc);
    let mut decorated = merge_structured_content(&mut spans, source, alloc);
    let mut out_len = 0;
    for read in 0..spans.len() {
        let span = spans[read].clone();
        while out_len != 0 {
            let back = &spans[out_len - 1];
            let (bs, be) = (back.source_span.start, back.source_span.end);
            let back_is_plain = matches!(back.kind, SpanKind::Plain);
            let (ss, se) = (span.source_span.start, span.source_span.end);
            if ss <= bs && be <= se && (ss < bs || se > be) {
                out_len = out_len
                    .checked_sub(1)
                    .expect("out_len is non-zero inside the guarded loop");
            } else if back_is_plain && bs < ss {
                spans[out_len - 1].source_span.end = be.min(ss);
                break;
            } else {
                break;
            }
        }
        spans[out_len] = span;
        out_len += 1;
    }
    spans.truncate(out_len);
    // Second phase: fold S4-foldable inline-range emphasis into forward leaves.
    let mut out = fold_inline_emphasis(spans, source, alloc);
    // Third phase: apply a declined forward emphasis onto a preceding ruby base
    // it uniquely names (#384).
    decorated.extend(decorate_ruby_bases(&mut out, source, alloc.store()));
    (out, decorated)
}

fn merge_structured_content(
    spans: &mut [ClassifiedSpan],
    source: &str,
    alloc: &mut Allocator,
) -> Vec<Span> {
    let mut decorated = Vec::new();
    for idx in 0..spans.len() {
        match spans[idx].kind {
            SpanKind::Aozora(Node::Ruby(ruby)) => {
                let candidate = StructuredRubyCandidate { idx, ruby };
                merge_structured_ruby_base(spans, candidate, source, alloc);
            }
            SpanKind::Aozora(Node::Format(format)) => {
                let candidate = StructuredForwardCandidate { idx, format };
                if let Some(span) = merge_structured_forward_target(spans, candidate, source, alloc)
                {
                    decorated.push(span);
                }
            }
            _ => {}
        }
    }
    decorated
}

fn merge_structured_forward_target(
    spans: &mut [ClassifiedSpan],
    candidate: StructuredForwardCandidate,
    source: &str,
    alloc: &mut Allocator,
) -> Option<Span> {
    let StructuredForwardCandidate {
        idx: format_idx,
        format,
    } = candidate;
    if !structured_forward_can_merge(format.origin, format.payload) {
        return None;
    }
    let [Content::Plain(quoted_target)] = alloc.store().resolve_content_range(format.target) else {
        return None;
    };
    let quoted_target = *quoted_target;
    let planned = {
        let store = alloc.store();
        let target = store.resolve_str(quoted_target);
        match spans[..format_idx].last()?.kind {
            SpanKind::Plain => {
                let text = spans[format_idx - 1].source_span.slice(source);
                if !plain_edge_matches_target(text, target) {
                    return None;
                }
            }
            SpanKind::Aozora(Node::Gaiji(_) | Node::AngleQuote(_) | Node::Kaeriten(_)) => {}
            _ => return None,
        }
        structured_forward_prefix(&spans[..format_idx], target, source, store)
    };
    let (source_start, plan) = planned?;
    let segments = materialize_segments(&plan, source, alloc);
    let content = alloc.content_segments(&segments);
    let Node::Format(mut merged) =
        alloc.forward_format(format.attr, content, ForwardOrigin::Reclaimed)
    else {
        unreachable!("forward format allocator returns a format node");
    };
    merged.payload = ForwardPayload::QuotedTarget(quoted_target);
    let directive_span = spans[format_idx].source_span;
    spans[format_idx].kind = SpanKind::Aozora(Node::Format(merged));
    spans[format_idx].source_span.start = source_start;
    Some(directive_span)
}

fn structured_forward_can_merge(origin: ForwardOrigin, payload: ForwardPayload) -> bool {
    matches!(
        origin,
        ForwardOrigin::SelfContained | ForwardOrigin::Referenced
    ) && matches!(payload, ForwardPayload::None | ForwardPayload::NestedSource)
}

fn plain_edge_matches_target(text: &str, target: &str) -> bool {
    target.ends_with(text) || text.ends_with(target)
}

fn structured_forward_prefix(
    spans: &[ClassifiedSpan],
    target: &str,
    source: &str,
    store: &NodeStore,
) -> Option<(u32, SegmentPlan)> {
    let mut remaining = target;
    let mut reversed = SegmentPlan::new();
    let mut source_start = spans.last()?.source_span.end;
    let mut cursor = spans.len();
    let mut contains_structured = false;

    while !remaining.is_empty() {
        let previous_idx = cursor.checked_sub(1)?;
        let previous = &spans[previous_idx];
        if previous.source_span.end != source_start {
            return None;
        }
        match previous.kind {
            SpanKind::Plain => {
                let text = previous.source_span.slice(source);
                if remaining.ends_with(text) {
                    reversed.push(PlannedSegment::Text(previous.source_span));
                    remaining = &remaining[..remaining.len() - text.len()];
                    source_start = previous.source_span.start;
                    cursor = previous_idx;
                } else if text.ends_with(remaining) {
                    let consumed = remaining;
                    source_start = previous.source_span.end.saturating_sub(
                        u32::try_from(consumed.len()).expect("source span offset fits u32"),
                    );
                    reversed.push(PlannedSegment::Text(Span::new(
                        source_start,
                        previous.source_span.end,
                    )));
                    remaining = "";
                } else {
                    return None;
                }
            }
            SpanKind::Aozora(Node::Gaiji(gaiji)) => {
                let stripped = strip_structured_gaiji_suffix(remaining, gaiji, store)?;
                reversed.push(PlannedSegment::Ready(Segment::Gaiji(gaiji)));
                remaining = stripped;
                source_start = previous.source_span.start;
                cursor = previous_idx;
                contains_structured = true;
            }
            SpanKind::Aozora(node @ (Node::AngleQuote(_) | Node::Kaeriten(_))) => {
                let stripped = strip_structured_forward_node_suffix(remaining, node, store)?;
                reversed.push(PlannedSegment::Ready(Segment::Node(node)));
                remaining = stripped;
                source_start = previous.source_span.start;
                cursor = previous_idx;
                contains_structured = true;
            }
            _ => return None,
        }
    }
    if !contains_structured {
        return None;
    }
    reversed.reverse();
    Some((source_start, reversed))
}

fn strip_structured_gaiji_suffix<'a>(
    text: &'a str,
    gaiji: Gaiji,
    store: &NodeStore,
) -> Option<&'a str> {
    match gaiji.resolve(store) {
        Some(Resolved::Char(ch)) => text.strip_suffix(ch),
        Some(Resolved::Multi(value)) => text.strip_suffix(value),
        None => text.strip_suffix(store.resolve_str(gaiji.hint)),
    }
}

fn strip_structured_forward_node_suffix<'a>(
    text: &'a str,
    node: Node,
    store: &NodeStore,
) -> Option<&'a str> {
    match node {
        Node::AngleQuote(quote) => {
            let text = text.strip_suffix('》')?;
            let text = strip_structured_forward_content_suffix(text, quote.content, store)?;
            text.strip_suffix('《')
        }
        Node::Kaeriten(kaeriten) => text.strip_suffix(store.resolve_str(kaeriten.mark)),
        _ => None,
    }
}

fn strip_structured_forward_content_suffix<'a>(
    mut text: &'a str,
    range: ContentRange,
    store: &NodeStore,
) -> Option<&'a str> {
    for content in store.resolve_content_range(range).iter().rev() {
        match *content {
            Content::Plain(id) => text = text.strip_suffix(store.resolve_str(id))?,
            Content::Segments(segments) => {
                for segment in store.resolve_seg_range(segments).iter().rev() {
                    match *segment {
                        Segment::Text(id) => text = text.strip_suffix(store.resolve_str(id))?,
                        Segment::Gaiji(gaiji) => {
                            text = strip_structured_gaiji_suffix(text, gaiji, store)?;
                        }
                        Segment::Directive(_) | Segment::Node(_) => return None,
                    }
                }
            }
        }
    }
    Some(text)
}

fn merge_structured_ruby_base(
    spans: &mut [ClassifiedSpan],
    candidate: StructuredRubyCandidate,
    source: &str,
    alloc: &mut Allocator,
) {
    let StructuredRubyCandidate {
        idx: ruby_idx,
        ruby,
    } = candidate;
    let ruby_source = spans[ruby_idx].source_span.slice(source);
    if ruby.side != RubySide::Right || ruby_source.starts_with('｜') {
        return;
    }
    let reading_only = ruby_source.starts_with('《');
    let Some((source_start, plan)) = explicit_ruby_prefix(spans, ruby_idx, source, alloc.store())
        .or_else(|| {
            (!reading_only).then(|| implicit_ruby_prefix(spans, ruby_idx, source, alloc.store()))?
        })
    else {
        return;
    };
    if reading_only
        && let Some(base) = alloc.store().content_range_as_plain(ruby.base)
        && plain_plan_matches(&plan, base, source)
    {
        spans[ruby_idx].source_span.start = source_start;
        return;
    }
    let mut prefix = materialize_segments(&plan, source, alloc);
    if !reading_only {
        match alloc.store().resolve_content_range(ruby.base)[0] {
            Content::Plain(id) => prefix.push(Segment::Text(id)),
            Content::Segments(range) => {
                prefix.extend_from_slice(alloc.store().resolve_seg_range(range));
            }
        }
    }
    let reading = alloc.store().resolve_content_range(ruby.reading)[0];
    let merged_base = alloc.content_segments(&prefix);
    let Node::Ruby(mut merged) = alloc.ruby(merged_base, reading) else {
        unreachable!("ruby allocator returns a ruby node");
    };
    merged.base_emphasis = ruby.base_emphasis;
    spans[ruby_idx].kind = SpanKind::Aozora(Node::Ruby(merged));
    spans[ruby_idx].source_span.start = source_start;
}

fn plain_plan_matches(plan: &[PlannedSegment], mut remaining: &str, source: &str) -> bool {
    for segment in plan {
        let PlannedSegment::Text(span) = segment else {
            return false;
        };
        let Some(rest) = remaining.strip_prefix(span.slice(source)) else {
            return false;
        };
        remaining = rest;
    }
    remaining.is_empty()
}

fn explicit_ruby_prefix(
    spans: &[ClassifiedSpan],
    ruby_idx: usize,
    source: &str,
    store: &NodeStore,
) -> Option<(u32, SegmentPlan)> {
    let SpanKind::Aozora(Node::Ruby(ruby)) = spans[ruby_idx].kind else {
        return None;
    };
    let mut reversed = SegmentPlan::new();
    let mut source_start = spans[ruby_idx].source_span.start;
    let mut cursor = ruby_idx;
    let mut contains_structured = content_range_contains_structured(ruby.base, store);
    let reclaim_plain = spans[ruby_idx].source_span.slice(source).starts_with('《');

    while let Some(previous_idx) = cursor.checked_sub(1) {
        let previous = &spans[previous_idx];
        if previous.source_span.end != source_start {
            return None;
        }
        match previous.kind {
            SpanKind::Aozora(Node::Gaiji(gaiji)) => {
                reversed.push(PlannedSegment::Ready(Segment::Gaiji(gaiji)));
                source_start = previous.source_span.start;
                cursor = previous_idx;
                contains_structured = true;
            }
            SpanKind::Aozora(Node::Directive(directive)) => {
                reversed.push(PlannedSegment::Ready(Segment::Directive(directive)));
                source_start = previous.source_span.start;
                cursor = previous_idx;
                contains_structured = true;
            }
            SpanKind::Aozora(node) if node_is_content_segment(node) => {
                reversed.push(PlannedSegment::Ready(Segment::Node(node)));
                source_start = previous.source_span.start;
                cursor = previous_idx;
                contains_structured = true;
            }
            SpanKind::Plain => {
                let text = previous.source_span.slice(source);
                if let Some(bar_start) = text.rfind('｜') {
                    let base_start = bar_start + '｜'.len_utf8();
                    if base_start < text.len() {
                        let text_start = previous.source_span.start.saturating_add(
                            u32::try_from(base_start).expect("source span offset fits u32"),
                        );
                        reversed.push(PlannedSegment::Text(Span::new(
                            text_start,
                            previous.source_span.end,
                        )));
                    }
                    if contains_structured || reclaim_plain {
                        reversed.reverse();
                        return Some((
                            previous.source_span.start.saturating_add(
                                u32::try_from(bar_start).expect("source span offset fits u32"),
                            ),
                            reversed,
                        ));
                    }
                    return None;
                }
                if !text.is_empty() {
                    reversed.push(PlannedSegment::Text(previous.source_span));
                }
                source_start = previous.source_span.start;
                cursor = previous_idx;
            }
            _ => return None,
        }
    }
    None
}

fn implicit_ruby_prefix(
    spans: &[ClassifiedSpan],
    ruby_idx: usize,
    source: &str,
    store: &NodeStore,
) -> Option<(u32, SegmentPlan)> {
    let SpanKind::Aozora(Node::Ruby(ruby)) = spans[ruby_idx].kind else {
        return None;
    };
    let class = content_ruby_base_class(store.resolve_content_range(ruby.base), store)?;
    let mut reversed = SegmentPlan::new();
    let mut source_start = spans[ruby_idx].source_span.start;
    let mut cursor = ruby_idx;
    let mut contains_structured = content_range_contains_structured(ruby.base, store);

    while let Some(previous_idx) = cursor.checked_sub(1) {
        let previous = &spans[previous_idx];
        if previous.source_span.end != source_start {
            break;
        }
        match previous.kind {
            SpanKind::Aozora(Node::Gaiji(gaiji))
                if gaiji_ruby_base_class(gaiji, store) == Some(class) =>
            {
                reversed.push(PlannedSegment::Ready(Segment::Gaiji(gaiji)));
                source_start = previous.source_span.start;
                cursor = previous_idx;
                contains_structured = true;
            }
            SpanKind::Plain => {
                let text = previous.source_span.slice(source);
                let suffix_start = trailing_class_start(text, class);
                if suffix_start == text.len() {
                    break;
                }
                source_start = previous.source_span.start.saturating_add(
                    u32::try_from(suffix_start).expect("source span offset fits u32"),
                );
                reversed.push(PlannedSegment::Text(Span::new(
                    source_start,
                    previous.source_span.end,
                )));
                if suffix_start != 0 {
                    break;
                }
                cursor = previous_idx;
            }
            _ => break,
        }
    }
    if !contains_structured {
        return None;
    }
    reversed.reverse();
    Some((source_start, reversed))
}

fn content_range_contains_structured(range: ContentRange, store: &NodeStore) -> bool {
    store
        .resolve_content_range(range)
        .iter()
        .any(|content| match *content {
            Content::Plain(_) => false,
            Content::Segments(segments) => store
                .resolve_seg_range(segments)
                .iter()
                .any(|segment| !matches!(segment, Segment::Text(_))),
        })
}

fn materialize_segments(
    plan: &[PlannedSegment],
    source: &str,
    alloc: &mut Allocator,
) -> MaterializedSegments {
    plan.iter()
        .map(|planned| match *planned {
            PlannedSegment::Text(span) => alloc.seg_text(span.slice(source)),
            PlannedSegment::Ready(Segment::Node(node)) => alloc.seg_node(node),
            PlannedSegment::Ready(segment) => segment,
        })
        .collect()
}

fn trailing_class_start(text: &str, class: RubyBaseClass) -> usize {
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        if ruby_base_class(ch) != Some(class) {
            break;
        }
        start = idx;
    }
    start
}

/// Whether a forward attribute decorates a whole run as a single emphasis
/// wrapper, so it can style a ruby base (#384). Excludes the sub-character /
/// target-splitting attributes — [`ForwardAttr::AccentDot`] (addresses letters
/// via an interned directive body the ruby cannot carry),
/// [`ForwardAttr::Accent`] (composes a single Latin letter), and
/// [`ForwardAttr::Fraction`] (splits the target on a slash) — which are never
/// meaningful over a kanji base and stay declined.
const fn attr_decorates_ruby_base(attr: ForwardAttr) -> bool {
    !matches!(
        attr,
        ForwardAttr::AccentDot | ForwardAttr::Accent(_) | ForwardAttr::Fraction
    )
}

/// Apply a declined forward emphasis directive (`［＃「X」に傍点/罫囲み/…］`, a
/// [`ForwardOrigin::Referenced`] leaf) onto a preceding ruby whose base is the
/// *unique* referent named `X` (#384). The classifier declines these because a
/// ruby base cannot be pulled into a plain forward leaf (bouten-over-ruby is not
/// representable); instead we set that ruby's `base_emphasis` so the renderer
/// wraps the base in the attribute's emphasis element. The directive leaf stays
/// `Referenced` (serializes the bracket verbatim, renders nothing), so serialize
/// stays byte-identical.
///
/// Uniqueness is load-bearing, not "nearest ruby wins": the target must match
/// **exactly one** preceding ruby base and **no** preceding plain-text run
/// anywhere in the look-back — a plain copy that precedes the ruby (cross-line
/// or same-line, out of the classifier's reset window) is a competing referent,
/// so we decline and keep the honest `forward_referent_not_stylable` warning.
/// Returns the directive spans decorated, so the builder can suppress exactly
/// those warnings.
fn decorate_ruby_bases(out: &mut [ClassifiedSpan], source: &str, store: &NodeStore) -> Vec<Span> {
    let mut decorated: Vec<Span> = Vec::new();
    for idx in 0..out.len() {
        // Fire only on a declined (Referenced) forward directive with a
        // whole-run-decoratable attribute.
        let SpanKind::Aozora(Node::Format(f)) = out[idx].kind else {
            continue;
        };
        if !matches!(f.origin, ForwardOrigin::Referenced) || !attr_decorates_ruby_base(f.attr) {
            continue;
        }
        // A ruby base is a single `Plain` run; a structured target never matches.
        let Some(target) = store.content_range_as_plain(f.target) else {
            continue;
        };
        // Scan the whole look-back for the unique referent.
        let mut ruby_match: Option<usize> = None;
        let mut ambiguous = false;
        for j in 0..idx {
            match &out[j].kind {
                SpanKind::Aozora(Node::Ruby(r)) => {
                    if store.content_range_as_plain(r.base) == Some(target) {
                        if ruby_match.is_some() {
                            ambiguous = true;
                            break;
                        }
                        ruby_match = Some(j);
                    }
                }
                // Any preceding plain run that carries the target text — before
                // the ruby, so invisible to the classifier's reset window — is a
                // competing referent that forces a decline.
                SpanKind::Plain => {
                    let s =
                        &source[out[j].source_span.start as usize..out[j].source_span.end as usize];
                    if s.contains(target) {
                        ambiguous = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if ambiguous {
            continue;
        }
        let Some(ruby_idx) = ruby_match else {
            continue;
        };
        let attr = f.attr;
        let directive_span = out[idx].source_span;
        if let SpanKind::Aozora(Node::Ruby(ref mut r)) = out[ruby_idx].kind {
            r.base_emphasis = Some(attr);
            decorated.push(directive_span);
        }
    }
    decorated
}

/// Byte position where `target` begins, **only if** it is the bare line
/// immediately preceding the `［` at `bracket_start`. `None` → the hint stays
/// inline.
fn find_heading_predecessor_position_at(
    source: &str,
    bracket_start: u32,
    target: &str,
) -> Option<u32> {
    let bytes = source.as_bytes();
    let cutoff = bracket_start as usize;
    if cutoff == 0 || bytes[cutoff - 1] != b'\n' {
        return None;
    }
    let text_end = cutoff - 1;
    let len = target.len();
    if text_end < len {
        return None;
    }
    let candidate_start = text_end - len;
    if &bytes[candidate_start..text_end] != target.as_bytes() {
        return None;
    }
    if candidate_start != 0 && bytes[candidate_start - 1] != b'\n' {
        return None;
    }
    u32::try_from(candidate_start).ok()
}

/// Promote each forward heading hint whose target is the bare line directly
/// above it into a `Heading`, reaching the span back over `target\n` so the
/// superset-drop pass reclaims the referent line.
fn promote_headings(
    mut spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    for span in &mut spans {
        let SpanKind::Aozora(Node::HeadingHint(hint)) = span.kind else {
            continue;
        };
        // Resolve the interned target to an owned string so the `&store` borrow
        // does not overlap the `&mut alloc` builder calls below.
        let target = alloc.store().resolve_str(hint.target).to_owned();
        let Some(referent_start) =
            find_heading_predecessor_position_at(source, span.source_span.start, &target)
        else {
            continue;
        };
        let text = alloc.content_plain(&target);
        span.kind = SpanKind::Aozora(alloc.aozora_heading(hint.level, hint.style, text));
        span.source_span.start = referent_start;
    }
    spans
}

/// The forward-scope attribute an inline-range region folds to.
const fn foldable_inline_attr(region: RegionFormat) -> Option<ForwardAttr> {
    match region {
        RegionFormat::Bold { padded: false } => Some(ForwardAttr::Bold),
        RegionFormat::Gothic { padded: false } => Some(ForwardAttr::Gothic),
        RegionFormat::Italic { padded: false } => Some(ForwardAttr::Italic),
        RegionFormat::Caption { padded: false } => Some(ForwardAttr::Caption),
        RegionFormat::Bouten { kind, position } => Some(ForwardAttr::Bouten { kind, position }),
        RegionFormat::SmallScript(position) => Some(ForwardAttr::SmallScript(position)),
        _ => None,
    }
}

/// An open inline-range marker awaiting its close, with the spans seen since.
struct OpenFrame {
    /// The `BlockOpen` span itself (re-emitted verbatim if the pair does not fold).
    open: ClassifiedSpan,
    /// The open marker's region (drives foldability and the close-match check).
    region: RegionFormat,
    /// Spans between this open and its eventual close, in source order.
    collected: Vec<ClassifiedSpan>,
}

/// Push a finished span onto the innermost open frame, or to `output` at top level.
fn emit_to(stack: &mut [OpenFrame], output: &mut Vec<ClassifiedSpan>, span: ClassifiedSpan) {
    if let Some(top) = stack.last_mut() {
        top.collected.push(span);
    } else {
        output.push(span);
    }
}

/// Fold a matched inline-range pair into a forward leaf, or `None` to keep it as
/// a container.
fn try_fold_inline(
    frame: &OpenFrame,
    close: &ClassifiedSpan,
    source: &str,
    alloc: &mut Allocator,
) -> Option<ClassifiedSpan> {
    let attr = foldable_inline_attr(frame.region)?;
    let SpanKind::BlockClose(close_region) = close.kind else {
        return None;
    };
    if RegionClose::of(frame.region) != close_region {
        return None;
    }
    if frame.collected.is_empty()
        || !frame
            .collected
            .iter()
            .all(|s| matches!(s.kind, SpanKind::Plain))
    {
        return None;
    }
    let mut text = String::new();
    for s in &frame.collected {
        text.push_str(&source[s.source_span.start as usize..s.source_span.end as usize]);
    }
    if text.is_empty() {
        return None;
    }
    let content = alloc.content_plain(&text);
    // An inline-range fold is adjacent by construction: the opener sits right
    // before the enclosed run, so the leaf reclaims its literal.
    let node = alloc.forward_format(attr, content, ForwardOrigin::Reclaimed);
    Some(ClassifiedSpan {
        kind: SpanKind::Aozora(node),
        source_span: Span::new(frame.open.source_span.start, close.source_span.end),
    })
}

/// Fold S4-foldable inline-range emphasis pairs into forward leaves.
fn fold_inline_emphasis(
    spans: Vec<ClassifiedSpan>,
    source: &str,
    alloc: &mut Allocator,
) -> Vec<ClassifiedSpan> {
    let mut output: Vec<ClassifiedSpan> = Vec::with_capacity(spans.len());
    let mut stack: Vec<OpenFrame> = Vec::new();
    for span in spans {
        match span.kind {
            SpanKind::BlockOpen(region) => {
                stack.push(OpenFrame {
                    open: span,
                    region,
                    collected: Vec::new(),
                });
            }
            SpanKind::BlockClose(_) => {
                if let Some(frame) = stack.pop() {
                    if let Some(folded) = try_fold_inline(&frame, &span, source, alloc) {
                        emit_to(&mut stack, &mut output, folded);
                    } else {
                        emit_to(&mut stack, &mut output, frame.open);
                        for c in frame.collected {
                            emit_to(&mut stack, &mut output, c);
                        }
                        emit_to(&mut stack, &mut output, span);
                    }
                } else {
                    output.push(span);
                }
            }
            _ => emit_to(&mut stack, &mut output, span),
        }
    }
    // Flush any unclosed opens (bottom-to-top reconstructs source order).
    for frame in stack {
        output.push(frame.open);
        output.extend(frame.collected);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{NormalizedOffset, PairKind, Sentinel};
    use crate::syntax::ast::{NodeRef, SourceNode};
    use crate::syntax::{BoutenKind, BoutenPosition};

    fn empty_region_build() -> BuildOutput {
        BuildOutput {
            normalized: String::new(),
            recorder: Recorder::default(),
            container_pairs: Vec::new(),
            store: Allocator::new().into_store(),
            links: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn type_state_chain_compiles() {
        let _final = Pipeline::new("｜青梅《おうめ》")
            .sanitize()
            .tokenize()
            .pair()
            .build();
    }

    #[test]
    fn run_to_completion_matches_chain() {
        let chain = Pipeline::new("｜青梅《おうめ》")
            .sanitize()
            .tokenize()
            .pair()
            .build();
        let oneshot = Pipeline::run_to_completion("｜青梅《おうめ》");
        assert_eq!(chain.normalized, oneshot.normalized);
        assert_eq!(chain.sanitized.len(), oneshot.sanitized.len());
        assert_eq!(
            chain.registry.count_kind(Sentinel::Inline),
            oneshot.registry.count_kind(Sentinel::Inline)
        );
    }

    #[test]
    fn implicit_ruby_reclaims_resolved_gaiji_and_plain_text() {
        let source = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》";
        let output = Pipeline::run_to_completion(source);
        assert!(output.diagnostics.is_empty());
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(
            entry.source_span,
            Span::new(0, u32::try_from(source.len()).expect("fixture fits u32"))
        );
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        let [Segment::Gaiji(gaiji), Segment::Text(text)] = output.store.resolve_seg_range(*range)
        else {
            panic!("expected gaiji followed by plain text in the ruby base");
        };
        assert_eq!(
            gaiji.resolve(&output.store).and_then(Resolved::as_char),
            Some('犍')
        );
        assert_eq!(output.store.resolve_str(*text), "陀多");
        assert_eq!(
            output.store.content_range_as_plain(ruby.reading),
            Some("かんだた")
        );
    }

    #[test]
    fn implicit_ruby_extends_a_structured_existing_base() {
        let source = "川※［＃「くさかんむり／弓」、第3水準1-90-62］《せんきゅう》";
        let output = Pipeline::run_to_completion(source);
        assert!(output.diagnostics.is_empty());
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(entry.source_span.slice(source), source);
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        let [Segment::Text(text), Segment::Gaiji(gaiji)] = output.store.resolve_seg_range(*range)
        else {
            panic!("expected text followed by gaiji in the ruby base");
        };
        assert_eq!(output.store.resolve_str(*text), "川");
        assert_eq!(
            gaiji.resolve(&output.store).and_then(Resolved::as_char),
            Some('芎')
        );
        assert_eq!(
            output.store.content_range_as_plain(ruby.reading),
            Some("せんきゅう")
        );
    }

    #[test]
    fn explicit_ruby_reclaims_text_and_gaiji_without_class_constraints() {
        let source = "前｜A※［＃「特のへん＋廴＋聿」、第3水準1-87-71］漢《r》後";
        let output = Pipeline::run_to_completion(source);
        assert!(output.diagnostics.is_empty());
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(
            entry.source_span.slice(source),
            "｜A※［＃「特のへん＋廴＋聿」、第3水準1-87-71］漢《r》"
        );
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        let [
            Segment::Text(first),
            Segment::Gaiji(gaiji),
            Segment::Text(last),
        ] = output.store.resolve_seg_range(*range)
        else {
            panic!("expected text, gaiji, text in the ruby base");
        };
        assert_eq!(output.store.resolve_str(*first), "A");
        assert_eq!(
            gaiji.resolve(&output.store).and_then(Resolved::as_char),
            Some('犍')
        );
        assert_eq!(output.store.resolve_str(*last), "漢");
    }

    #[test]
    fn explicit_ruby_preserves_semantic_nodes_inside_its_base() {
        let source = "｜瑞岩東畔命［＃二］軽舟［＃一］《ずいがんとうはんめいずけいしうを》";
        let output = Pipeline::run_to_completion(source);
        assert!(output.diagnostics.is_empty());
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(entry.source_span.slice(source), source);
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        let [
            Segment::Text(first),
            Segment::Node(Node::Kaeriten(two)),
            Segment::Text(second),
            Segment::Node(Node::Kaeriten(one)),
        ] = output.store.resolve_seg_range(*range)
        else {
            panic!(
                "expected text and kaeriten segments, got {:?}",
                output.store.resolve_seg_range(*range)
            );
        };
        assert_eq!(output.store.resolve_str(*first), "瑞岩東畔命");
        assert_eq!(output.store.resolve_str(two.mark), "二");
        assert_eq!(output.store.resolve_str(*second), "軽舟");
        assert_eq!(output.store.resolve_str(one.mark), "一");
    }

    #[test]
    fn ruby_reading_preserves_reclaimed_forward_format() {
        let source = "折口《ツムレ［＃「ムレ」に白丸傍点］》";
        let output = Pipeline::run_to_completion(source);
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.reading) else {
            panic!("expected a segmented ruby reading");
        };
        let [Segment::Text(prefix), Segment::Node(Node::Format(format))] =
            output.store.resolve_seg_range(*range)
        else {
            panic!(
                "expected text and format segments, got {:?}",
                output.store.resolve_seg_range(*range)
            );
        };
        assert_eq!(output.store.resolve_str(*prefix), "ツ");
        assert_eq!(
            output.store.content_range_as_plain(format.target),
            Some("ムレ")
        );
        assert!(matches!(format.origin, ForwardOrigin::Reclaimed));
        assert!(matches!(
            format.attr,
            ForwardAttr::Bouten {
                kind: BoutenKind::WhiteCircle,
                position: BoutenPosition::Right
            }
        ));
    }

    #[test]
    fn failed_explicit_ruby_probe_does_not_intern_text() {
        let source = "前文漢《かん》";
        let ruby_start = u32::try_from("前文".len()).expect("fixture fits u32");
        let mut alloc = Allocator::new();
        let base = alloc.content_plain("漢");
        let reading = alloc.content_plain("かん");
        let ruby = alloc.ruby(base, reading);
        let spans = [
            ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(0, ruby_start),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(
                    ruby_start,
                    u32::try_from(source.len()).expect("fixture fits u32"),
                ),
            },
        ];
        let before = (
            alloc.store().interner.len(),
            alloc.store().interner.stats.calls,
            alloc.store().interner.stats.allocs,
        );

        assert!(explicit_ruby_prefix(&spans, 1, source, alloc.store()).is_none());
        assert_eq!(
            (
                alloc.store().interner.len(),
                alloc.store().interner.stats.calls,
                alloc.store().interner.stats.allocs,
            ),
            before
        );
    }

    #[test]
    fn structured_forward_merge_requires_supported_origin_and_payload() {
        let mut alloc = Allocator::new();
        let Content::Plain(quoted) = alloc.content_plain("quoted") else {
            panic!("plain content")
        };

        assert!(structured_forward_can_merge(
            ForwardOrigin::SelfContained,
            ForwardPayload::None
        ));
        assert!(structured_forward_can_merge(
            ForwardOrigin::Referenced,
            ForwardPayload::NestedSource
        ));
        assert!(!structured_forward_can_merge(
            ForwardOrigin::Reclaimed,
            ForwardPayload::None
        ));
        assert!(!structured_forward_can_merge(
            ForwardOrigin::SelfContained,
            ForwardPayload::QuotedTarget(quoted)
        ));
    }

    #[test]
    fn structured_forward_plain_edge_accepts_only_prefix_related_targets() {
        assert!(plain_edge_matches_target("末", "前末"));
        assert!(plain_edge_matches_target("前末", "末"));
        assert!(!plain_edge_matches_target("前", "末"));
    }

    #[test]
    fn trailing_class_start_keeps_an_unrelated_prefix() {
        assert_eq!(
            trailing_class_start("前ABC", RubyBaseClass::Latin),
            "前".len()
        );
    }

    #[test]
    fn structured_gaiji_suffix_keeps_the_preceding_text() {
        let mut alloc = Allocator::new();
        let gaiji = alloc.make_gaiji("特のへん＋廴＋聿", Some("第3水準1-87-71"), false);

        assert_eq!(
            strip_structured_gaiji_suffix("前犍", gaiji, alloc.store()),
            Some("前")
        );
    }

    #[test]
    fn explicit_ruby_marker_does_not_add_an_empty_text_segment() {
        let source = "｜※［＃「特のへん＋廴＋聿」、第3水準1-87-71］《r》";
        let marker_end = u32::try_from('｜'.len_utf8()).expect("fixture fits u32");
        let ruby_start = u32::try_from(source.find('《').expect("ruby reading")).unwrap();
        let mut alloc = Allocator::new();
        let gaiji = alloc.make_gaiji("特のへん＋廴＋聿", Some("第3水準1-87-71"), false);
        let base = alloc.content_segments(&[alloc.seg_gaiji(gaiji)]);
        let reading = alloc.content_plain("r");
        let ruby = alloc.ruby(base, reading);
        let spans = [
            ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(0, marker_end),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(Node::Gaiji(gaiji)),
                source_span: Span::new(marker_end, ruby_start),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(
                    ruby_start,
                    u32::try_from(source.len()).expect("fixture fits u32"),
                ),
            },
        ];

        let (_, plan) = explicit_ruby_prefix(&spans, 2, source, alloc.store())
            .expect("structured base is reclaimed to the explicit marker");

        assert!(matches!(
            plan.as_slice(),
            [PlannedSegment::Ready(Segment::Gaiji(_))]
        ));
    }

    #[test]
    fn ordinary_ruby_does_not_rebuild_the_store() {
        let source = "前漢《かん》";
        let ruby_start = u32::try_from("前".len()).expect("fixture fits u32");
        let mut alloc = Allocator::new();
        let base = alloc.content_plain("漢");
        let reading = alloc.content_plain("かん");
        let ruby = alloc.ruby(base, reading);
        let mut spans = [
            ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(0, ruby_start),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(
                    ruby_start,
                    u32::try_from(source.len()).expect("fixture fits u32"),
                ),
            },
        ];
        let original = spans.clone();
        let before = (
            alloc.store().interner.len(),
            alloc.store().interner.stats.calls,
            alloc.store().interner.stats.allocs,
        );

        let decorated = merge_structured_content(&mut spans, source, &mut alloc);

        assert!(decorated.is_empty());
        assert_eq!(spans, original);
        assert_eq!(
            (
                alloc.store().interner.len(),
                alloc.store().interner.stats.calls,
                alloc.store().interner.stats.allocs,
            ),
            before
        );
    }

    #[test]
    fn streamed_plain_explicit_ruby_only_expands_its_span() {
        let source = "｜〔canapé〕《長椅子》";
        let base_start = u32::try_from("｜".len()).expect("fixture fits u32");
        let ruby_start = u32::try_from("｜〔canapé〕".len()).expect("fixture fits u32");
        let source_end = u32::try_from(source.len()).expect("fixture fits u32");
        let mut alloc = Allocator::new();
        let base = alloc.content_plain("〔canapé〕");
        let reading = alloc.content_plain("長椅子");
        let ruby = alloc.ruby(base, reading);
        let mut spans = [
            ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(0, base_start),
            },
            ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(base_start, ruby_start),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(ruby_start, source_end),
            },
        ];
        let original_ruby = spans[2].kind.clone();
        let before = (
            alloc.store().interner.len(),
            alloc.store().interner.stats.calls,
            alloc.store().interner.stats.allocs,
        );

        let decorated = merge_structured_content(&mut spans, source, &mut alloc);

        assert!(decorated.is_empty());
        assert_eq!(spans[2].source_span, Span::new(0, source_end));
        assert_eq!(spans[2].kind, original_ruby);
        assert_eq!(
            (
                alloc.store().interner.len(),
                alloc.store().interner.stats.calls,
                alloc.store().interner.stats.allocs,
            ),
            before
        );
    }

    #[test]
    fn failed_structured_forward_probe_does_not_intern_partial_match() {
        let directive = "［＃改ページ］";
        let source = format!("{directive}末");
        let boundary = u32::try_from(directive.len()).expect("fixture fits u32");
        let spans = [
            ClassifiedSpan {
                kind: SpanKind::Aozora(Node::PageBreak),
                source_span: Span::new(0, boundary),
            },
            ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(
                    boundary,
                    u32::try_from(source.len()).expect("fixture fits u32"),
                ),
            },
        ];
        let alloc = Allocator::new();
        let before = (
            alloc.store().interner.len(),
            alloc.store().interner.stats.calls,
            alloc.store().interner.stats.allocs,
        );

        assert!(structured_forward_prefix(&spans, "前末", &source, alloc.store()).is_none());
        assert_eq!(
            (
                alloc.store().interner.len(),
                alloc.store().interner.stats.calls,
                alloc.store().interner.stats.allocs,
            ),
            before
        );
    }

    #[test]
    fn explicit_ruby_bar_does_not_cross_a_boundary_or_failed_ruby() {
        for source in [
            "｜捨て\n甲［＃二］《こう》",
            "｜捨て［＃改ページ］甲［＃二］《こう》",
            "｜A《》B［＃二］《b》",
        ] {
            let output = Pipeline::run_to_completion(source);
            assert!(
                output
                    .source_nodes
                    .iter()
                    .all(|entry| !matches!(entry.node, NodeRef::Inline(Node::Ruby(_)))),
                "stale bar formed a ruby in {source:?}: {:?}",
                output.source_nodes
            );
        }
    }

    #[test]
    fn explicit_ruby_reclaims_unresolved_gaiji() {
        let source = "｜陀※［＃「架空の字」］多《r》";
        let output = Pipeline::run_to_completion(source);
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(entry.source_span.slice(source), source);
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        let [
            Segment::Text(first),
            Segment::Gaiji(gaiji),
            Segment::Text(last),
        ] = output.store.resolve_seg_range(*range)
        else {
            panic!("expected text, gaiji, text in the ruby base");
        };
        assert_eq!(output.store.resolve_str(*first), "陀");
        assert!(gaiji.resolve(&output.store).is_none());
        assert_eq!(output.store.resolve_str(*last), "多");
        assert!(matches!(
            output.diagnostics.as_slice(),
            [Diagnostic::UnresolvedGaiji { .. }]
        ));
    }

    #[test]
    fn explicit_ruby_uses_the_latest_marker() {
        let source = "文｜｜※［＃「特のへん＋廴＋聿」、第3水準1-87-71］《r》";
        let output = Pipeline::run_to_completion(source);
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(
            entry.source_span.slice(source),
            "｜※［＃「特のへん＋廴＋聿」、第3水準1-87-71］《r》"
        );
    }

    #[test]
    fn implicit_ruby_reclaims_alternating_resolved_gaiji_and_text() {
        let source = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀※［＃「特のへん＋廴＋聿」、第3水準1-87-71］多《r》";
        let output = Pipeline::run_to_completion(source);
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        assert_eq!(entry.source_span.slice(source), source);
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(range)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        assert!(matches!(
            output.store.resolve_seg_range(*range),
            [
                Segment::Gaiji(_),
                Segment::Text(_),
                Segment::Gaiji(_),
                Segment::Text(_)
            ]
        ));
    }

    #[test]
    fn unresolved_gaiji_does_not_guess_an_implicit_ruby_class() {
        let before = "※［＃「架空の字」］陀多《r》";
        let before_output = Pipeline::run_to_completion(before);
        assert!(matches!(
            before_output.source_nodes.as_slice(),
            [
                SourceNode {
                    node: NodeRef::Inline(Node::Gaiji(_)),
                    ..
                },
                SourceNode {
                    node: NodeRef::Inline(Node::Ruby(_)),
                    ..
                }
            ]
        ));

        let after = "陀多※［＃「架空の字」］《r》";
        let after_output = Pipeline::run_to_completion(after);
        let [entry] = after_output.source_nodes.as_slice() else {
            panic!(
                "expected one ruby node, got {:?}",
                after_output.source_nodes
            );
        };
        assert_eq!(entry.source_span.slice(after), "※［＃「架空の字」］《r》");
    }

    #[test]
    fn reading_gaiji_stays_in_the_reading_while_the_base_is_reclaimed() {
        let source = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀《かん※［＃「特のへん＋廴＋聿」、第3水準1-87-71］》";
        let output = Pipeline::run_to_completion(source);
        let [entry] = output.source_nodes.as_slice() else {
            panic!("expected one source node, got {:?}", output.source_nodes);
        };
        let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
            panic!("expected ruby node, got {:?}", entry.node);
        };
        let [Content::Segments(base)] = output.store.resolve_content_range(ruby.base) else {
            panic!("expected a segmented ruby base");
        };
        let [Content::Segments(reading)] = output.store.resolve_content_range(ruby.reading) else {
            panic!("expected a segmented ruby reading");
        };
        assert!(matches!(
            output.store.resolve_seg_range(*base),
            [Segment::Gaiji(_), Segment::Text(_)]
        ));
        assert!(matches!(
            output.store.resolve_seg_range(*reading),
            [Segment::Text(_), Segment::Gaiji(_)]
        ));
    }

    #[test]
    fn run_region_rejects_each_invalid_range_shape() {
        let source = "aあb";

        let inverted = Range { start: 4, end: 1 };
        assert!(Pipeline::run_region(source, inverted).is_none());
        assert!(Pipeline::run_region(source, 0..source.len() + 1).is_none());
        assert!(Pipeline::run_region(source, 2..4).is_none());
        assert!(Pipeline::run_region(source, 1..2).is_none());
        assert!(Pipeline::run_region(source, 1..4).is_some());
    }

    #[test]
    fn region_output_rejects_each_out_of_bounds_product() {
        let mut node = empty_region_build();
        node.recorder.source_nodes.push(SourceNode {
            source_span: Span::new(0, 1),
            normalized_offset: NormalizedOffset::new(0),
            node: NodeRef::Inline(Node::PageBreak),
        });
        assert!(build_region_output(node, 1, 2).is_none());

        let mut node = empty_region_build();
        node.recorder.source_nodes.push(SourceNode {
            source_span: Span::new(1, 3),
            normalized_offset: NormalizedOffset::new(0),
            node: NodeRef::Inline(Node::PageBreak),
        });
        assert!(build_region_output(node, 1, 2).is_none());

        let mut bounded_node = empty_region_build();
        bounded_node.recorder.source_nodes.push(SourceNode {
            source_span: Span::new(1, 2),
            normalized_offset: NormalizedOffset::new(0),
            node: NodeRef::Inline(Node::PageBreak),
        });
        assert!(build_region_output(bounded_node, 1, 2).is_some());

        let mut link = empty_region_build();
        link.links.push(PairLink::new(
            PairKind::Bracket,
            Span::new(0, 1),
            Span::new(1, 2),
        ));
        assert!(build_region_output(link, 1, 2).is_none());

        let mut link = empty_region_build();
        link.links.push(PairLink::new(
            PairKind::Bracket,
            Span::new(1, 2),
            Span::new(2, 3),
        ));
        assert!(build_region_output(link, 1, 2).is_none());

        let mut bounded_link = empty_region_build();
        bounded_link.links.push(PairLink::new(
            PairKind::Bracket,
            Span::new(1, 1),
            Span::new(1, 2),
        ));
        assert!(build_region_output(bounded_link, 1, 2).is_some());

        let mut diagnostic = empty_region_build();
        diagnostic
            .diagnostics
            .push(Diagnostic::source_contains_pua(Span::new(0, 1), '\u{E001}'));
        assert!(build_region_output(diagnostic, 1, 2).is_none());

        let mut diagnostic = empty_region_build();
        diagnostic
            .diagnostics
            .push(Diagnostic::source_contains_pua(Span::new(1, 3), '\u{E001}'));
        assert!(build_region_output(diagnostic, 1, 2).is_none());

        let mut bounded = empty_region_build();
        bounded
            .diagnostics
            .push(Diagnostic::source_contains_pua(Span::new(1, 2), '\u{E001}'));
        assert!(build_region_output(bounded, 1, 2).is_some());
    }

    #[test]
    fn intermediate_inspection_at_sanitized() {
        let p = Pipeline::new("plain text").sanitize();
        assert_eq!(p.sanitized_text(), "plain text");
        assert!(p.diagnostics().is_empty());
        drop(p.tokenize().pair().build());
    }

    #[test]
    fn intermediate_inspection_at_tokenized() {
        let p = Pipeline::new("a｜b《c》").sanitize().tokenize();
        assert!(p.tokens().len() >= 5);
        drop(p.pair().build());
    }

    #[test]
    fn intermediate_inspection_at_paired() {
        let p = Pipeline::new("a｜b《c》").sanitize().tokenize().pair();
        assert!(!p.events().is_empty());
        drop(p.build());
    }

    #[test]
    fn sanitize_pua_collision_diagnostic_propagates() {
        let out = Pipeline::run_to_completion("abc\u{E001}def");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn empty_source_round_trips() {
        let out = Pipeline::run_to_completion("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.sanitized.is_empty());
    }

    #[test]
    fn source_accessor_returns_original() {
        let s = "the original";
        let p = Pipeline::new(s);
        assert_eq!(p.source(), s);
    }

    // -----------------------------------------------------------------
    // Mutation-hardening unit tests (#488). Each pins an exact decision /
    // offset / classification a recogniser makes, hitting both sides of
    // every boundary so no branch/return/comparison mutant survives.
    // -----------------------------------------------------------------

    fn plain(start: u32, end: u32) -> ClassifiedSpan {
        ClassifiedSpan {
            kind: SpanKind::Plain,
            source_span: Span::new(start, end),
        }
    }

    fn newline(start: u32, end: u32) -> ClassifiedSpan {
        ClassifiedSpan {
            kind: SpanKind::Newline,
            source_span: Span::new(start, end),
        }
    }

    // find_heading_predecessor_position_at — every comparison / offset /
    // return boundary. The positive case returns `Some(3)` (≥ 2 so it
    // separates the whole-fn `Some(0)`/`Some(1)`/`None` stubs), and the
    // `\n`-adjacency / length / match boundaries each get a `None` twin.
    #[test]
    fn heading_predecessor_recognises_bare_line_above_bracket() {
        // "zz\nABC\nQ": the bare line `ABC` (bytes 3..6) sits directly above
        // the bracket at byte 7, and is itself preceded by a `\n` at byte 2.
        assert_eq!(
            find_heading_predecessor_position_at("zz\nABC\nQ", 7, "ABC"),
            Some(3)
        );
    }

    #[test]
    fn heading_predecessor_at_source_start_returns_start() {
        // "AB\nQ": target `AB` at bytes 0..2, bracket at 3. candidate_start is
        // 0 (start-of-source counts as a line boundary) — pins the `< len`
        // equal case and the `candidate_start == 0` short-circuit.
        assert_eq!(
            find_heading_predecessor_position_at("AB\nQ", 3, "AB"),
            Some(0)
        );
    }

    #[test]
    fn heading_predecessor_rejects_non_boundaries() {
        // bracket at byte 0 → no predecessor (cutoff == 0 short-circuit).
        assert_eq!(
            find_heading_predecessor_position_at("whatever", 0, "AB"),
            None
        );
        // target longer than the available text before the `\n`.
        assert_eq!(find_heading_predecessor_position_at("\nQ", 1, "ABC"), None);
        // the char before the byte offset is not `\n`, so no bare line.
        assert_eq!(find_heading_predecessor_position_at("ABxQ", 4, "AB"), None);
        // target matches but is not itself preceded by a line break
        // (`x` sits before `AB`): candidate_start != 0 && byte != '\n'.
        assert_eq!(
            find_heading_predecessor_position_at("xAB\nQ", 4, "AB"),
            None
        );
    }

    // foldable_inline_attr — each foldable arm maps to its own attribute,
    // and the non-inline / block variants fall through to `None`.
    #[test]
    fn foldable_inline_attr_maps_each_arm() {
        let k = BoutenKind::Goma;
        let p = BoutenPosition::Right;
        assert_eq!(
            foldable_inline_attr(RegionFormat::Bold { padded: false }),
            Some(ForwardAttr::Bold)
        );
        assert_eq!(
            foldable_inline_attr(RegionFormat::Gothic { padded: false }),
            Some(ForwardAttr::Gothic)
        );
        assert_eq!(
            foldable_inline_attr(RegionFormat::Italic { padded: false }),
            Some(ForwardAttr::Italic)
        );
        assert_eq!(
            foldable_inline_attr(RegionFormat::Caption { padded: false }),
            Some(ForwardAttr::Caption)
        );
        assert_eq!(
            foldable_inline_attr(RegionFormat::Bouten {
                kind: k,
                position: p
            }),
            Some(ForwardAttr::Bouten {
                kind: k,
                position: p
            })
        );
        assert_eq!(
            foldable_inline_attr(RegionFormat::SmallScript(p)),
            Some(ForwardAttr::SmallScript(p))
        );
        // Block-level (padded) and non-foldable variants decline.
        assert_eq!(
            foldable_inline_attr(RegionFormat::Bold { padded: true }),
            None
        );
        assert_eq!(foldable_inline_attr(RegionFormat::Table), None);
    }

    // attr_decorates_ruby_base — a decoratable attribute returns true, a
    // sub-character / target-splitting one returns false.
    #[test]
    fn attr_decorates_ruby_base_both_sides() {
        assert!(attr_decorates_ruby_base(ForwardAttr::Bold));
        assert!(attr_decorates_ruby_base(ForwardAttr::Italic));
        assert!(!attr_decorates_ruby_base(ForwardAttr::AccentDot));
        assert!(!attr_decorates_ruby_base(ForwardAttr::Fraction));
    }

    // lower_spans full-superset drop: when an incoming span is a *proper*
    // superset of the committed `back`, `back` is dropped.
    #[test]
    fn lower_spans_superset_drop() {
        let src = "x".repeat(40);
        // Proper superset by left extension → back popped.
        let (out, _) = lower_spans(
            vec![plain(10, 20), plain(5, 20)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source_span, Span::new(5, 20));
        // Proper superset by right extension → back popped.
        let (out, _) = lower_spans(
            vec![plain(10, 20), plain(10, 25)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source_span, Span::new(10, 25));
        // Equal spans are NOT a proper superset → both retained.
        let (out, _) = lower_spans(
            vec![plain(10, 20), plain(10, 20)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out.len(), 2);
    }

    // lower_spans partial-overlap truncation: a span pulling back into the
    // tail of a committed *plain* run truncates that run's end to the pull
    // point; every other geometry leaves the run untouched.
    #[test]
    fn lower_spans_partial_overlap_truncates_plain_tail() {
        let src = "x".repeat(40);
        // Tail overlap (bs < ss < be), back is plain → truncate end to ss.
        let (out, _) = lower_spans(
            vec![plain(10, 20), plain(15, 30)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out[0].source_span.end, 15);
        // back is NOT plain (Newline) → no truncation.
        let (out, _) = lower_spans(
            vec![newline(10, 20), plain(15, 30)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out[0].source_span.end, 20);
        // no overlap (ss >= be) → no truncation.
        let (out, _) = lower_spans(
            vec![plain(10, 20), plain(25, 30)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out[0].source_span.end, 20);
        // bs == ss (not strictly inside) → no truncation.
        let (out, _) = lower_spans(
            vec![plain(10, 20), plain(10, 15)],
            &src,
            &mut Allocator::new(),
        );
        assert_eq!(out[0].source_span.end, 20);
    }

    // decorate_ruby_bases — a Referenced forward directive with a
    // decoratable attribute whose target is a *unique* preceding ruby base
    // sets that ruby's `base_emphasis` and reports the directive span.
    #[test]
    fn decorate_ruby_bases_unique_referent() {
        let mut alloc = Allocator::new();
        let base = alloc.content_plain("青");
        let reading = alloc.content_plain("あお");
        let ruby = alloc.ruby(base, reading);
        let tgt = alloc.content_plain("青");
        let fmt = alloc.forward_format(ForwardAttr::Bold, tgt, ForwardOrigin::Referenced);
        let mut out = vec![
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(0, 3),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(fmt),
                source_span: Span::new(3, 20),
            },
        ];
        let decorated = decorate_ruby_bases(&mut out, "青あお青", alloc.store());
        assert_eq!(decorated, vec![Span::new(3, 20)]);
        let SpanKind::Aozora(Node::Ruby(r)) = out[0].kind else {
            panic!("expected ruby span");
        };
        assert_eq!(r.base_emphasis, Some(ForwardAttr::Bold));
    }

    // A Referenced directive with a non-decoratable attribute never fires,
    // even when its target uniquely names a preceding ruby base.
    #[test]
    fn decorate_ruby_bases_declines_non_decoratable_attr() {
        let mut alloc = Allocator::new();
        let base = alloc.content_plain("青");
        let reading = alloc.content_plain("あお");
        let ruby = alloc.ruby(base, reading);
        let tgt = alloc.content_plain("青");
        let fmt = alloc.forward_format(ForwardAttr::AccentDot, tgt, ForwardOrigin::Referenced);
        let mut out = vec![
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(0, 3),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(fmt),
                source_span: Span::new(3, 20),
            },
        ];
        let decorated = decorate_ruby_bases(&mut out, "青あお", alloc.store());
        assert!(decorated.is_empty());
    }

    // A preceding *plain* run carrying the target text is a competing
    // referent that forces a decline (ambiguity).
    #[test]
    fn decorate_ruby_bases_declines_on_competing_plain() {
        let mut alloc = Allocator::new();
        let base = alloc.content_plain("山茶花");
        let reading = alloc.content_plain("さざんか");
        let ruby = alloc.ruby(base, reading);
        let tgt = alloc.content_plain("山茶花");
        let fmt = alloc.forward_format(ForwardAttr::Bold, tgt, ForwardOrigin::Referenced);
        let n = u32::try_from("山茶花".len()).unwrap(); // 9 bytes
        let mut out = vec![
            plain(0, n), // a plain copy of the target, before the ruby
            ClassifiedSpan {
                kind: SpanKind::Aozora(ruby),
                source_span: Span::new(n, n + n),
            },
            ClassifiedSpan {
                kind: SpanKind::Aozora(fmt),
                source_span: Span::new(100, 130),
            },
        ];
        let decorated = decorate_ruby_bases(&mut out, "山茶花", alloc.store());
        assert!(decorated.is_empty());
    }

    // try_fold_inline — a matched foldable pair over an all-plain body folds
    // into a single forward-format leaf spanning open..close.
    #[test]
    fn try_fold_inline_folds_all_plain_body() {
        let mut alloc = Allocator::new();
        let source = "ABCDEF";
        let frame = OpenFrame {
            open: ClassifiedSpan {
                kind: SpanKind::BlockOpen(RegionFormat::Bold { padded: false }),
                source_span: Span::new(0, 1),
            },
            region: RegionFormat::Bold { padded: false },
            collected: vec![plain(1, 4)],
        };
        let close = ClassifiedSpan {
            kind: SpanKind::BlockClose(RegionClose::Bold { padded: false }),
            source_span: Span::new(4, 5),
        };
        let folded = try_fold_inline(&frame, &close, source, &mut alloc).expect("pair folds");
        assert!(matches!(folded.kind, SpanKind::Aozora(Node::Format(_))));
        assert_eq!(folded.source_span, Span::new(0, 5));
    }

    // A body that is not entirely plain is not foldable → `None`.
    #[test]
    fn try_fold_inline_rejects_non_plain_body() {
        let mut alloc = Allocator::new();
        let source = "ABCDEF";
        let frame = OpenFrame {
            open: ClassifiedSpan {
                kind: SpanKind::BlockOpen(RegionFormat::Bold { padded: false }),
                source_span: Span::new(0, 1),
            },
            region: RegionFormat::Bold { padded: false },
            collected: vec![plain(1, 3), newline(3, 4)],
        };
        let close = ClassifiedSpan {
            kind: SpanKind::BlockClose(RegionClose::Bold { padded: false }),
            source_span: Span::new(4, 5),
        };
        assert!(try_fold_inline(&frame, &close, source, &mut alloc).is_none());
    }

    // fold_inline_emphasis — a BlockOpen opens a frame whose matched close
    // folds the enclosed plain run into one Aozora leaf.
    #[test]
    fn fold_inline_emphasis_collapses_matched_pair() {
        let mut alloc = Allocator::new();
        let source = "ABCDEF";
        let spans = vec![
            ClassifiedSpan {
                kind: SpanKind::BlockOpen(RegionFormat::Bold { padded: false }),
                source_span: Span::new(0, 1),
            },
            plain(1, 4),
            ClassifiedSpan {
                kind: SpanKind::BlockClose(RegionClose::Bold { padded: false }),
                source_span: Span::new(4, 5),
            },
        ];
        let out = fold_inline_emphasis(spans, source, &mut alloc);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].kind, SpanKind::Aozora(Node::Format(_))));
    }

    // The Paired state's link side-table is materialised, not empty: a ruby
    // reading `《…》` resolves to a `PairLink`.
    #[test]
    fn paired_links_side_table_is_populated() {
        let p = Pipeline::new("｜青梅《おうめ》")
            .sanitize()
            .tokenize()
            .pair();
        assert!(!p.links().is_empty());
        drop(p.build());
    }

    // build() suppresses the `forward_referent_not_stylable` warning for a
    // directive the lowering pass decorated onto a unique ruby base (#384):
    // `山茶花《さざんか》…［＃「山茶花」は罫囲み］` decorates and drops the warning.
    #[test]
    fn build_suppresses_decorated_ruby_base_warning() {
        let out = Pipeline::run_to_completion("笠の山茶花《さざんか》［＃「山茶花」は罫囲み］");
        assert_eq!(
            out.diagnostics
                .iter()
                .filter(|d| matches!(d, Diagnostic::ForwardReferentNotStylable { .. }))
                .count(),
            0,
            "decorated directive's warning must be dropped, got {:?}",
            out.diagnostics
        );
    }

    // build() keeps the warning for a directive that did NOT decorate (an
    // ambiguous target). With a decoration also present in the document the
    // retain closure must key on the exact span, not on any/every warning.
    #[test]
    fn build_keeps_undecorated_forward_warning() {
        let out = Pipeline::run_to_completion(
            "笠の山茶花《さざんか》［＃「山茶花」は罫囲み］\n｜青梅《おうめ》と｜青梅《せいばい》は［＃「青梅」に傍点］別。",
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::ForwardReferentNotStylable { .. })),
            "ambiguous directive's warning must survive, got {:?}",
            out.diagnostics
        );
    }
}
