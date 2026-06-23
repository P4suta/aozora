//! Type-state lex pipeline.
//!
//! `Pipeline<'src, 'a, S>` makes the lex stage order enforceable at
//! compile time. The state markers [`Source`], [`Sanitized`],
//! [`Tokenized`], [`Paired`] track which stages have run; methods
//! consume `self` and return the next state. Calling `.pair()` on a
//! `Source` is a type error; calling `.tokenize()` twice is a type
//! error; etc.
//!
//! # Two entry shapes
//!
//! - [`Pipeline::run_to_completion`] — one-shot, equivalent to
//!   [`crate::lex`]. Used by `Document::parse` and the
//!   FFI / WASM / Python drivers.
//! - [`Pipeline::new`] → `.sanitize()` → `.tokenize()` → `.pair()` →
//!   `.build()` — explicit chain. Use for inspection / instrumentation:
//!   each intermediate state exposes accessors (`.sanitized_text()`,
//!   `.tokens()`, `.events()`, `.diagnostics()`) so callers can probe
//!   the partial output without re-running the pipeline.
//!
//! # Arena-batch passing
//!
//! Every inter-stage boundary materialises a [`bumpalo::collections::Vec`]
//! inside the pipeline's [`Arena`]. The tokenize stage emits
//! `BumpVec<'a, Token>`; the pair stage emits `BumpVec<'a, PairEvent>`;
//! the classify stage streams its `ClassifiedSpan`s through the
//! `ArenaNormalizer` callback (no third Vec materialisation — the
//! streaming `classify` Iterator path is the cheapest shape on the
//! corpus).
//!
//! Net effect on the corpus profile: per-parse `malloc`/`free`
//! traffic collapses into a single bump-pointer advance per element.
//!
//! # State carries its own payload
//!
//! Each state marker is a field-bound struct holding exactly the
//! stage outputs it has produced (`Sanitized` carries the arena
//! `&'a str`; `Tokenized` adds the token `BumpVec`; …). Reading
//! `.sanitized_text()` from `Pipeline<'_, '_, Sanitized>` is a
//! field projection on the state struct — no `Option::expect`
//! lives in production code. The compiler enforces "you cannot
//! ask for tokens unless you are in `Tokenized`" via method
//! resolution alone.
//!
//! # Lifetime model
//!
//! `'src` is the original source text lifetime; `'a` is the arena
//! lifetime. The sanitized text is materialised into the arena at the
//! `Sanitized` transition (cost: one `arena.alloc_str` of
//! `sanitize(source).text`), so all downstream stages borrow from the
//! arena rather than from in-Pipeline storage. This eliminates the
//! self-referential-struct problem `Tokenizer<'sanitized>` would
//! otherwise impose.
//!
//! # Compile-time stage-order enforcement
//!
//! Calling `.pair()` on a fresh [`Source`] (without going through
//! `.sanitize().tokenize()`) is a *type error*: there is no
//! `impl Pipeline<'_, '_, Source>::pair` method. The compile-fail
//! doctest below pins this contract — adding such an impl in the
//! future would silently break the type-state guarantee:
//!
//! ```compile_fail
//! use aozora_pipeline::Pipeline;
//! use aozora_syntax::borrowed::Arena;
//!
//! let arena = Arena::new();
//! // .pair() on Source skips the sanitize + tokenize stages — must not compile.
//! let _ = Pipeline::new("plain", &arena).pair();
//! ```
//!
//! # Why `build` is the terminal transition
//!
//! The classify stage requires `&mut BorrowedAllocator<'a>`. The
//! allocator owns the `Interner<'a>` whose internal `RefCell` makes
//! it `!Sync`; threading `&mut alloc` through Pipeline states would
//! force the allocator to live as long as the pipeline, blocking any
//! external pause-and-inspect between the pair and classify stages. We
//! collapse the classify stage + the `ArenaNormalizer` fold
//! into a single terminal `.build()` call instead — inspection up
//! through `Paired` works freely; the final allocation pass is
//! atomic.

use core::marker::PhantomData;

use crate::lexer::{
    ClassifiedSpan, PairEvent, SpanKind, Token, classify, pair_in, sanitize, tokenize_in,
};
use aozora_spec::{Diagnostic, PairLink};
use core::mem::take;

use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::{Arena, ContainerPair, Registry};
use aozora_syntax::{ForwardAttr, RegionClose, RegionFormat, Span};
use bumpalo::collections::Vec as BumpVec;

use crate::LexOutput;
use crate::borrowed::{ArenaNormalizer, SourceNode};

// =====================================================================
// State markers (field-bound — each state carries the stage output it
// is responsible for. No `Option` / `expect` chain in production code:
// the type system guarantees the field is present whenever the state
// type can be named).
// =====================================================================

/// Initial state — no stage has run yet.
#[derive(Debug, Clone, Copy)]
pub struct Source;

/// The sanitize stage has run; sanitized text is materialised in the arena.
#[derive(Debug, Clone, Copy)]
pub struct Sanitized<'a> {
    sanitized_text: &'a str,
}

/// The tokenize stage has run; the token list is materialised inside the arena.
#[derive(Debug)]
pub struct Tokenized<'a> {
    sanitized_text: &'a str,
    tokens: BumpVec<'a, Token>,
}

/// The pair stage has run; the event list and the resolved (open, close)
/// link side-table are materialised inside the arena.
#[derive(Debug)]
pub struct Paired<'a> {
    sanitized_text: &'a str,
    events: BumpVec<'a, PairEvent>,
    links: BumpVec<'a, PairLink>,
}

// =====================================================================
// Pipeline
// =====================================================================

/// Type-state lex pipeline. Each state's transition method consumes
/// `self`, materialises its stage output into the next state struct,
/// and returns a new pipeline in the next state.
#[derive(Debug)]
pub struct Pipeline<'src, 'a, S> {
    source: &'src str,
    arena: &'a Arena,
    diagnostics: Vec<Diagnostic>,
    state: S,
    // Tie the unused `'a` lifetime to the struct so the compiler
    // accepts state structs that reference the arena even when the
    // current state marker (`Source`) doesn't. Zero size at runtime.
    _arena: PhantomData<&'a Arena>,
}

// ---------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Source> {
    /// Wrap a source string for type-state-driven lex. The sanitize
    /// stage has not yet run; only `source` and `arena` are set.
    #[must_use]
    pub fn new(source: &'src str, arena: &'a Arena) -> Self {
        Self {
            source,
            arena,
            diagnostics: Vec::new(),
            state: Source,
            _arena: PhantomData,
        }
    }

    /// One-shot driver: run every stage and return the final
    /// [`LexOutput`]. Equivalent to [`crate::lex`].
    #[must_use]
    pub fn run_to_completion(source: &'src str, arena: &'a Arena) -> LexOutput<'a> {
        Self::new(source, arena)
            .sanitize()
            .tokenize()
            .pair()
            .build()
    }

    /// Borrow the original source text.
    #[must_use]
    pub fn source(&self) -> &'src str {
        self.source
    }

    /// Run the sanitize stage. Materialises the sanitized text in the
    /// arena so downstream stages borrow from the arena, not from the
    /// Pipeline struct (which would be self-referential).
    #[must_use]
    pub fn sanitize(mut self) -> Pipeline<'src, 'a, Sanitized<'a>> {
        let out = sanitize(self.source);
        self.diagnostics.extend(out.diagnostics);
        let arena_text: &'a str = self.arena.alloc_str(&out.text);
        Pipeline {
            source: self.source,
            arena: self.arena,
            diagnostics: self.diagnostics,
            state: Sanitized {
                sanitized_text: arena_text,
            },
            _arena: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Sanitized
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Sanitized<'a>> {
    /// Sanitized text (arena-allocated).
    #[must_use]
    pub fn sanitized_text(&self) -> &'a str {
        self.state.sanitized_text
    }

    /// Diagnostics accumulated through the sanitize stage.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run the tokenize stage. Materialises the full
    /// `BumpVec<'a, Token>` inside `arena` via [`tokenize_in`].
    #[must_use]
    pub fn tokenize(self) -> Pipeline<'src, 'a, Tokenized<'a>> {
        let sanitized_text = self.state.sanitized_text;
        let tokens = tokenize_in(sanitized_text, self.arena);
        Pipeline {
            source: self.source,
            arena: self.arena,
            diagnostics: self.diagnostics,
            state: Tokenized {
                sanitized_text,
                tokens,
            },
            _arena: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Tokenized
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Tokenized<'a>> {
    /// Borrow the materialised token list. Useful for instrumentation.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.state.tokens
    }

    /// Run the pair stage. Materialises a paired-event stream
    /// inside `arena` via [`pair_in`]. The pair stage's
    /// diagnostics are drained into the pipeline's diagnostic
    /// accumulator immediately.
    #[must_use]
    pub fn pair(mut self) -> Pipeline<'src, 'a, Paired<'a>> {
        let Tokenized {
            sanitized_text,
            tokens,
        } = self.state;
        let out = pair_in(&tokens, self.arena);
        self.diagnostics.extend(out.diagnostics);
        Pipeline {
            source: self.source,
            arena: self.arena,
            diagnostics: self.diagnostics,
            state: Paired {
                sanitized_text,
                events: out.events,
                links: out.links,
            },
            _arena: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Paired (terminal)
// ---------------------------------------------------------------------

impl<'a> Pipeline<'_, 'a, Paired<'a>> {
    /// Borrow the materialised pair-event list. Useful for inspection
    /// before `.build()`.
    #[must_use]
    pub fn events(&self) -> &[PairEvent] {
        &self.state.events
    }

    /// Borrow the resolved (open, close) pair side-table. Useful for
    /// inspection before `.build()`.
    #[must_use]
    pub fn links(&self) -> &[PairLink] {
        &self.state.links
    }

    /// Drive the classify stage + the arena normalizer fold and return
    /// the final [`LexOutput`]. Terminal transition because
    /// `&mut BorrowedAllocator` cannot be safely held across an external
    /// pause without locking the pipeline into a single thread for the
    /// allocator's lifetime.
    ///
    /// # Diagnostic order
    ///
    /// Sanitize stage → pair stage (unclosed/unmatched) →
    /// classify stage (unknown annotations etc.). Matches the
    /// pre-Pipeline `lex` ordering.
    ///
    /// # Panics
    ///
    /// Panics if the sanitized source exceeds `u32::MAX` bytes
    /// (the lexer's `Span` width contract). In practice unreachable;
    /// the sanitize stage caps source length at the same boundary.
    #[must_use]
    pub fn build(mut self) -> LexOutput<'a> {
        let Paired {
            sanitized_text,
            events,
            links,
        } = self.state;
        let sanitized_len =
            u32::try_from(sanitized_text.len()).expect("sanitize asserts source.len() <= u32::MAX");

        // Allocator capacity hint: source.len()/32 is a rough upper bound
        // on the number of distinct strings the borrowed pipeline will
        // intern. `BorrowedAllocator::with_capacity` rounds up to the
        // next power of two; floor of 64 covers short documents.
        let interner_hint = (sanitized_text.len() / 32).max(64);
        let mut alloc = BorrowedAllocator::with_capacity(self.arena, interner_hint);
        let mut builder = ArenaNormalizer::new(sanitized_text, sanitized_text.len() / 64);

        // Drain the arena-allocated `BumpVec<PairEvent>` through the
        // streaming `classify` Iterator path.
        let mut events_iter = events.into_iter();
        let mut classify_stream = classify(&mut events_iter, sanitized_text, &mut alloc);
        // Materialise the classified spans and drain the stream's
        // diagnostics, then drop the stream so its `&mut alloc` borrow is
        // released before the NORMALIZE (lowering) pass — the pass folds
        // surface dialects into canonical core nodes (e.g. inline-range
        // emphasis → forward leaf) and needs the allocator to mint them.
        let spans: Vec<ClassifiedSpan<'a>> = (&mut classify_stream).collect();
        let classify_diagnostics: Vec<Diagnostic> = classify_stream.take_diagnostics();
        drop(classify_stream);
        for span in &lower_spans(spans, sanitized_text, &mut alloc) {
            builder.emit(span);
        }
        self.diagnostics.extend(classify_diagnostics);
        // Normalizer diagnostics (e.g. mismatched container close) are
        // produced during the `emit` fold above but buffered on the
        // builder; append them *after* the classify-stage set so the
        // final vector stays in pipeline-stage order (the normalizer is
        // the post-classify fold). See `tests/diagnostic_ordering.rs`.
        self.diagnostics.extend(take(&mut builder.diagnostics));

        let normalized: &'a str = self.arena.alloc_str(&builder.out);
        // Single-table Registry: classifier emits in source order so
        // `entries` is already sorted by position; from_sorted_slice
        // skips the redundant sort pass.
        let registry = Registry::from_sorted_slice(&builder.entries);
        // Freeze the arena `BumpVec<PairLink>` into a `&'a [PairLink]`.
        // `BumpVec::into_bump_slice` consumes self and returns a slice
        // alive for the bump allocator's lifetime, exactly the lifetime
        // we need on `LexOutput::pairs`.
        let pairs: &'a [PairLink] = links.into_bump_slice();
        // Move the source-keyed side table out of the heap-backed
        // `Vec<SourceNode>` and into the arena, in one allocation.
        let source_nodes: &'a [SourceNode<'a>] = self.arena.alloc_slice_copy(&builder.source_nodes);
        // Same dance for the container-pair side table — close-order
        // (matches the close events as the open-stack drains).
        let container_pairs: &'a [ContainerPair] =
            self.arena.alloc_slice_copy(&builder.container_pairs);
        let intern_stats = alloc.into_interner().stats;

        LexOutput {
            normalized,
            sanitized: sanitized_text,
            registry,
            diagnostics: self.diagnostics,
            sanitized_len,
            pairs,
            source_nodes,
            container_pairs,
            intern_stats,
        }
    }
}

/// NORMALIZE (lowering) pass over the materialized classified-span list.
///
/// This is the seam the normalization waist is built on. Today it performs
/// only the source-byte **drop-superset** the streaming 4-span window did:
/// when a later span's source span is a proper superset of an earlier one
/// — a backward pull-back, e.g. a promoted 大/中/小 heading reclaiming its
/// referent line `序章\n`, or a forward node reclaiming its predecessor
/// literal — the subsumed earlier span is dropped, so the normalizer (which
/// appends in source order) does not emit the reclaimed text twice.
///
/// Running over the whole list rather than a depth-4 tail is behaviour-
/// preserving (a pull-back reaches only the immediately-preceding line, so
/// nothing more than 4 spans back was ever subsumed) and is what later
/// folds extend: a forward directive's target span(s) will be wrapped into
/// a scope-free region here instead of via the classifier's streaming
/// pull-back, dissolving the `consumed_predecessor` round-trip pathology.
fn lower_spans<'a>(
    spans: Vec<ClassifiedSpan<'a>>,
    source: &'a str,
    alloc: &mut BorrowedAllocator<'a>,
) -> Vec<ClassifiedSpan<'a>> {
    let mut out: Vec<ClassifiedSpan<'a>> = Vec::with_capacity(spans.len());
    for span in spans {
        while let Some(back) = out.last() {
            let (bs, be) = (back.source_span.start, back.source_span.end);
            let back_is_plain = matches!(back.kind, SpanKind::Plain);
            let (ss, se) = (span.source_span.start, span.source_span.end);
            if ss <= bs && be <= se && (ss < bs || se > be) {
                // Full superset: `span` reclaimed all of `back` (a promoted
                // heading swallowing its referent line). Drop `back`.
                out.pop();
            } else if back_is_plain && bs < ss && ss < be {
                // Partial overlap: `span` (a consumed_predecessor forward
                // node) pulled its source region back into the *tail* of a
                // committed plain run — the streaming flush could not splice
                // the hole, so the reclaimed literal sits in BOTH the plain
                // tail and the node, doubling on serialize (issue #180,
                // unbounded growth). Truncate the plain to end where the node
                // begins so the literal is emitted once, by the node.
                if let Some(last) = out.last_mut() {
                    last.source_span.end = ss;
                }
                break;
            } else {
                break;
            }
        }
        out.push(span);
    }
    // Second phase: fold S4-foldable inline-range emphasis (`［＃太字］ … ［＃太字終わり］`
    // bare ranges over 太字 / 斜体 / キャプション) into canonical forward leaves.
    fold_inline_emphasis(out, source, alloc)
}

/// The forward-scope attribute an S4-foldable inline-range region folds to.
///
/// Only the bare-range (`padded: false`) 太字 / 斜体 / キャプション forms fold;
/// the block (`padded: true`) forms and every other region stay
/// [`SpanKind::BlockOpen`] / [`SpanKind::BlockClose`] containers. 傍点 / 傍線
/// ranges are out of scope here — they fold in a later step.
const fn foldable_inline_attr(region: RegionFormat) -> Option<ForwardAttr> {
    match region {
        RegionFormat::Bold { padded: false } => Some(ForwardAttr::Bold),
        RegionFormat::Italic { padded: false } => Some(ForwardAttr::Italic),
        RegionFormat::Caption { padded: false } => Some(ForwardAttr::Caption),
        _ => None,
    }
}

/// An open inline-range marker awaiting its close, with the spans seen since.
struct OpenFrame<'a> {
    /// The `BlockOpen` span itself (re-emitted verbatim if the pair does not fold).
    open: ClassifiedSpan<'a>,
    /// The open marker's region (drives foldability and the close-match check).
    region: RegionFormat,
    /// Spans between this open and its eventual close, in source order.
    collected: Vec<ClassifiedSpan<'a>>,
}

/// Push a finished span onto the innermost open frame, or to `output` at top level.
fn emit_to<'a>(
    stack: &mut [OpenFrame<'a>],
    output: &mut Vec<ClassifiedSpan<'a>>,
    span: ClassifiedSpan<'a>,
) {
    if let Some(top) = stack.last_mut() {
        top.collected.push(span);
    } else {
        output.push(span);
    }
}

/// Fold a matched inline-range pair into a forward leaf, or `None` to keep it
/// as a container.
///
/// Folds only when (a) the open is an S4-foldable bare range, (b) the close's
/// family matches the open (an `［＃太字］…［＃斜体終わり］` mismatch keeps both
/// markers so the normalizer still diagnoses it), and (c) the enclosed run is a
/// non-empty, *text-only* (`Plain`) sequence. The text-only bound is load-
/// bearing: the serializer's `emit_content_as_plain` reproduces gaiji / 注記
/// bodies bare (no `※［＃…］` / `［＃…］`), so absorbing a non-text segment into a
/// forward target would silently drop its notation on serialize. Ruby / nested
/// formats cannot be a `Segment` at all. Such ranges stay containers.
fn try_fold_inline<'a>(
    frame: &OpenFrame<'a>,
    close: &ClassifiedSpan<'a>,
    source: &'a str,
    alloc: &mut BorrowedAllocator<'a>,
) -> Option<ClassifiedSpan<'a>> {
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
    let node = alloc.forward_format(attr, content, true);
    Some(ClassifiedSpan {
        kind: SpanKind::Aozora(node),
        source_span: Span::new(frame.open.source_span.start, close.source_span.end),
    })
}

/// Fold S4-foldable inline-range emphasis pairs into forward leaves.
///
/// A balanced-stack walk mirroring the normalizer's container pairing: each
/// `BlockClose` pops the nearest open (regardless of kind). A matched pair
/// folds when [`try_fold_inline`] allows it; otherwise the open marker, its
/// collected spans, and the close marker flow through verbatim, so mismatched /
/// non-foldable / unclosed ranges behave exactly as before. Innermost pairs
/// fold first, so a nested range leaves its parent's enclosed run non-text-only
/// (it now holds an `Aozora` node) — which correctly blocks the outer fold.
fn fold_inline_emphasis<'a>(
    spans: Vec<ClassifiedSpan<'a>>,
    source: &'a str,
    alloc: &mut BorrowedAllocator<'a>,
) -> Vec<ClassifiedSpan<'a>> {
    let mut output: Vec<ClassifiedSpan<'a>> = Vec::with_capacity(spans.len());
    let mut stack: Vec<OpenFrame<'a>> = Vec::new();
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
                    // Stray close with no open on the stack — pass through.
                    output.push(span);
                }
            }
            _ => emit_to(&mut stack, &mut output, span),
        }
    }
    // Flush any unclosed opens (bottom-to-top reconstructs source order: an
    // inner open's frame holds only the spans after it opened).
    for frame in stack {
        output.push(frame.open);
        output.extend(frame.collected);
    }
    output
}

#[cfg(test)]
mod tests {
    use core::ptr;

    use aozora_syntax::borrowed::Arena;

    use super::*;

    #[test]
    fn type_state_chain_compiles() {
        let arena = Arena::new();
        let _final = Pipeline::new("｜青梅《おうめ》", &arena)
            .sanitize()
            .tokenize()
            .pair()
            .build();
    }

    #[test]
    fn run_to_completion_matches_chain() {
        let arena1 = Arena::new();
        let arena2 = Arena::new();
        let chain = Pipeline::new("｜青梅《おうめ》", &arena1)
            .sanitize()
            .tokenize()
            .pair()
            .build();
        let oneshot = Pipeline::run_to_completion("｜青梅《おうめ》", &arena2);
        assert_eq!(chain.normalized, oneshot.normalized);
        assert_eq!(chain.sanitized_len, oneshot.sanitized_len);
        assert_eq!(
            chain.registry.count_kind(aozora_spec::Sentinel::Inline),
            oneshot.registry.count_kind(aozora_spec::Sentinel::Inline)
        );
    }

    #[test]
    fn intermediate_inspection_at_sanitized() {
        let arena = Arena::new();
        let p = Pipeline::new("plain text", &arena).sanitize();
        assert_eq!(p.sanitized_text(), "plain text");
        assert!(p.diagnostics().is_empty());
        drop(p.tokenize().pair().build());
    }

    #[test]
    fn intermediate_inspection_at_tokenized() {
        let arena = Arena::new();
        let p = Pipeline::new("a｜b《c》", &arena).sanitize().tokenize();
        // Token sanity: at least Text+Trigger+Text+Trigger+Text+Trigger.
        assert!(p.tokens().len() >= 5);
        drop(p.pair().build());
    }

    #[test]
    fn intermediate_inspection_at_paired() {
        let arena = Arena::new();
        let p = Pipeline::new("a｜b《c》", &arena)
            .sanitize()
            .tokenize()
            .pair();
        assert!(!p.events().is_empty());
        drop(p.build());
    }

    #[test]
    fn sanitize_pua_collision_diagnostic_propagates() {
        let arena = Arena::new();
        let out = Pipeline::run_to_completion("abc\u{E001}def", &arena);
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
        let arena = Arena::new();
        let out = Pipeline::run_to_completion("", &arena);
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert_eq!(out.sanitized_len, 0);
    }

    #[test]
    fn source_accessor_returns_original() {
        let arena = Arena::new();
        let s = "the original";
        let p = Pipeline::new(s, &arena);
        assert!(ptr::eq(p.source(), s));
    }
}
