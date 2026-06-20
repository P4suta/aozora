//! Phase 3 — classify the Phase 2 event stream into [`borrowed::Node`] spans.
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
//!   concrete [`borrowed::Node`] that Phase 4 will replace with a PUA
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
//! sanitized source. The driver loop's `Classifier::try_recognize`
//! dispatches based on the leading event kind:
//!
//! * Ruby (`｜X《Y》` explicit, trailing-kanji implicit)
//! * Bracket annotations, dispatched on the body keyword:
//!   fixed keyword (`改ページ` / `地付き` / ...), kaeriten
//!   (`一`/`二`/... plus okurigana `（X）`), indent / align-end
//!   (`N字下げ` / `地からN字上げ`), sashie (`挿絵`), forward-ref
//!   bouten, forward-ref TCY, paired-container open / close, and
//!   an `Directive{Unknown}` catch-all.
//! * Gaiji — `※［＃...］` reference-mark + bracket combos.
//! * Double-angle quotation `≪…≫` (displayed as `《…》`) (`AngleQuote`).
//!
//! The catch-all makes every well-formed `［＃…］` bracket produce
//! *some* `Node`, so the Tier-A canary (no bare `［＃` in the
//! HTML output outside an `aozora-directive` wrapper) holds regardless
//! of which specialised recogniser claims the bracket.
//!
//! ## Inlining note (negative result)
//!
//! `classify_subsystems` (instrumented) reports 88 % of classify wall in
//! "iterator-dispatch overhead" and only 9.4 % in actual recogniser
//! leaves. The straightforward fix — sprinkle `#[inline]` on
//! `recognize_and_emit` / `try_ruby_emit` / `try_bracket_emit` /
//! `try_gaiji_emit` / `process_event` / `handle_top_level` /
//! `handle_stream_event` / `Iterator::next` — was tried and reverted:
//! aggressive inlining regressed throughput by 1–6 % across all
//! bands. Selective inline (only the *small* helpers `push_output` /
//! `flush_plain_up_to` / `append_to_frame` / `pending_outputs_pop_front`
//! plus Phase 1 `flush_text` / `pair_text_then` / `try_merge_double`)
//! brought it within ±1.3 % of baseline — neutral.
//!
//! Conclusion: **LLVM's default inline judgement is already optimal
//! on this code at -O3 + fat-LTO.** Forcing inline on
//! recogniser-wrapping dispatchers regresses via i-cache thrash
//! (the wrappers expand into the per-call hot path and code bloat
//! dominates the dispatch saving). The 88 % "overhead" reported by
//! the instrumented build is partly the instrumentation itself
//! (`Instant::now()` per guard); the production overhead is real but
//! attacking it with attributes alone doesn't move it.
//!
//! The remaining headroom requires *structural* changes (Vec-passing
//! between phases, removing iterator chains) rather than attribute
//! hints.

use core::mem;
use core::ops::Range;
use std::collections::VecDeque;

#[cfg(feature = "classify-instrument")]
use super::instrumentation::{
    Subsystem, SubsystemGuard, YieldKind, record_pending_size, record_replay_body_size,
    record_yield,
};

// Phase 3 builds borrowed AST directly via `BorrowedAllocator`'s
// inherent methods. The `NodeAllocator` trait abstraction was retired
// in F.4 once the owned-AST path was gone.
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed;
use aozora_syntax::{ContainerKind, DirectiveKind, Span};

use super::pair::{PairEvent, PairKind};
use super::token::TriggerKind;
use aozora_spec::Diagnostic;

mod directive;
mod forward;
mod gaiji;
mod kaeriten;
pub use directive::build_body_dispatcher;
pub(crate) use directive::prewarm;
use forward::install_forward_target_index_from_source;
use kaeriten::{KaeritenObs, classify_kaeriten_mark, family_index, looks_like_kana_prose};

/// One classified slice of the sanitized source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedSpan<'a> {
    pub kind: SpanKind<'a>,
    pub source_span: Span,
}

/// Classification of a [`ClassifiedSpan`].
///
/// Phase 4 (now folded into `aozora_lex::lex`'s
/// `ArenaNormalizer` walk) maps the variants to PUA sentinels as
/// follows:
///
/// | variant        | sentinel              | `post_process` role |
/// |----------------|-----------------------|-------------------|
/// | `Plain`        | verbatim source bytes | — |
/// | `Newline`      | verbatim `\n`         | — |
/// | `Aozora(n)`    | `E001` if inline, `E002` if block-leaf | splice Aozora node into the AST |
/// | `BlockOpen`    | `E003`                | pair with matching `BlockClose` |
/// | `BlockClose`   | `E004`                | close nearest unclosed `BlockOpen` |
///
/// The `BlockOpen` / `BlockClose` split exists because paired
/// containers (`ここから字下げ` … `ここで字下げ終わり`) span arbitrary
/// content between the two markers. The lexer emits both markers as
/// independent spans and lets `post_process` walk the AST to wrap
/// sibling nodes in the container.
///
/// # Memory layout
///
/// The `Aozora(borrowed::Node<'a>)` variant is *not* boxed —
/// `borrowed::Node<'a>` is `Copy` and 16 bytes, so storing it
/// inline keeps `SpanKind` to `Aozora`-variant size while avoiding
/// the `Box` indirection the legacy owned shape paid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanKind<'a> {
    /// Source bytes that carry no Aozora construct. Emitted verbatim
    /// by the normalizer.
    Plain,
    /// Classified Aozora construct (inline span or block-leaf line).
    /// The normalizer replaces the source span with an `E001` (inline)
    /// or `E002` (block-leaf) sentinel and records the node in the
    /// placeholder registry keyed at the sentinel's normalized
    /// position.
    Aozora(borrowed::Node<'a>),
    /// Paired-container opener — `［＃ここから字下げ］`, `［＃罫囲み］`,
    /// etc. The normalizer emits an `E003` sentinel line; `post_process`
    /// matches it to the corresponding `BlockClose` via a balanced
    /// stack walk of the AST.
    BlockOpen(ContainerKind),
    /// Paired-container closer — `［＃ここで字下げ終わり］`,
    /// `［＃罫囲み終わり］`, etc. The normalizer emits an `E004`
    /// sentinel line; the carried `ContainerKind` is a hint used by
    /// `post_process` to diagnose `［＃罫囲み終わり］` closing an
    /// `Indent` opener (kind mismatch).
    BlockClose(ContainerKind),
    /// A `\n` in the sanitized text. Retained as its own span kind
    /// because block-level recognizers need line boundaries.
    Newline,
}

/// Classify a streaming Phase 2 [`PairEvent`] iterator against the
/// sanitized source.
///
/// Returns a [`ClassifyStream`] iterator yielding one [`ClassifiedSpan`]
/// per call to [`Iterator::next`]. After exhaustion, call
/// [`ClassifyStream::take_diagnostics`] to drain non-fatal observations
/// accumulated during recognition. The upstream pair stream's
/// diagnostics are NOT forwarded automatically — the caller is
/// responsible for calling `pair_stream.take_diagnostics()` after the
/// classify stream is dropped (the fused pipeline in `aozora-lex` does
/// this).
///
/// Pure function; no I/O. The yielded spans byte-contiguously cover
/// `source` — see the module-level span-coverage invariant.
#[must_use]
pub fn classify<'src, 'al, 'a, I>(
    events: I,
    source: &'src str,
    alloc: &'al mut BorrowedAllocator<'a>,
) -> ClassifyStream<'src, 'al, 'a, I::IntoIter>
where
    I: IntoIterator<Item = PairEvent>,
{
    // Pre-pass: scan raw source bytes for `「…」` quote bodies and
    // record the FIRST byte position of each unique body. The streaming
    // pipeline never materialises a `Vec<PairEvent>`, so the legacy
    // event-driven AC pre-pass (which walked the event slice to collect
    // forward-reference targets) doesn't fit; this source-byte variant
    // is event-free and pays one extra `memmem` sweep per document
    // before classification starts. Only installed when the source has
    // enough quote bodies to amortise the build (the median corpus doc
    // skips the index entirely; the pathological annotation-dense
    // 252-occurrence doc reclaims the 170 ms → 20 ms classify win this
    // index used to give the legacy event-driven pre-pass).
    install_forward_target_index_from_source(source);
    ClassifyStream::new(events.into_iter(), source, alloc)
}

/// Streaming Phase 3 classifier.
///
/// Owns the upstream [`PairEvent`] iterator and consumes it lazily,
/// yielding one [`ClassifiedSpan`] per [`Iterator::next`] call. The
/// classifier maintains its own per-pair frame stack — when a top-level
/// `PairOpen` arrives, all subsequent events accumulate into a smallvec
/// body buffer until the matching `PairClose`; recognition then runs
/// against the buffer and yields a single span (or, in the rare
/// gaiji+ref-mark case, consumes a buffered `Solo(RefMark)` from the
/// previous emission and folds it into the bracket span).
///
/// State:
/// * `pending_outputs`: queue of complete `ClassifiedSpan`s waiting to
///   be returned by `next()`. A single input event can produce multiple
///   outputs (e.g. flush a pending Plain run + emit a recognised span);
///   draining this queue first keeps `next` simple.
/// * `frame`: current outermost open frame, if any. Inside a frame all
///   incoming events are appended to the body buffer; nested
///   `PairOpen`/`PairClose` adjust the buffer-local stack so `close_idx`
///   slots can be patched and the OUTER pair can be detected as
///   "matching close at depth 0".
/// * `pending_plain_start`: byte position where the current Plain run
///   began (top-level only).
/// * `pending_refmark`: a top-level `Solo(RefMark)` waiting to be
///   absorbed by the next `PairOpen(Bracket)` (gaiji shape). If the
///   following event is anything else the refmark is folded into the
///   pending Plain run.
/// * `diagnostics`: non-fatal observations accumulated during the pass.
#[allow(
    missing_debug_implementations,
    reason = "the &mut BorrowedAllocator field cannot derive Debug; the iterator is opaque to consumers"
)]
pub struct ClassifyStream<'src, 'al, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    events: I,
    source: &'src str,
    source_len: u32,
    alloc: &'al mut BorrowedAllocator<'a>,
    /// Buffered ready-to-yield spans drained one-per-`next()` by the
    /// consumer. `VecDeque` (not `SmallVec`) because the consumer pulls
    /// from the front and `SmallVec::remove(0)` is `O(N)`: the
    /// `replay_unrecognised_body` path can push thousands of spans at
    /// once for top-level unrecognised paired containers (e.g. doc 49178
    /// in the corpus emits ~16k pending spans), and the per-yield
    /// front-pop turns into a quadratic memmove storm. `VecDeque` is a
    /// ring buffer with `O(1)` `push_back` / `pop_front`, eliminating
    /// the back-shift entirely.
    pending_outputs: VecDeque<ClassifiedSpan<'a>>,
    frame: Option<Frame>,
    /// Stream-through state for top-level pair kinds that have no
    /// recogniser (Quote, Tortoise). `None` in normal operation. When
    /// `Some((kind, depth))`, `process_event` bypasses frame buffering
    /// — events stream directly through `handle_stream_event` so we
    /// don't waste an O(N) `SmallVec` push per event followed by an
    /// O(N) replay walk. The depth counter tracks nested opens of the
    /// same kind so the outer close is unambiguous.
    ///
    /// Optimisation: doc 49178 (corpus outlier) wraps ~24k inner
    /// events in two top-level Quote pairs. With buffering the
    /// classify pass paid 24k × (push to body smallvec + read it back
    /// in replay); with stream-through that becomes a single forward
    /// walk.
    streaming: Option<StreamingFrame>,
    pending_plain_start: Option<u32>,
    pending_refmark: Option<Span>,
    diagnostics: Vec<Diagnostic>,
    /// Bracketed kaeriten observed in document order, drained by
    /// [`Self::finalize_kaeriten`] in [`Self::take_diagnostics`] for the
    /// document-wide base-presence pairing check and the outside-kanbun
    /// heuristic.
    kaeriten_obs: Vec<KaeritenObs>,
    finished: bool,
}

/// Active stream-through frame for a top-level Quote / Tortoise pair.
/// Carries no event buffer — just the outer pair kind and a nested-
/// open depth counter. Each `PairOpen` of the same kind increments,
/// each `PairClose` of the same kind decrements; reaching zero ends
/// stream-through.
#[derive(Debug, Clone, Copy)]
struct StreamingFrame {
    kind: PairKind,
    depth: u32,
}

/// Body window passed to recogniser helpers.
///
/// `events` is a contiguous body slice (between matched
/// `PairOpen`/`PairClose`); `links[i]` gives the body-local index of
/// the matching `PairOpen`/`PairClose` for `events[i]` if it's a
/// paired event (`u32::MAX` otherwise). Both slices are the same
/// length and are constructed by [`ClassifyStream`]'s frame buffers.
///
/// The split keeps [`PairEvent`] free of cross-link fields (Phase 2
/// can stream events one-at-a-time without back-patching) while still
/// giving recogniser helpers O(1) "jump to my matching delimiter"
/// access via the parallel side-table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyView<'b> {
    pub events: &'b [PairEvent],
    pub links: &'b [u32],
}

/// Per-recognise-call shared context.
///
/// Bundles the (allocator, sanitized source) pair that every Phase 3
/// recogniser / classifier helper needs but doesn't vary per call
/// within a single recognise pass. Threading it as a single `&mut`
/// argument keeps each helper's signature at ≤4 args (project-rule
/// `clippy.toml::too-many-arguments-threshold = 4`) without losing
/// the per-call positional clarity of the body-window indices.
///
/// The three lifetimes deliberately stay distinct:
/// - `'al` — the borrow lifetime of the `&mut alloc` reference
/// - `'a`  — the arena lifetime that strings interned now will live in
/// - `'s`  — the sanitized source lifetime
///
/// In practice `'a` and `'s` collapse at the top-level driver
/// (the arena and the source both live as long as the `Document`),
/// but keeping them separate here avoids over-constraining helpers
/// that thread synthetic source slices through `Cow`.
pub(crate) struct RecogniseCtx<'al, 'a, 's> {
    pub alloc: &'al mut BorrowedAllocator<'a>,
    pub source: &'s str,
    /// Non-fatal diagnostics raised while building *nested* content
    /// (a gaiji inside a ruby / annotation reading). The owning
    /// `ClassifyStream` drains this into its own sink after each
    /// recognise call — a `RecogniseCtx` is a short-lived per-call view,
    /// so an owned `Vec` avoids threading a `&mut` sink (and a fourth
    /// lifetime) through every recogniser.
    pub diagnostics: Vec<Diagnostic>,
}

/// One outermost open-pair frame currently being buffered.
///
/// `body` holds every event seen between the open and the matching
/// close (inclusive of nested Pair events). The parallel `links`
/// smallvec records, for each entry of `body`, the body-local index
/// of the matching `PairOpen` / `PairClose` (or `u32::MAX` for
/// non-paired entries and unmatched delimiters). The recognise
/// helpers (`recognize_ruby` / `recognize_annotation` /
/// `recognize_gaiji` / `try_angle_quote`) consume the buffer as a
/// [`BodyView`] with `open_idx = 0` and `close_idx = body.len() - 1`.
///
/// `inner_stack` tracks the per-buffer mini-stack of nested opens —
/// `(kind, body_index)` — so that on each nested close we can locate
/// the matching open in the body buffer and patch the `links` table.
struct Frame {
    body: smallvec::SmallVec<[PairEvent; 16]>,
    links: smallvec::SmallVec<[u32; 16]>,
    inner_stack: smallvec::SmallVec<[(PairKind, usize); 8]>,
    /// `true` when the outer open follows a `Solo(RefMark)` and the
    /// frame should be recognised as gaiji rather than a generic
    /// bracket annotation.
    gaiji_refmark: Option<Span>,
}

/// Span of the first ruby (`《…》`) opening *inside* the body
/// event range `lo..hi`, if any. Used by [`ClassifyStream::try_ruby_emit`]
/// to flag `nested_ruby` — `build_content_from_body` folds only nested
/// gaiji / annotation, so an inner ruby open would otherwise survive raw.
fn first_nested_ruby_open(events: &[PairEvent], lo: usize, hi: usize) -> Option<Span> {
    events[lo..hi].iter().find_map(|e| match e {
        PairEvent::PairOpen {
            kind: PairKind::Ruby,
            span,
        } => Some(*span),
        _ => None,
    })
}

/// When an explicit-base ruby (`｜base《》`) has an empty reading, the span
/// of the whole `｜base《》` to flag as `empty_ruby_reading`. Returns `None`
/// when there is no explicit `｜` base (a bare `《》` is just literal text)
/// or when the `《…》` reading is non-empty.
fn empty_explicit_ruby_span(
    bar_byte_offset: Option<usize>,
    preceding_start: u32,
    open_span: Span,
    close_span: Span,
) -> Option<Span> {
    let bar_off = bar_byte_offset?;
    if open_span.end < close_span.start {
        return None; // reading carries bytes — not empty
    }
    let bar_pos = preceding_start + u32::try_from(bar_off).ok()?;
    Some(Span::new(bar_pos, close_span.end))
}

/// Build the synthetic event / link stream `recognize_ruby` expects in the
/// streaming model: `[optional Solo(Bar), Text(base), ...body events...]`,
/// with links shifted to account for the prepended prefix. Returns the
/// stream, its parallel link table, and the index of the synthetic `《`
/// open. `prev_text_range` is the preceding plain run (`start` = its byte
/// offset, `end` = the `《` open); `bar_byte_offset` is the position of a
/// `｜` inside it for the explicit form. Returns `None` when an explicit
/// `｜` base would leave the base text empty.
fn build_synth_ruby_view(
    body: BodyView<'_>,
    prev_text_range: Span,
    bar_byte_offset: Option<usize>,
) -> Option<(Vec<PairEvent>, Vec<u32>, usize)> {
    let mut synth: Vec<PairEvent> = Vec::with_capacity(body.events.len() + 2);
    let mut synth_links: Vec<u32> = Vec::with_capacity(body.events.len() + 2);
    let synth_open_idx = if let Some(bar_off) = bar_byte_offset {
        let bar_pos = prev_text_range.start + u32::try_from(bar_off).expect("bar offset fits");
        let bar_span = Span::new(bar_pos, bar_pos + u32::try_from('｜'.len_utf8()).unwrap());
        synth.push(PairEvent::Solo {
            kind: TriggerKind::Bar,
            span: bar_span,
        });
        synth_links.push(u32::MAX);
        if bar_span.end >= prev_text_range.end {
            return None; // explicit base would be empty
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
    // Append the body events verbatim, shifting their links past the prefix.
    let shift = u32::try_from(synth.len()).expect("synth prefix fits u32");
    synth.extend(body.events.iter().cloned());
    synth_links.extend(
        body.links
            .iter()
            .map(|&l| if l == u32::MAX { u32::MAX } else { l + shift }),
    );
    Some((synth, synth_links, synth_open_idx))
}

impl<'src, 'al, 'a, I> ClassifyStream<'src, 'al, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn new(events: I, source: &'src str, alloc: &'al mut BorrowedAllocator<'a>) -> Self {
        Self {
            events,
            source,
            source_len: u32::try_from(source.len()).expect("sanitize asserts fit in u32"),
            alloc,
            pending_outputs: VecDeque::new(),
            frame: None,
            streaming: None,
            pending_plain_start: None,
            pending_refmark: None,
            diagnostics: Vec::new(),
            kaeriten_obs: Vec::new(),
            finished: false,
        }
    }

    /// Drain accumulated diagnostics. Should be called after the
    /// iterator is exhausted (otherwise the trailing Plain flush has
    /// not yet recorded any final-span observations).
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        self.finalize_kaeriten();
        mem::take(&mut self.diagnostics)
    }

    /// Document-final kaeriten checks, run once the event stream is
    /// exhausted: the document-wide base-presence pairing check
    /// (`bracketed_kaeriten_no_pair`) and the conservative
    /// outside-kanbun heuristic (`kaeriten_outside_kanbun`). Both work
    /// off `kaeriten_obs`, accumulated during classification.
    ///
    /// The pairing rule is *document-wide base presence*: a ladder mark
    /// of rank ≥ 2 fires only when its family's base (`一` / `上` / `甲`)
    /// is absent from the entire document. This is calibrated against the
    /// real 青空文庫 corpus — kanbun return-mark groups routinely span
    /// `、` / `。` and line boundaries, and 上下点 skips `中`, so any
    /// narrower scope or stricter ladder misfires on valid kanbun (per-
    /// clause strict: 586 false positives across 337 corpus files; this
    /// rule: 2). It matches the catalogue's literal wording — "a `［＃二］`
    /// with no `［＃一］`".
    fn finalize_kaeriten(&mut self) {
        let obs = mem::take(&mut self.kaeriten_obs);
        if obs.is_empty() {
            return;
        }
        // Conservative outside-kanbun heuristic: the document holds a
        // single, isolated kaeriten and its surroundings read as kana
        // prose — most likely a stray annotation, not a genuine 返り点.
        if let [only] = obs.as_slice()
            && looks_like_kana_prose(self.source, only.span)
        {
            self.diagnostics
                .push(Diagnostic::kaeriten_outside_kanbun(only.span));
        }
        // Document-wide base presence per ladder family.
        let mut has_base = [false; 3];
        for o in obs.iter().filter(|o| o.is_ladder && o.rank == 1) {
            has_base[family_index(o.family)] = true;
        }
        for o in obs.iter().filter(|o| o.is_ladder && o.rank > 1) {
            if !has_base[family_index(o.family)] {
                self.diagnostics
                    .push(Diagnostic::bracketed_kaeriten_no_pair(o.span));
            }
        }
    }

    fn push_output(&mut self, span: ClassifiedSpan<'a>) {
        #[cfg(feature = "classify-instrument")]
        record_yield(match &span.kind {
            SpanKind::Plain => YieldKind::Plain,
            SpanKind::Newline => YieldKind::Newline,
            SpanKind::Aozora(_) => YieldKind::Aozora,
            SpanKind::BlockOpen(_) => YieldKind::BlockOpen,
            SpanKind::BlockClose(_) => YieldKind::BlockClose,
        });
        self.pending_outputs.push_back(span);
    }

    /// Emit any pending top-level plain run whose end is `end`. The
    /// pending refmark, if any, is folded into the plain run's coverage
    /// (its span is contiguous with the surrounding text).
    fn flush_plain_up_to(&mut self, end: u32) {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::FlushPlain);
        // A pending refmark contributes its bytes to the plain run.
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

    /// Open a new top-level frame. `gaiji_refmark` is `Some(span)` when
    /// the outer open was preceded by a `Solo(RefMark)` waiting to be
    /// absorbed (the gaiji shape).
    fn open_frame(&mut self, open_event: PairEvent, gaiji_refmark: Option<Span>) {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::OpenFrame);
        let mut body: smallvec::SmallVec<[PairEvent; 16]> = smallvec::SmallVec::new();
        let mut links: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
        // Inner stack tracks NESTED opens; the outer open lives at
        // body[0], so we record its position there.
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

    /// Append an event to the current frame's body, updating the
    /// inner-stack and patching the parallel `links` side-table as
    /// needed. Returns `true` if the appended event closed the
    /// OUTERMOST pair (i.e. `inner_stack` became empty), signalling
    /// that the caller should run recognition on the now-complete
    /// buffer.
    fn append_to_frame(&mut self, event: PairEvent) -> bool {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::FrameAppend);
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
                // Find the matching open via the inner stack. Phase 2
                // guarantees that a PairClose only arrives when the
                // top of the global stack matches its kind, but inside
                // the body buffer we may have nested opens of various
                // kinds — we patch the nearest matching open.
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
                    // No matching open in this buffer — should not
                    // happen because Phase 2's stack-balance contract
                    // means a PairClose only arrives when the outer
                    // stack matches; but be defensive and append as-is.
                    frame.body.push(event);
                    frame.links.push(u32::MAX);
                }
            }
            _ => {
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
        }

        frame.inner_stack.is_empty()
    }

    /// Run recognition on the current frame's body buffer and emit the
    /// resulting span. Called when the OUTERMOST pair has just closed.
    fn recognize_and_emit(&mut self) {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::RecognizeAndEmit);
        let frame = self
            .frame
            .take()
            .expect("recognize_and_emit requires an active frame");
        let body = frame.body;
        let links = frame.links;
        debug_assert!(body.len() >= 2, "frame body must contain open + close");
        debug_assert_eq!(body.len(), links.len(), "links must parallel body");

        // The frame's outer open lives at body[0], the matching close
        // at body[body.len() - 1].
        let open_idx = 0usize;
        let close_idx = body.len() - 1;

        // Pull open span / kind for emission and pending-plain truncation.
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
                // Empty `≪≫` falls through to `replay_unrecognised_body`
                // so the bytes flow back into the pending plain run.
            }
            PairKind::Bracket => {
                let refmark = frame.gaiji_refmark;
                if let Some(rm_span) = refmark {
                    // B (zero-copy): pass the original body+links
                    // directly with `bracket_open_idx = 0`. The previous
                    // shape rebuilt body+links into a SmallVec just to
                    // prepend a synthetic Solo(RefMark) at index 0 so
                    // `recognize_gaiji` could read the bracket open at
                    // index 1. But `recognize_gaiji` parameterises the
                    // bracket entry point via `bracket_open_idx` and
                    // takes `refmark_span` as a separate argument — the
                    // synthetic prefix was never consumed. Eliminating
                    // the rebuild closes pathological doc 50685's
                    // memcpy_memmove 25.13 % bucket and doc 49178's
                    // 22.63 %, both attributed to this hot path.
                    if let Some(span) = self.try_gaiji_emit(view, open_idx, rm_span) {
                        self.push_output(span);
                        return;
                    }
                    // Gaiji recognition declined. Fold the refmark bytes
                    // into the pending plain run, then attempt a normal
                    // bracket annotation recognition on the original body.
                    if self.pending_plain_start.is_none() {
                        self.pending_plain_start = Some(rm_span.start);
                    }
                    if let Some(span) = self.try_bracket_emit(view, open_idx, close_idx) {
                        self.push_output(span);
                        return;
                    }
                    // Both gaiji and bracket annotation declined: replay
                    // the body and let the refmark span fall into plain.
                    self.replay_unrecognised_body(body, None);
                    return;
                }
                if let Some(span) = self.try_bracket_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            // Tortoise / Quote at top level have no built-in
            // recogniser; the bracket bytes flow through as plain.
            _ => {}
        }

        // Recognition declined — every event in the body becomes plain.
        // Replay the buffered events through the per-event acceptor so
        // that any Newlines inside fire as their own spans and the
        // surrounding bytes attach to a top-level Plain run. If the
        // frame was opened in gaiji-mode, the refmark span is also
        // folded back to plain.
        self.replay_unrecognised_body(body, frame.gaiji_refmark);
    }

    /// Replay the events from a frame whose recognition declined.
    /// Each event is treated as if it had been received at top level
    /// without a frame ever opening — text/solo/unmatched fold into
    /// the pending Plain run; newlines flush and fire as Newline
    /// spans.
    ///
    /// `refmark` is `Some(span)` when the frame was opened in
    /// gaiji-mode (Bracket preceded by `※`). The refmark bytes need
    /// to be re-folded into plain since gaiji recognition declined.
    ///
    /// `Unclosed` events are SKIPPED during replay: they are
    /// synthetic EOF markers carrying the same span as the original
    /// `PairOpen` (which is also in `body`), and re-adding their span
    /// to the pending plain run would double-count bytes already
    /// covered by the open's `body[0]` entry.
    fn replay_unrecognised_body(
        &mut self,
        body: smallvec::SmallVec<[PairEvent; 16]>,
        refmark: Option<Span>,
    ) {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::ReplayBody);
        #[cfg(feature = "classify-instrument")]
        record_replay_body_size(body.len() as u64);
        if let Some(rm) = refmark
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        for ev in body {
            if matches!(ev, PairEvent::Unclosed { .. }) {
                continue;
            }
            self.handle_top_level(ev, /*replay=*/ true);
        }
    }

    /// Handle a top-level event (no active frame) in either streaming
    /// mode (`replay = false`) or replay mode (`replay = true`, which
    /// suppresses the frame-open path so a residual nested `PairOpen` in
    /// a declined body doesn't try to re-open a sub-frame).
    fn handle_top_level(&mut self, event: PairEvent, replay: bool) {
        match event {
            PairEvent::Newline { pos } => {
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } if !replay => {
                // Hold the refmark pending the next event. If a flush
                // is requested before the next event arrives the
                // refmark is folded into the plain run.
                self.pending_refmark = Some(span);
            }
            PairEvent::PairOpen { kind, span, .. } if !replay => {
                // Stream-through: Quote and Tortoise have no
                // top-level recogniser. Buffering their body events
                // for an inevitable replay would burn O(N) work for
                // nothing — instead enter `streaming` mode and let
                // body events flow straight through. The open's bytes
                // become the seed of a fresh pending plain run.
                if matches!(kind, PairKind::Quote | PairKind::Tortoise) {
                    // Fold any pending refmark into plain first, then
                    // start the new plain run at the open's first byte.
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
                // Opening a top-level pair MAY flush the pending plain
                // up to (but not including) the open's start. The
                // refmark, if any and only when this open is a
                // Bracket, is absorbed into the frame; for any other
                // pair kind the refmark is folded into plain first.
                //
                // Ruby and AngleQuote are special: they consume the
                // preceding text (explicit `｜base《reading》` or
                // implicit trailing-kanji). We DON'T flush
                // `pending_plain_start` here so `try_ruby_emit` can
                // walk the preceding source bytes and decide how much
                // of the plain run the ruby actually swallows.
                //
                // Bracket joins the same list: forward-reference
                // `［＃「X」に傍点］` / `…は縦中横］` classifiers need
                // the chance to pull `consume_start` back over the
                // preceding target literal. `try_bracket_emit` then
                // calls `flush_plain_up_to(consume_start)` itself,
                // with the same effect for recognised brackets and a
                // single fused Plain run (literal + raw bracket bytes)
                // for the unrecognised path's replay — visually
                // identical to the pre-defer two-span shape.
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
                // Catch-all: every non-Newline event carries a span
                // and folds into the pending plain run.
                let Some(span) = other.span() else {
                    return;
                };
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
        }
    }

    /// Handle one event while in stream-through mode (top-level
    /// Quote / Tortoise pair, no recogniser candidate). Mirrors the
    /// `replay = true` behaviour of [`Self::handle_top_level`] but
    /// (a) reads from the live event stream rather than a buffered
    /// `SmallVec`, (b) tracks nested-open depth so the outer close
    /// unambiguously exits the mode, and (c) skips the inner-frame
    /// open path (a nested `Bracket` / `Ruby` / `AngleQuote` inside
    /// an unrecognised `Quote` folds into the surrounding plain run,
    /// same as the legacy buffered-replay behaviour).
    fn handle_stream_event(&mut self, event: PairEvent) {
        // Defensive — only called when streaming is Some.
        let stream = self
            .streaming
            .as_mut()
            .expect("handle_stream_event without streaming state");
        match event {
            PairEvent::Newline { pos } => {
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::PairOpen { kind, span } if kind == stream.kind => {
                stream.depth = stream.depth.saturating_add(1);
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
            PairEvent::PairClose { kind, span } if kind == stream.kind => {
                stream.depth = stream.depth.saturating_sub(1);
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
                if stream.depth == 0 {
                    self.streaming = None;
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                // Phase 2 emits synthetic Unclosed events when EOF
                // arrives mid-frame, one per still-open pair. Each
                // one's span aliases its original PairOpen — those
                // bytes were already folded into the plain run when
                // the PairOpen arrived (or emitted by an intervening
                // Newline flush). Re-folding here would set
                // `pending_plain_start` to a position *behind* the
                // cursor and break the tiling invariant.
                //
                // For Unclosed of the streaming kind, decrement depth
                // so the count mirrors the matching PairOpens (one
                // increment per open, one Unclosed per still-open).
                // When depth reaches zero the outer pair is gone, so
                // exit streaming. Other-kind Unclosed events (a
                // nested Bracket / Ruby / AngleQuote that streaming
                // mode never opened a frame for) are simply ignored.
                if kind == stream.kind {
                    stream.depth = stream.depth.saturating_sub(1);
                    if stream.depth == 0 {
                        self.streaming = None;
                    }
                }
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

    fn try_ruby_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::TryRubyEmit);
        // Ruby recognition uses the PRECEDING text (if any) as the
        // base — but in the streaming model we don't have that text in
        // the body buffer. We walk back through `pending_outputs` and
        // `pending_plain_start` to find it.
        //
        // The simplest correct approach: synthesise a body slice that
        // includes a single preceding Text event derived from the
        // current `pending_plain_start..open_span.start` range, plus
        // any `Solo(Bar)` if the explicit-ruby shape applies. This
        // mirrors what `recognize_ruby` expects:
        //   events[open_idx - 1] = Text { range: ... preceding ... }
        //   events[open_idx - 2] = optional Solo(Bar)
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return None;
        };

        // Nested ruby: a `《…》` opening *inside* the reading
        // body is an authoring error. Flag the first one (caret on the
        // inner `《`); the outer ruby still parses best-effort. Touched
        // before `ctx` reborrows `self.alloc`, so no borrow clash.
        if let Some(inner_open) = first_nested_ruby_open(body.events, open_idx + 1, close_idx) {
            self.diagnostics.push(Diagnostic::nested_ruby(inner_open));
        }

        // Determine the preceding plain run (the ruby base lives here) and
        // detect the explicit `｜` form by scanning it for a bar. The
        // streaming model has no preceding events, so we synthesise them.
        let preceding_start = self.pending_plain_start.unwrap_or(open_span.start);
        if preceding_start >= open_span.start {
            return None;
        }
        let prev_text_range = Span::new(preceding_start, open_span.start);
        let preceding_bytes = &self.source[preceding_start as usize..open_span.start as usize];
        let bar_byte_offset = preceding_bytes.rfind('｜');

        let (synth, synth_links, synth_open_idx) =
            build_synth_ruby_view(body, prev_text_range, bar_byte_offset)?;
        let synth_close_idx = synth_open_idx + (close_idx - open_idx);
        let synth_view = BodyView {
            events: &synth,
            links: &synth_links,
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
        };
        let Some(m) = ctx.recognize_ruby(synth_view, synth_open_idx, synth_close_idx) else {
            // `recognize_ruby` rejects an empty `《》` reading; flag the
            // explicit-base `｜base《》` shape (a bare `《》` stays silent).
            // The bytes fall through to plain replay either way. `ctx`'s
            // reborrow of `self.alloc` ended at the call above, so pushing
            // onto the disjoint `self.diagnostics` is borrow-clear.
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
            return None;
        };
        // Drain diagnostics raised while building the nested ruby reading.
        self.diagnostics.append(&mut ctx.diagnostics);
        // Truncate any in-progress plain run to end exactly where the ruby
        // takes over.
        self.flush_plain_up_to(m.consume_start);
        let base_content = self.alloc.content_plain(m.base);
        let node = self.alloc.ruby(base_content, m.reading, m.explicit);
        self.pending_plain_start = None;
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    /// Attempt to classify the buffered body as a `AngleQuote` node.
    ///
    /// Returns `None` when the body content is empty (`≪≫` with
    /// no payload) — the caller falls through to plain replay so the
    /// bytes show up as literal source. Emitting a `AngleQuote` span
    /// here would violate the [`borrowed::NonEmpty`] invariant on the
    /// `Content` payload.
    fn try_angle_quote_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan<'a>> {
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
        };
        let content = ctx.build_content_from_body(
            body,
            &BodyWindow {
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );
        // Drain diagnostics raised while building the nested angle-quote body.
        self.diagnostics.append(&mut ctx.diagnostics);
        // Empty `≪≫` is not a valid AngleQuote — let the bytes
        // flow through as plain text. The caller's fall-through path
        // (`replay_unrecognised_body`) handles the plain emission.
        if matches!(content, borrowed::Content::Plain(s) if s.is_empty())
            || matches!(content, borrowed::Content::Segments(segs) if segs.is_empty())
        {
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
    ) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::TryBracketEmit);
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
        };
        let m = ctx.recognize_annotation(body, open_idx, close_idx)?;
        // Drain diagnostics raised while building nested reading content
        // (a gaiji inside a left-ruby / annotation reading) into our sink.
        self.diagnostics.append(&mut ctx.diagnostics);
        self.flush_plain_up_to(m.consume_start);
        let kind = match m.emit {
            EmitKind::Aozora(node) => SpanKind::Aozora(node),
            EmitKind::BlockOpen(container) => SpanKind::BlockOpen(container),
            EmitKind::BlockClose(container) => SpanKind::BlockClose(container),
        };
        self.pending_plain_start = None;
        // Surface any non-fatal warning the recogniser attached
        // (unrecognised container directive / 縦中横 target not found /
        // ambiguous bouten target). The emitted node is unaffected — for
        // the catch-all cases it is still `Directive{Unknown}`, so the
        // Tier-A "no bare ［＃" canary holds. `ctx`'s reborrow of
        // `self.alloc` ended at `recognize_annotation`, so pushing onto the
        // disjoint `self.diagnostics` is borrow-clear.
        if let Some(diag) = m.pending_diagnostic {
            self.diagnostics.push(diag);
        }
        // Record bracketed kaeriten for the end-of-document pairing /
        // context checks (`finalize_kaeriten`). The directive span is the
        // whole `［＃…］`.
        if let SpanKind::Aozora(borrowed::Node::Kaeriten(k)) = kind {
            let span = Span::new(m.consume_start, m.consume_end);
            let (family, rank, is_ladder) = classify_kaeriten_mark(k.mark.as_str());
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
    ) -> Option<ClassifiedSpan<'a>> {
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
        };
        let m = ctx.recognize_gaiji(body, refmark_span, bracket_open_idx)?;
        self.flush_plain_up_to(m.consume_start);
        let node = self.alloc.gaiji(m.payload);
        self.pending_plain_start = None;
        // The gaiji still renders best-effort (as its description text)
        // when resolution misses; flag the miss so authors know the glyph
        // won't appear. `m.payload` is a `Copy` arena reference and the
        // `ctx` reborrow of `self.alloc` ended at the `gaiji()` call above,
        // so reading `ucs` and pushing onto `self.diagnostics` is clear of
        // the borrow. Scope: this `unresolved-gaiji` warning fires for
        // top-level `※［＃…］` only. Gaiji nested in a ruby reading / annotation
        // body are still resolved + rendered (by `build_content_from_body`),
        // but without this diagnostic; a gaiji buried in a forward-reference
        // quote target (nested `［＃…］` breaks pairing) falls to `Unknown`.
        if m.payload.ucs.is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    m.consume_start,
                    m.consume_end,
                )));
        }
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    /// Final flush: emit any trailing Plain run covering the source
    /// tail. Called once when the upstream iterator hits None.
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

impl<'a, I> Iterator for ClassifyStream<'_, '_, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    type Item = ClassifiedSpan<'a>;

    fn next(&mut self) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::IterDispatch);
        loop {
            if let Some(span) = self.pending_outputs_pop_front() {
                return Some(span);
            }
            if self.finished {
                return None;
            }
            #[cfg(feature = "classify-instrument")]
            let events_next_guard = SubsystemGuard::new(Subsystem::EventsNext);
            let next_event = self.events.next();
            #[cfg(feature = "classify-instrument")]
            drop(events_next_guard);
            if let Some(event) = next_event {
                #[cfg(feature = "classify-instrument")]
                let _phase3_loop_guard = SubsystemGuard::new(Subsystem::LoopBody);
                self.process_event(event);
            } else {
                // Upstream exhausted. Close any active frame as
                // unclosed (its body events fold back to plain;
                // a gaiji-mode refmark also falls into plain),
                // then run final flush.
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

impl<'a, I> ClassifyStream<'_, '_, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn pending_outputs_pop_front(&mut self) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "classify-instrument")]
        {
            // Record pending_outputs.len() BEFORE the pop so the
            // distribution histogram tracks pre-pop sizes.
            let len = self.pending_outputs.len() as u64;
            if len > 0 {
                record_pending_size(len);
            }
        }
        self.pending_outputs.pop_front()
    }

    fn process_event(&mut self, event: PairEvent) {
        // Stream-through path for top-level Quote / Tortoise — see
        // `StreamingFrame` for the rationale. Bypasses both frame
        // buffering AND replay; events flow straight through.
        if self.streaming.is_some() {
            self.handle_stream_event(event);
            return;
        }
        if self.frame.is_some() {
            // Inside a frame: every event accumulates. A pending
            // refmark cannot exist while a frame is open (frames are
            // opened from top level and the refmark would have been
            // absorbed or flushed there).
            debug_assert!(
                self.pending_refmark.is_none(),
                "frames are opened from top level; any pending refmark should have been absorbed or flushed before frame entry"
            );
            let outer_closed = self.append_to_frame(event);
            if outer_closed {
                self.recognize_and_emit();
            }
            return;
        }

        // Top level. If a refmark is pending, decide based on the
        // current event:
        if self.pending_refmark.is_some() {
            if let PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            } = &event
            {
                // Will be absorbed by the next handle_top_level call.
            } else {
                // Refmark not followed by Bracket: fold into plain
                // up to the end of the refmark, then continue
                // processing the new event normally. The refmark's
                // span gets absorbed by `flush_plain_up_to` because
                // we set `pending_plain_start` to `rm.start` before
                // taking it.
                let rm = self.pending_refmark.take().expect("checked Some");
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(rm.start);
                }
            }
        }

        self.handle_top_level(event, /*replay=*/ false);
    }
}

/// Intermediate result of `recognize_ruby`. `base` stays borrowed
/// (the two forms we handle — explicit `｜X《Y》` and implicit
/// trailing-kanji — both come from a single [`PairEvent::Text`] event
/// with no nested structure). `reading`, on the other hand, can carry
/// embedded gaiji (`※［＃…］`) or annotations (`［＃ママ］`), so it is
/// already resolved into a `Content` via `build_content_from_body`.
///
/// Collapsing inside the lexer (rather than leaving the splitting to
/// the renderer) keeps the [`borrowed::Node`] payload self-contained:
/// Phase 4 stamps one PUA sentinel over the whole `｜…《…》` source
/// span, and the inner gaiji/annotation never reach the top-level
/// `spans` list or downstream consumers.
struct RubyMatch<'s, 'a> {
    base: &'s str,
    reading: borrowed::Content<'a>,
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
/// The `《…》` reading body is walked with `build_content_from_body`
/// so nested `※［＃…］` gaiji and `［＃…］` annotations fold into the
/// returned `Content` as `Segment::Gaiji` / `Segment::Directive`.
/// Pure-text readings collapse back to `Content::Plain` via
/// `Content::from_segments`.
///
/// Returns `None` if neither shape applies (empty reading, no
/// preceding Text, no kanji for implicit).
impl<'a, 's> RecogniseCtx<'_, 'a, 's> {
    fn recognize_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<RubyMatch<'s, 'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Ruby);
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
        let prev_text = &self.source[prev_range.start as usize..prev_range.end as usize];

        let reading = self.build_content_from_body(
            view,
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
}

/// Half-open window into a [`PairEvent`] stream. Bundles the event-
/// index range with the matching byte-offset range so
/// `build_content_from_body` can flush text segments using source
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
/// `Content`.
///
/// Each nested `※［＃description、mencode］` reduces to a
/// `Segment::Gaiji` via `recognize_gaiji`; each standalone
/// `［＃…］` reduces to a `Segment::Directive` via
/// `recognize_annotation`. Every other byte (plain text, stray
/// triggers, unmatched delimiters) is captured into adjacent
/// `Segment::Text` runs by tracking a single "outstanding text
/// start" byte offset and flushing only when a recognisable construct
/// consumes the intervening bytes.
///
/// Non-Directive Aozora emits (a paired-container opener, a block
/// leaf, etc.) are *not* first-class segments and are folded back
/// into `Directive{Unknown}` with the raw bracket bytes — this keeps
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
/// `Content::from_segments`, so a slow-path body that turned out to
/// contain only text (for example because its brackets were malformed
/// and skipped) still collapses back to `Content::Plain`.
/// Immutable per-call body-walk context shared across the
/// `build_content_from_body` orchestrator and its per-shape helpers
/// (`try_emit_gaiji_at` / `try_emit_annotation_at`). Bundling `view`
/// and `window` together prevents the per-helper signatures from
/// exceeding the project's 4-arg threshold without losing positional
/// clarity at call sites.
#[derive(Clone, Copy)]
struct BodyWalkCtx<'b> {
    view: BodyView<'b>,
    window: &'b BodyWindow,
}

/// Mutable per-call build state for `build_content_from_body`.
/// Tracks the under-construction segment vector and the byte position
/// where the current pending Text run started. Threading these through
/// per-shape helpers as a single `&mut` field keeps the helper
/// signatures at ≤4 args.
struct ContentBuild<'a> {
    segments: Vec<borrowed::Segment<'a>>,
    /// Byte position of the earliest source byte not yet committed
    /// to a `Segment::Text`. Each successful gaiji / annotation emit
    /// advances this past the consumed bracket.
    text_start: u32,
}

impl<'a> RecogniseCtx<'_, 'a, '_> {
    /// Build a borrowed [`Content`](borrowed::Content) for the body
    /// window, recognising any nested gaiji / annotation constructs in
    /// a single forward sweep.
    ///
    /// Fast path returns when the body has no `※` and no `［` —
    /// emits the raw byte run as a single `Plain`. The slow path
    /// dispatches each event index through two per-shape recognise
    /// helpers and falls through (advancing `i`) when neither claims
    /// the slot.
    fn build_content_from_body(
        &mut self,
        view: BodyView<'_>,
        window: &BodyWindow,
    ) -> borrowed::Content<'a> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::BuildContent);
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
        if !has_nested_candidate(body_events) {
            // Fast path: no `※` and no `［` in the body; bytes pass
            // through verbatim. `content_plain("")` canonicalises to
            // empty `Segments(&[])` to match the legacy
            // `Content::from(&str)` shape exactly.
            let text = &self.source[window.bytes.start as usize..window.bytes.end as usize];
            return self.alloc.content_plain(text);
        }

        // Slow path: at least one potential nested construct exists.
        // Pre-size the segment vector: worst case is `ceil(n / 2)` runs
        // of `Text, Construct, Text, …` plus one trailing Text.
        // `body_events.len() + 1` is a safe upper bound that is small
        // in practice (ruby readings almost never reach double-digit
        // events).
        let body = BodyWalkCtx { view, window };
        let mut build = ContentBuild {
            segments: Vec::with_capacity(body_events.len() + 1),
            text_start: window.bytes.start,
        };
        let mut i = window.events.start;
        while i < window.events.end {
            if let Some(next_i) = self.try_emit_gaiji_at(body, &mut build, i) {
                i = next_i;
                continue;
            }
            if let Some(next_i) = self.try_emit_annotation_at(body, &mut build, i) {
                i = next_i;
                continue;
            }
            i += 1;
        }
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..window.bytes.end,
            self.alloc,
        );
        self.alloc.content_segments(&build.segments)
    }

    /// Shape 1: `※［＃…］` — `Solo(RefMark)` immediately followed by a
    /// matched `PairOpen(Bracket)`. On a successful gaiji recognise,
    /// flush the pending Text run, push a `Segment::Gaiji`, advance
    /// `text_start`, and return the index of the first event past the
    /// bracket close. Returns `None` if the shape doesn't match or if
    /// the inner [`Self::recognize_gaiji`] bails.
    fn try_emit_gaiji_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild<'a>,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::Solo {
            kind: TriggerKind::RefMark,
            span: refmark_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let bracket_idx = i + 1;
        if bracket_idx >= body.window.events.end {
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
        if close_idx >= body.window.events.end {
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
        // A nested gaiji whose mencode resolves nothing renders best-effort as
        // its description; flag the miss so nested references match the
        // top-level `※［＃…］` behaviour (#84). The owning `ClassifyStream`
        // drains `self.diagnostics` after the recognise call.
        if g.payload.ucs.is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    g.consume_start,
                    g.consume_end,
                )));
        }
        Some(close_idx + 1)
    }

    /// Shape 2: `［＃…］` — a standalone bracket annotation. Tried
    /// after [`Self::try_emit_gaiji_at`] so the `※`+bracket combo gets
    /// first claim on a leading bracket. `recognize_annotation` has an
    /// `Unknown` catch-all (only returns `None` for malformed brackets
    /// with no `＃` sentinel); on success the bracket folds into a
    /// `Segment::Directive` with the recogniser's payload (or a
    /// synthetic `Unknown` payload built from the raw source bytes
    /// when the recogniser left `annotation_payload` unset). The
    /// fallback synthesis preserves the Tier-A canary: no bare `［＃`
    /// ever leaks outside an `aozora-directive` wrapper.
    fn try_emit_annotation_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild<'a>,
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
        if close_idx >= body.window.events.end {
            return None;
        }
        let a = self.recognize_annotation(body.view, i, close_idx)?;
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.view.events[close_idx]
        else {
            // Phase 2 invariant: PairOpen's link always targets a
            // PairClose of the matching kind.
            unreachable!("PairOpen link must target a PairClose");
        };
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..a.consume_start,
            self.alloc,
        );
        // A no-`※` standalone gaiji (#122) reaches here as `Aozora(Gaiji)`;
        // wrap it as a `Segment::Gaiji` (not the `Unknown` annotation the
        // payload fallback would build) and flag an unresolved miss (#84).
        // Every other recogniser keeps the `Segment::Directive` path.
        if let EmitKind::Aozora(borrowed::Node::Gaiji(g)) = a.emit {
            build.segments.push(self.alloc.seg_gaiji(g));
            if g.ucs.is_none() {
                self.diagnostics
                    .push(Diagnostic::unresolved_gaiji(Span::new(
                        a.consume_start,
                        a.consume_end,
                    )));
            }
        } else {
            let payload = if let Some(p) = a.annotation_payload {
                p
            } else {
                let raw = &self.source[open_span.start as usize..close_span.end as usize];
                self.alloc.make_directive(raw, DirectiveKind::Unknown)
            };
            build.segments.push(self.alloc.seg_annotation(payload));
        }
        build.text_start = a.consume_end;
        Some(close_idx + 1)
    }
}

/// Whether `body` could host a nested gaiji / annotation. The Phase 2
/// event model guarantees that:
///
/// * `※［＃…］` always emits a `Solo(RefMark)` event at its `※`.
/// * `［＃…］` always emits a `PairOpen(Bracket)` event at its `［`.
///
/// So the absence of both event shapes in the body is sufficient proof
/// that no nested construct can be recognised, allowing
/// `build_content_from_body` to take the allocation-free fast path.
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
fn push_text_segment<'a>(
    segments: &mut Vec<borrowed::Segment<'a>>,
    source: &str,
    bytes: Range<u32>,
    alloc: &mut BorrowedAllocator<'a>,
) {
    if !bytes.is_empty() {
        segments.push(alloc.seg_text(&source[bytes.start as usize..bytes.end as usize]));
    }
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

/// Intermediate result of `recognize_annotation`.
///
/// `emit` decides which [`SpanKind`] the driver pushes for the
/// top-level case. `annotation_payload` is `Some` exactly when the
/// recogniser produced an `Directive{…}` payload — the
/// `build_content_from_body` caller uses it to wrap the same payload
/// as a `Segment::Directive` without reconstructing it. The emit
/// variants `BlockOpen` / `BlockClose` and non-`Directive` `Aozora`
/// nodes leave `annotation_payload` as `None`, so the body-builder
/// falls back to its `Directive{Unknown}` synthesis path.
struct AnnotationMatch<'a> {
    emit: EmitKind<'a>,
    annotation_payload: Option<&'a borrowed::Directive<'a>>,
    consume_start: u32,
    consume_end: u32,
    /// A non-fatal warning to surface for this bracket, if any —
    /// `unrecognised_container_directive`, `tcy_target_not_found`, or
    /// `bouten_target_ambiguous`. The caller drains it into the diagnostic
    /// stream; the emitted node (often the `Directive{Unknown}` catch-all,
    /// canary intact) is unaffected.
    pending_diagnostic: Option<Diagnostic>,
}

/// What to emit for a matched annotation.
enum EmitKind<'a> {
    /// Inline or block-leaf — becomes [`SpanKind::Aozora`].
    Aozora(borrowed::Node<'a>),
    /// Paired-container opener — becomes [`SpanKind::BlockOpen`].
    BlockOpen(ContainerKind),
    /// Paired-container closer — becomes [`SpanKind::BlockClose`].
    BlockClose(ContainerKind),
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

    use super::directive::parse_emphasis_body;
    use super::forward::emphasis_kind_from_suffix;
    use super::*;
    use aozora_syntax::{AlignEnd, BoutenKind, BoutenPosition, EmphasisKind, Indent, SectionKind};
    // Borrowed-AST types pattern-matched throughout. `Node<'a>`
    // is `Copy` and holds payloads via `&'a Ruby<'a>` etc., so tests
    // pattern-match `Node::Ruby(r)` where `r` is already a
    // reference — no `Box` deref needed.
    #[allow(
        unused_imports,
        reason = "individual tests pattern-match on subsets; bringing them all in keeps the import block stable"
    )]
    use aozora_syntax::borrowed::{
        AngleQuote, Arena, Bouten, CombineUpright, Content, Directive, Gaiji, HeadingHint,
        Illustration, Kaeriten, Node, Ruby, Segment,
    };

    use crate::lexer::pair::pair;
    use crate::lexer::tokenize::tokenize;

    /// Test-only materialised classify output: collects `spans` from
    /// the streaming iterator and merges its post-exhaustion
    /// diagnostics with the upstream pair stream's diagnostics.
    /// Per-test convenience shape so tests can assert on the full
    /// pipeline result without rebuilding the collection inline at
    /// every site.
    #[derive(Debug)]
    struct TestClassifyOutput<'a> {
        spans: Vec<ClassifiedSpan<'a>>,
        diagnostics: Vec<Diagnostic>,
    }

    /// Test-only `run` macro. Materialises a fresh
    /// [`Arena`] / [`BorrowedAllocator`] pair in the calling scope and
    /// binds `out` (or the explicitly-named identifier) to a
    /// [`TestClassifyOutput`]. Replaces the legacy
    /// `let out = run(src)` shape so each test's borrow chain is
    /// arena-rooted in the test's own stack frame, with no per-test
    /// allocator boilerplate.
    macro_rules! run {
        ($name:ident, $src:expr) => {
            let arena = Arena::new();
            let mut alloc = BorrowedAllocator::new(&arena);
            let mut pair_stream = pair(tokenize($src));
            let mut spans: Vec<ClassifiedSpan<'_>> = Vec::new();
            let classify_diagnostics: Vec<Diagnostic> = {
                let mut stream = classify(&mut pair_stream, $src, &mut alloc);
                for span in &mut stream {
                    spans.push(span);
                }
                stream.take_diagnostics()
            };
            let mut diagnostics = pair_stream.take_diagnostics();
            diagnostics.extend(classify_diagnostics);
            let $name = TestClassifyOutput { spans, diagnostics };
        };
    }

    /// Test-only helper: extract the `Aozora` variant's borrowed
    /// `Node<'a>` (which is `Copy`) so tests can pattern-match
    /// on it without spelling out the variant boilerplate at every
    /// call site.
    fn aozora_node<'a>(span: &ClassifiedSpan<'a>) -> Option<Node<'a>> {
        match span.kind {
            SpanKind::Aozora(node) => Some(node),
            _ => None,
        }
    }

    #[test]
    fn empty_input_produces_empty_span_vector() {
        run!(out, "");
        assert!(out.spans.is_empty());
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
    fn plain_multibyte_becomes_single_plain_span() {
        let src = "こんにちは";
        run!(out, src);
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(
            out.spans[0].source_span,
            Span::new(0, u32::try_from(src.len()).expect("fits"))
        );
    }

    #[test]
    fn newline_in_middle_splits_into_three_spans() {
        run!(out, "line1\nline2");
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
        run!(out, "\nbody\n");
        // Expected: Newline, Plain("body"), Newline. No empty Plain at the edges.
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].kind, SpanKind::Plain);
        assert_eq!(out.spans[2].kind, SpanKind::Newline);
    }

    #[test]
    fn explicit_ruby_produces_single_aozora_span() {
        let src = "｜青梅《おうめ》";
        run!(out, src);
        assert_eq!(out.spans.len(), 1);
        let SpanKind::Aozora(node) = out.spans[0].kind else {
            panic!("expected Aozora span, got {:?}", out.spans[0].kind);
        };
        let Node::Ruby(ruby) = node else {
            panic!("expected Ruby variant, got {node:?}");
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
        run!(out, src);
        assert_eq!(out.spans.len(), 2);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        let SpanKind::Aozora(node) = out.spans[1].kind else {
            panic!("expected Aozora span, got {:?}", out.spans[1].kind);
        };
        let Node::Ruby(ruby) = node else {
            panic!("expected Ruby variant, got {node:?}");
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
        run!(out, src);
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
        run!(out, src);
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
        run!(out, src);
        // Plain("line1"), Newline, Aozora(Ruby)
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        let is_ruby = matches!(out.spans[2].kind, SpanKind::Aozora(Node::Ruby(_)));
        assert!(
            is_ruby,
            "expected Aozora(Ruby), got {:?}",
            out.spans[2].kind
        );
    }

    #[test]
    fn implicit_ruby_after_non_text_event_is_not_recognized() {
        // A close-bracket between `」` and `《` means the preceding
        // event is PairClose, not Text. Implicit ruby can't bind.
        let src = "「台詞」《かんじ》";
        run!(out, src);
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
    fn only_ruby<'a>(out: &TestClassifyOutput<'a>) -> &'a Ruby<'a> {
        let mut found = None;
        for span in &out.spans {
            if let SpanKind::Aozora(Node::Ruby(r)) = span.kind {
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
        run!(out, "｜青梅《おうめ》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("青梅"));
        assert_eq!(r.reading.as_plain(), Some("おうめ"));
    }

    #[test]
    fn ruby_reading_with_embedded_gaiji_produces_segments() {
        // `※［＃「ほ」、第3水準1-85-54］` inside the reading must fold
        // into a `Segment::Gaiji` between Text segments so the renderer
        // can wrap it in `<span class="aozora-gaiji">` without leaking the
        // bare `［＃` marker (Tier A).
        run!(out, "｜日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("日本"));
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 3);
        assert!(
            matches!(&segs[0], Segment::Text(t) if &**t == "に"),
            "segment 0: {:?}",
            segs[0]
        );
        let Segment::Gaiji(g) = segs[1] else {
            panic!("segment 1 should be Gaiji, got {:?}", segs[1]);
        };
        assert_eq!(g.description, "ほ");
        assert_eq!(g.mencode, Some("第3水準1-85-54"));
        assert!(
            matches!(&segs[2], Segment::Text(t) if &**t == "ん"),
            "segment 2: {:?}",
            segs[2]
        );
    }

    /// A no-`※` standalone gaiji (#122) nested in a ruby reading folds into a
    /// `Segment::Gaiji` (not the `Unknown` annotation the payload fallback
    /// would otherwise build).
    #[test]
    fn nested_standalone_gaiji_folds_into_gaiji_segment() {
        run!(out, "｜日本《に［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        let Segment::Gaiji(g) = segs[1] else {
            panic!("segment 1 should be Gaiji, got {:?}", segs[1]);
        };
        assert_eq!(g.description, "ほ");
        assert!(g.standalone, "no `※` in source → standalone");
    }

    /// #84: an unresolved gaiji nested inside a ruby reading now raises the
    /// `unresolved-gaiji` warning, matching the top-level `※［＃…］` scan
    /// (previously the nested reference was silently best-effort rendered).
    #[test]
    fn nested_unresolved_gaiji_fires_diagnostic() {
        run!(out, "｜謎《な※［＃「謎＋字」、99-99-99］ぞ》");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnresolvedGaiji { .. })),
            "expected an unresolved-gaiji diagnostic, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn ruby_reading_wholly_gaiji_produces_single_gaiji_segment() {
        // No surrounding text; the reading is exactly one gaiji
        // marker. The Segments run must be a single Gaiji (not a
        // trailing empty Text on either side).
        run!(out, "｜日本《※［＃「にほん」、第3水準1-85-54］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 1);
        let Segment::Gaiji(g) = segs[0] else {
            panic!("expected Gaiji, got {:?}", segs[0]);
        };
        assert_eq!(g.description, "にほん");
    }

    #[test]
    fn ruby_reading_with_trailing_annotation_produces_annotation_segment() {
        // `［＃ママ］` inside a reading indicates editorial "sic" —
        // must fold as `Segment::Directive` so the renderer wraps it
        // in the hidden `aozora-directive` span (Tier A compliance).
        run!(out, "｜日本《にほん［＃ママ］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 2);
        assert!(
            matches!(&segs[0], Segment::Text(t) if &**t == "にほん"),
            "segment 0: {:?}",
            segs[0]
        );
        let Segment::Directive(a) = segs[1] else {
            panic!("segment 1 should be Directive, got {:?}", segs[1]);
        };
        assert_eq!(a.raw.as_str(), "［＃ママ］");
    }

    #[test]
    fn ruby_reading_with_gaiji_and_annotation_interleaved() {
        // Exercises the general Segments shape: Text, Gaiji, Text,
        // Directive. Proves the flusher preserves ordering and the
        // `text_start` advancement correctly spans each gap.
        run!(out, "｜日本《に※［＃「ほ」、第3水準1-85-54］ん［＃ママ］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 4);
        assert!(matches!(&segs[0], Segment::Text(t) if &**t == "に"));
        assert!(matches!(&segs[1], Segment::Gaiji(_)));
        assert!(matches!(&segs[2], Segment::Text(t) if &**t == "ん"));
        assert!(matches!(&segs[3], Segment::Directive(_)));
    }

    #[test]
    fn implicit_ruby_reading_with_embedded_gaiji_also_produces_segments() {
        // Implicit form must use the same body walker; only the base
        // extraction differs (trailing-kanji run instead of explicit
        // `｜`-delimited Text event).
        run!(out, "日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("日本"));
        assert!(!r.delim_explicit);
        let Content::Segments(segs) = r.reading.get() else {
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
        run!(out, src);
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
        // non-Directive emit path in `build_content_from_body` must
        // downgrade such shapes into `Directive{Unknown}` so the
        // bare `［＃` never reaches the rendered HTML through a
        // `Segment::Text` channel (Tier A canary).
        run!(out, "｜日本《にほん［＃改ページ］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        // Last segment must be an Directive carrying the raw bytes.
        let last = segs.last().expect("non-empty segments");
        let Segment::Directive(a) = last else {
            panic!("final segment should be Directive, got {last:?}");
        };
        assert_eq!(a.raw.as_str(), "［＃改ページ］");
        assert_eq!(a.kind, DirectiveKind::Unknown);
    }

    /// 縦中横 paired range `［＃縦中横］ … ［＃縦中横終わり］` opens and closes a
    /// `ContainerKind::CombineUprightRange` (a corpus convention, tolerant extension);
    /// the forward-reference `「X」は縦中横` leaf is unaffected. A longer
    /// needle-prefix body declines to Unknown.
    #[test]
    fn tcy_range_recognised() {
        run!(out, "前あ［＃縦中横］１２［＃縦中横終わり］後");
        let opens = out
            .spans
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    SpanKind::BlockOpen(ContainerKind::CombineUprightRange)
                )
            })
            .count();
        let closes = out
            .spans
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    SpanKind::BlockClose(ContainerKind::CombineUprightRange)
                )
            })
            .count();
        assert_eq!(opens, 1, "one CombineUprightRange open");
        assert_eq!(closes, 1, "one CombineUprightRange close");
        // A needle-prefix-but-longer body must not be claimed.
        run!(out, "x［＃縦中横ほげ］y");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Directive(a)) if a.kind == DirectiveKind::Unknown)),
            "縦中横ほげ should fall through to Unknown"
        );
    }

    #[test]
    fn page_break_annotation_becomes_single_page_break_span() {
        let src = "前\n［＃改ページ］\n後";
        run!(out, src);
        // Plain("前"), Newline, Aozora(PageBreak), Newline, Plain("後")
        assert_eq!(out.spans.len(), 5);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert!(matches!(aozora_node(&out.spans[2]), Some(Node::PageBreak)));
        assert_eq!(out.spans[2].source_span.slice(src), "［＃改ページ］");
        assert_eq!(out.spans[3].kind, SpanKind::Newline);
        assert_eq!(out.spans[4].kind, SpanKind::Plain);
    }

    /// 改頁 (the kanji spelling of 改ページ) and 地より (the alternate wording
    /// of 地から) are corpus spellings of supported layout directives — they
    /// emit the same nodes and canonicalise to 改ページ / 地から on serialize.
    #[test]
    fn alt_spelling_page_break_and_align_end() {
        run!(out, "前［＃改頁］後");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::PageBreak))),
            "改頁 should emit a PageBreak"
        );
        run!(out, "本文［＃地より２字上げ］続き");
        let offset = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::AlignEnd(a)) => Some(a.offset),
                _ => None,
            })
            .unwrap_or_else(|| panic!("地より should emit an AlignEnd"));
        assert_eq!(offset, 2);
    }

    #[test]
    fn section_break_kaicho_recognized() {
        run!(out, "［＃改丁］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::SectionBreak(SectionKind::Kaicho))
        ));
    }

    #[test]
    fn section_break_dan_recognized() {
        run!(out, "［＃改段］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::SectionBreak(SectionKind::Kaidan))
        ));
    }

    #[test]
    fn section_break_spread_recognized() {
        run!(out, "［＃改見開き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::SectionBreak(SectionKind::Kaimihiraki))
        ));
    }

    #[test]
    fn bracket_without_hash_is_not_an_annotation() {
        // `［普通］` (no `＃`) is plain literal text, not an annotation.
        run!(out, "［普通］");
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
        // recogniser matches, the `Directive{Unknown}` fallback wraps
        // the raw source so the renderer can emit an `aozora-directive`
        // hidden span instead of leaking the brackets as plain text.
        run!(out, "［＃未知のキーワード］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Directive(a)) => Some(a),
                _ => None,
            })
            .expect("unknown keyword must promote to Directive{Unknown}");
        assert_eq!(ann.kind, DirectiveKind::Unknown);
        assert_eq!(ann.raw.as_str(), "［＃未知のキーワード］");
    }

    #[test]
    fn annotation_with_whitespace_padding_still_matches() {
        // Corpus occasionally has `［＃ 改ページ ］` with spaces. We
        // trim the body to be lenient.
        run!(out, "［＃ 改ページ ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(aozora_node(&out.spans[0]), Some(Node::PageBreak)));
    }

    #[test]
    fn empty_bracket_with_hash_is_typed_as_empty() {
        // Real Aozora corpora use `［＃］` as the de-facto-standard symbol in
        // the file-header 凡例 (e.g. "［＃］：入力者注…"). It is typed as
        // DirectiveKind::Empty — recognised, not the Unknown catch-all — while
        // the Tier-A canary still holds (raw `［＃］` bytes preserved for
        // round-trip, no bare `［＃` leaking into HTML).
        run!(out, "［＃］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Directive(a)) => Some(a),
                _ => None,
            })
            .expect("empty body must wrap as an Directive");
        assert_eq!(ann.kind, DirectiveKind::Empty);
        assert_eq!(ann.raw.as_str(), "［＃］");
    }

    #[test]
    fn indent_with_full_width_digit() {
        run!(out, "［＃２字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::Indent(Indent { amount: 2 }))
        ));
    }

    #[test]
    fn indent_with_ascii_digit() {
        run!(out, "［＃10字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::Indent(Indent { amount: 10 }))
        ));
    }

    #[test]
    fn indent_overflow_falls_back_to_annotation_unknown() {
        // 300 > 255, doesn't fit in u8 — the `N字下げ` recogniser
        // declines. The `Directive { Unknown }` catch-all then
        // claims the bracket so the renderer wraps the body in an
        // aozora-directive span instead of leaking raw brackets.
        run!(out, "［＃300字下げ］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Directive(a)) => Some(a),
                _ => None,
            })
            .expect("overflow should fall back to Directive{Unknown}");
        assert_eq!(ann.kind, DirectiveKind::Unknown);
        assert_eq!(ann.raw.as_str(), "［＃300字下げ］");
        // The specialised Indent recogniser MUST NOT claim it.
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_digit_falls_through() {
        // N=0 is meaningless for 字下げ (a zero-width indent is not
        // a thing). Fullwidth-digit variant.
        run!(out, "［＃０字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_ascii_digit_falls_through() {
        // ASCII-digit variant of the N=0 reject.
        run!(out, "［＃0字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Indent(_)))),
        );
    }

    #[test]
    fn align_end_zero_digit_falls_through() {
        // 地から0字上げ is redundant with 地付き and not spec-sanctioned —
        // reject so the text falls through to a generic Directive.
        run!(out, "［＃地から0字上げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::AlignEnd(_)))),
        );
    }

    #[test]
    fn chitsuki_zero_offset_recognized() {
        run!(out, "［＃地付き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::AlignEnd(AlignEnd { offset: 0 }))
        ));
    }

    #[test]
    fn chi_kara_n_ji_age_recognized() {
        run!(out, "［＃地から３字上げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(Node::AlignEnd(AlignEnd { offset: 3 }))
        ));
    }

    #[test]
    fn indent_without_digits_falls_through() {
        // "ここから字下げ" is a paired-container opener, not a leaf
        // indent — the leaf classifier must not grab it, and the
        // paired-container recogniser claims it instead.
        run!(out, "［＃ここから字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Indent(_)))),
        );
    }

    #[test]
    fn forward_bouten_goma_recognized() {
        // Preceding text "前置き" plus "青空" before the bracket — the
        // target literal must appear in the preceding source for the
        // forward-reference classifier to promote.
        run!(out, "前置きの青空［＃「青空」に傍点］後ろ");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.kind, BoutenKind::Goma);
        assert_eq!(bouten.target.as_plain(), Some("青空"));
    }

    #[test]
    fn forward_emphasis_script_and_kogaki_recognized() {
        // 上付き/下付き小文字 (super/subscript) and 行右/行左小書き — the
        // four emphasis-page forward-reference families beyond 太字/斜体
        // (per <https://www.aozora.gr.jp/annotation/etc.html>). Each is a
        // first-class `Emphasis` leaf, NOT an `Directive{Unknown}`.
        for (src, want) in [
            ("x２［＃「２」は上付き小文字］", EmphasisKind::SuperScript),
            ("H２［＃「２」は下付き小文字］", EmphasisKind::SubScript),
            ("あ［＃「あ」は行右小書き］", EmphasisKind::SmallRight),
            ("い［＃「い」は行左小書き］", EmphasisKind::SmallLeft),
            ("注意［＃「注意」は罫囲み］", EmphasisKind::KeigakomiInline),
            ("西暦［＃「西暦」は横組み］", EmphasisKind::HorizontalInline),
        ] {
            run!(out, src);
            let emphasis = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Emphasis(e)) => Some(e),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected an Emphasis span for {src:?}"));
            assert_eq!(emphasis.kind, want, "src = {src:?}");
        }
    }

    #[test]
    fn forward_emphasis_font_size_recognized() {
        // 文字サイズ変更 `N段階大きな/小さな文字` — 大きな is a positive
        // stage count, 小さな negative; full-width and ASCII digits both
        // parse (per <https://www.aozora.gr.jp/annotation/etc.html>).
        for (src, want) in [
            (
                "甲［＃「甲」は2段階大きな文字］",
                EmphasisKind::FontSize { steps: 2 },
            ),
            (
                "乙［＃「乙」は1段階小さな文字］",
                EmphasisKind::FontSize { steps: -1 },
            ),
            (
                "丙［＃「丙」は３段階大きな文字］",
                EmphasisKind::FontSize { steps: 3 },
            ),
        ] {
            run!(out, src);
            let emphasis = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Emphasis(e)) => Some(e),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected an Emphasis span for {src:?}"));
            assert_eq!(emphasis.kind, want, "src = {src:?}");
        }
    }

    /// Bare-range font-size (`［＃{N}段階大きな/小さな文字］ … ［＃…文字終わり］`)
    /// and bare-range 横組み (`［＃横組み］ … ［＃横組み終わり］`) — the
    /// ここから/ここで-less siblings of the block forms. Corpus-attested in
    /// bulk (e.g. 共産党宣言) but previously fell through to
    /// `Directive{Unknown}`, silently dropping the styling. They reuse the
    /// existing `FontSize` / `Horizontal` containers; the close marker carries
    /// a ±1 placeholder magnitude (the open side is authoritative on pairing).
    #[test]
    fn bare_range_font_size_and_horizontal_recognised() {
        let cases: &[(&str, ContainerKind, ContainerKind)] = &[
            (
                "あ［＃１段階小さな文字］x［＃小さな文字終わり］い",
                ContainerKind::FontSize { steps: -1 },
                ContainerKind::FontSize { steps: -1 },
            ),
            (
                "あ［＃２段階大きな文字］x［＃大きな文字終わり］い",
                ContainerKind::FontSize { steps: 2 },
                ContainerKind::FontSize { steps: 1 },
            ),
            (
                "あ［＃横組み］x［＃横組み終わり］い",
                ContainerKind::Horizontal,
                ContainerKind::Horizontal,
            ),
        ];
        for (src, want_open, want_close) in cases {
            run!(out, src);
            let opens: Vec<ContainerKind> = out
                .spans
                .iter()
                .filter_map(|s| match s.kind {
                    SpanKind::BlockOpen(k) => Some(k),
                    _ => None,
                })
                .collect();
            let closes: Vec<ContainerKind> = out
                .spans
                .iter()
                .filter_map(|s| match s.kind {
                    SpanKind::BlockClose(k) => Some(k),
                    _ => None,
                })
                .collect();
            assert!(
                opens.contains(want_open),
                "expected open {want_open:?} for {src:?}, got {opens:?}"
            );
            assert!(
                closes.contains(want_close),
                "expected close {want_close:?} for {src:?}, got {closes:?}"
            );
            let unknown = out.spans.iter().any(|s| {
                matches!(
                    aozora_node(s),
                    Some(Node::Directive(a)) if a.kind == DirectiveKind::Unknown
                )
            });
            assert!(!unknown, "unexpected Unknown fall-through for {src:?}");
        }
    }

    /// ゴシック体 / ゴチック are corpus spellings of 太字 (bold): the official
    /// guide writes 太字（ゴシック）. Both map to `EmphasisKind::Bold` in the
    /// forward-reference suffix and in every range / block body, and
    /// canonicalise to 太字 on serialize (`Bold.keyword()`).
    #[test]
    fn gothic_spellings_map_to_bold() {
        assert_eq!(
            emphasis_kind_from_suffix("ゴシック体"),
            Some(EmphasisKind::Bold)
        );
        assert_eq!(
            emphasis_kind_from_suffix("ゴチック"),
            Some(EmphasisKind::Bold)
        );
        assert_eq!(
            parse_emphasis_body("ゴシック体"),
            Some((EmphasisKind::Bold, false, false))
        );
        assert_eq!(
            parse_emphasis_body("ゴチック終わり"),
            Some((EmphasisKind::Bold, false, true))
        );
        assert_eq!(
            parse_emphasis_body("ここからゴシック体"),
            Some((EmphasisKind::Bold, true, false))
        );
        assert_eq!(
            parse_emphasis_body("ここでゴチック終わり"),
            Some((EmphasisKind::Bold, true, true))
        );
        // Every spelling serializes back to the canonical 太字.
        assert_eq!(EmphasisKind::Bold.keyword(), "太字");
    }

    /// The bare 横組み needle must NOT claim a compound like `横組みで、…`
    /// (the exact-match guard rejects it) — it degrades to
    /// `Directive{Unknown}` rather than wrongly opening a Horizontal range.
    #[test]
    fn bare_horizontal_compound_stays_unknown() {
        run!(out, "あ［＃横組みで、ページの左右中央に］い");
        let unknown = out.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(Node::Directive(a)) if a.kind == DirectiveKind::Unknown
            )
        });
        let opened_horizontal = out
            .spans
            .iter()
            .any(|s| matches!(s.kind, SpanKind::BlockOpen(ContainerKind::Horizontal)));
        assert!(
            unknown && !opened_horizontal,
            "expected Directive{{Unknown}} and no Horizontal open, got {:?}",
            out.spans
        );
    }

    /// 小書き range `［＃行右小書き］ … ［＃行右小書き終わり］` (and 行左) —
    /// the bare-range sibling of the forward `「X」は行右小書き` emphasis —
    /// opens an inline `SmallScript` container carrying the 右/左 side.
    #[test]
    fn small_script_range_recognised() {
        let cases: &[(&str, ContainerKind)] = &[
            (
                "x［＃行右小書き］２）［＃行右小書き終わり］y",
                ContainerKind::SmallScript {
                    side: BoutenPosition::Right,
                },
            ),
            (
                "x［＃行左小書き］左［＃行左小書き終わり］y",
                ContainerKind::SmallScript {
                    side: BoutenPosition::Left,
                },
            ),
        ];
        for (src, want) in cases {
            run!(out, src);
            let opens: Vec<ContainerKind> = out
                .spans
                .iter()
                .filter_map(|s| match s.kind {
                    SpanKind::BlockOpen(k) => Some(k),
                    _ => None,
                })
                .collect();
            let closes: Vec<ContainerKind> = out
                .spans
                .iter()
                .filter_map(|s| match s.kind {
                    SpanKind::BlockClose(k) => Some(k),
                    _ => None,
                })
                .collect();
            assert!(opens.contains(want), "open for {src:?}: {opens:?}");
            assert!(closes.contains(want), "close for {src:?}: {closes:?}");
        }
        // A needle-prefix-but-longer body must not be claimed.
        run!(out, "x［＃行右小書きほげ］y");
        let opened = out.spans.iter().any(|s| {
            matches!(
                s.kind,
                SpanKind::BlockOpen(ContainerKind::SmallScript { .. })
            )
        });
        assert!(
            !opened,
            "行右小書きほげ must not open a SmallScript container"
        );
    }

    /// Input-editor notes type correctly instead of degrading to Unknown:
    /// `「X」はママ` / bare `ママ` → `Sic`; `…底本では…` → `BaseTextVariant`.
    /// These were previously dead `DirectiveKind` variants (emitted nowhere
    /// on the whole corpus). The target text stays in place.
    #[test]
    fn editorial_notes_type_as_asis_and_textual_note() {
        use aozora_syntax::borrowed::Node;
        let cases: &[(&str, DirectiveKind)] = &[
            ("誤［＃「誤」はママ］", DirectiveKind::Sic),
            ("あ［＃ママ］", DirectiveKind::Sic),
            // 底本のまま — kept-irregularity note, the same *sic* family as ママ.
            ("綴り［＃底本のまま］", DirectiveKind::Sic),
            (
                "名刺［＃「名刺」は底本では「名剌」］",
                DirectiveKind::BaseTextVariant,
            ),
            (
                "。［＃「。」は底本では脱落］",
                DirectiveKind::BaseTextVariant,
            ),
            // 初出では — the first-appearance divergence note, same shape as 底本では.
            (
                "正字［＃「正字」は初出では「異字」］",
                DirectiveKind::BaseTextVariant,
            ),
        ];
        for (src, want) in cases {
            run!(out, src);
            let kind = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Directive(a)) => Some(a.kind),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected an Directive for {src:?}"));
            assert_eq!(kind, *want, "src = {src:?}");
        }
    }

    /// The empty directive `［＃］` (and whitespace-only `［＃　］`) — the
    /// file-header 凡例 symbol that prefixes nearly every work — types as
    /// `Empty`, not the `Unknown` catch-all, while still round-tripping.
    #[test]
    fn empty_directive_types_as_empty() {
        use aozora_syntax::borrowed::Node;
        // ［＃］ (入力者注), whitespace-only ［＃　］, ［＃…］ (返り点 legend
        // symbol), ［＃（…）］ (訓点送り仮名 legend symbol).
        for src in [
            "序文［＃］：入力者注",
            "本文［＃　］続き",
            "x［＃…］：返り点",
            "y［＃（…）］：訓点送り仮名",
        ] {
            run!(out, src);
            let kind = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Directive(a)) => Some(a.kind),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected an Directive for {src:?}"));
            assert_eq!(kind, DirectiveKind::Empty, "src = {src:?}");
        }
    }

    /// The remaining corpus-attested layout forms: block 罫囲み / 割り注
    /// (`ここから…`), 天から{N}字下げ (Indent leaf), the bare top-flush hanging
    /// indent, range bouten `「X」～「Y」に<kind>`, and `×傍点` (→ Cross).
    #[test]
    fn remaining_layout_and_range_forms_recognised() {
        use aozora_syntax::borrowed::Node;
        let opens = |src: &str| -> Vec<ContainerKind> {
            run!(out, src);
            out.spans
                .iter()
                .filter_map(|s| match s.kind {
                    SpanKind::BlockOpen(k) => Some(k),
                    _ => None,
                })
                .collect()
        };
        assert!(opens("［＃ここから罫囲み］").contains(&ContainerKind::Framed));
        assert!(opens("［＃ここから割り注］").contains(&ContainerKind::Warichu));
        assert!(
            opens("［＃改行天付き、折り返して２字下げ］").contains(&ContainerKind::Indent {
                amount: 0,
                wrap: Some(2),
                center: false,
            })
        );
        // 天から{N}字下げ → Indent leaf
        run!(t, "［＃天から３字下げ］本文");
        assert!(t.spans.iter().any(|s| matches!(
            aozora_node(s),
            Some(Node::Indent(i)) if i.amount == 3
        )));
        // range bouten and ×傍点 → Bouten leaf
        run!(
            r,
            "あ實は中身呉れるのである［＃「實は」～「呉れるのである」に傍点］"
        );
        assert!(
            r.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_))))
        );
        run!(x, "天皇制［＃「天皇制」に×傍点］");
        assert!(x.spans.iter().any(|s| matches!(
            aozora_node(s),
            Some(Node::Bouten(b)) if b.kind == BoutenKind::Cross
        )));
    }

    /// Caption forms: the bare range `［＃キャプション］…終わり` and block
    /// `ここからキャプション…終わり` open a `Caption` container (inline / block);
    /// the forward `「X」はキャプション` is an `Emphasis{Caption}` leaf.
    #[test]
    fn caption_range_block_and_forward_recognised() {
        use aozora_syntax::borrowed::Node;
        // bare range → inline Caption container
        run!(a, "図［＃キャプション］第一図［＃キャプション終わり］");
        assert!(a.spans.iter().any(|s| matches!(
            s.kind,
            SpanKind::BlockOpen(ContainerKind::Caption { block: false })
        )));
        // block → block Caption container
        run!(
            b,
            "［＃ここからキャプション］本文［＃ここでキャプション終わり］"
        );
        assert!(b.spans.iter().any(|s| matches!(
            s.kind,
            SpanKind::BlockOpen(ContainerKind::Caption { block: true })
        )));
        // forward leaf
        run!(c, "第一図［＃「第一図」はキャプション］");
        assert!(c.spans.iter().any(|s| matches!(
            aozora_node(s),
            Some(Node::Emphasis(e)) if e.kind == EmphasisKind::Caption
        )));
    }

    /// The general image form `<説明>（file［、横W×縦H］）入る` (graphics.html):
    /// any leading description becomes the alt, dimensions split off the
    /// file, and the leading text round-trips verbatim. The `のキャプション
    /// 付き` form is left to `classify_caption_figure` (it earns a figcaption).
    #[test]
    fn general_image_form_recognised() {
        use aozora_syntax::borrowed::Node;
        let cases = [
            (
                "［＃図（fig1.png、横100×縦80）入る］",
                "図",
                "fig1.png",
                Some("横100×縦80"),
            ),
            ("［＃口絵（fig2.png）入る］", "口絵", "fig2.png", None),
            (
                "［＃神代文字ア（f.png、横20×縦20）入る］",
                "神代文字ア",
                "f.png",
                Some("横20×縦20"),
            ),
        ];
        for (src, desc, file, dims) in cases {
            run!(out, src);
            let s = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Illustration(s)) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected a Illustration for {src:?}"));
            assert_eq!(s.description, Some(desc), "desc {src:?}");
            assert_eq!(s.file.as_str(), file, "file {src:?}");
            assert_eq!(s.dimensions, dims, "dims {src:?}");
            assert!(
                s.number.is_none() && s.caption.is_none(),
                "general form has no number / trailing caption: {src:?}"
            );
        }
        // The のキャプション付き form stays with classify_caption_figure, which
        // lifts the 「caption」 into a figcaption rather than the alt.
        run!(out, "［＃「絵」のキャプション付きの図（f.png）入る］");
        let cap = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Illustration(s)) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| panic!("caption-figure should still be a Illustration"));
        assert!(
            cap.caption.is_some() && cap.description.is_none(),
            "caption-figure routes its 「caption」 to a figcaption, not the alt"
        );
    }

    /// `「caption」のキャプション付きの(図|挿絵)（file）入る` is a Illustration whose
    /// caption precedes the figure.
    #[test]
    fn caption_before_figure_recognised() {
        use aozora_syntax::borrowed::Node;
        for src in [
            "［＃「第一図」のキャプション付きの図（fig01.png）入る］",
            "［＃「絵」のキャプション付きの挿絵（fig02.png）入る］",
        ] {
            run!(out, src);
            let sashie = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Illustration(s)) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected a Illustration for {src:?}"));
            assert!(sashie.caption.is_some(), "caption for {src:?}");
        }
    }

    /// The compound `「X」は縦中横、行右/左小書き` (numbered list markers) is
    /// recognised as 縦中横 (the dominant transform), not Unknown.
    #[test]
    fn tcy_small_script_compound_recognised_as_tcy() {
        use aozora_syntax::borrowed::Node;
        run!(out, "１）［＃「１）」は縦中横、行右小書き］");
        let is_tcy = out
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(Node::CombineUpright(_))));
        let unknown = out.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(Node::Directive(a)) if a.kind == DirectiveKind::Unknown
            )
        });
        assert!(
            is_tcy && !unknown,
            "expected CombineUpright, got {:?}",
            out.spans
        );
    }

    /// The bare `「X」に「Y」の注記` side-annotation (the corpus's dominant
    /// shape) is recognised as a `MarginNote`, like the explicit
    /// `「X」の左に「Y」の注記` left form. `MarginNote` has no side axis, so both
    /// map to the same node.
    #[test]
    fn side_note_right_form_recognised() {
        use aozora_syntax::borrowed::Node;
        for src in [
            "は［＃「は」に「ママ」の注記］",
            "は［＃「は」の左に「ママ」の注記］",
        ] {
            run!(out, src);
            let is_side_note = out
                .spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::MarginNote(_))));
            assert!(
                is_side_note,
                "expected a MarginNote for {src:?}: {:?}",
                out.spans
            );
        }
    }

    /// `「X」に「Y」の傍記` (issue #125, the censorship-marker form) is
    /// recognised as a `MarginNote` tagged [`MarginNoteKind::Marginal`] —
    /// distinct from the 注記 flavour so it round-trips to `の傍記`.
    #[test]
    fn boki_form_recognised() {
        use aozora_syntax::MarginNoteKind;
        use aozora_syntax::borrowed::Node;
        run!(out, "資本主義の一般的危機［＃「危機」に「×」の傍記］");
        let marginal = out.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(Node::MarginNote(sn)) if sn.kind == MarginNoteKind::Marginal
            )
        });
        assert!(
            marginal,
            "expected a Marginal MarginNote, got {:?}",
            out.spans
        );
    }

    /// `［＃ここから改行天付き、折り返して{M}字下げ］` — the corpus's most
    /// common compound indent — opens an Indent container with the first
    /// line flush to the top (amount 0) and wrapped lines indented M.
    #[test]
    fn kaigyou_tentsuki_wrap_indent_recognised() {
        run!(out, "［＃ここから改行天付き、折り返して２字下げ］");
        let open = out.spans.iter().find_map(|s| match s.kind {
            SpanKind::BlockOpen(k) => Some(k),
            _ => None,
        });
        assert_eq!(
            open,
            Some(ContainerKind::Indent {
                amount: 0,
                wrap: Some(2),
                center: false,
            }),
            "spans = {:?}",
            out.spans
        );
    }

    /// A target-bearing `「ママ」に傍点` must still be claimed as a Bouten by
    /// the earlier recogniser — the editorial-note tail never sees it.
    #[test]
    fn mama_target_with_bouten_stays_bouten() {
        use aozora_syntax::borrowed::Node;
        run!(out, "ママ［＃「ママ」に傍点］");
        let is_bouten = out
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_))));
        assert!(is_bouten, "expected a Bouten, got {:?}", out.spans);
    }

    #[test]
    fn forward_emphasis_font_size_zero_and_overflow_decline() {
        // 0段階 is degenerate and >127 overflows i8 — both decline cleanly
        // to Directive{Unknown} (no Emphasis node).
        for src in [
            "甲［＃「甲」は0段階大きな文字］",
            "甲［＃「甲」は200段階大きな文字］",
        ] {
            run!(out, src);
            assert!(
                !out.spans
                    .iter()
                    .any(|s| matches!(aozora_node(s), Some(Node::Emphasis(_)))),
                "src = {src:?} should not yield an Emphasis node",
            );
        }
    }

    #[test]
    fn forward_bouten_consumes_immediate_predecessor_literal() {
        // The fix for forward-ref text duplication: when the target
        // literal sits *immediately* before the `［`, the classifier
        // hands back a `consume_start` that pulls the bouten span back
        // to swallow it, so the preceding plain run flushes only up to
        // the literal's start. Without this, the renderer would emit
        // both the raw literal and the bouten's own content, producing
        // `<text>青空<em>...青空</em>` — the playground welcome-page bug.
        //
        // Concretely we expect two spans:
        //   [0] Plain("前置きの")     — the prefix up to (but not
        //                               including) the consumed literal
        //   [1] Aozora(Bouten { … })  — covers "青空［＃「青空」に傍点］"
        //   [2] Plain("後ろ")         — the trailing prefix
        run!(out, "前置きの青空［＃「青空」に傍点］後ろ");
        let classified: Vec<_> = out
            .spans
            .iter()
            .map(|s| {
                let kind_str = match s.kind {
                    SpanKind::Plain => "Plain",
                    SpanKind::Aozora(Node::Bouten(_)) => "Bouten",
                    _ => "Other",
                };
                let slice = &"前置きの青空［＃「青空」に傍点］後ろ"
                    [s.source_span.start as usize..s.source_span.end as usize];
                (kind_str, slice)
            })
            .collect();
        // The legacy bug-state would have produced ["Plain('前置きの青空')",
        // "Bouten('［＃「青空」に傍点］')", "Plain('後ろ')"]. The fix
        // shrinks the Plain prefix and grows the Bouten span backwards.
        assert_eq!(
            classified.as_slice(),
            &[
                ("Plain", "前置きの"),
                ("Bouten", "青空［＃「青空」に傍点］"),
                ("Plain", "後ろ"),
            ],
            "spans drift from the consume_start=literal_start contract",
        );
        // The classifier must also flip the per-node consumed flag so the
        // serializer round-trips the literal back into place.
        let bouten_flag = out.spans.iter().find_map(|s| match s.kind {
            SpanKind::Aozora(Node::Bouten(b)) => Some(b.consumed_predecessor),
            _ => None,
        });
        assert_eq!(
            bouten_flag,
            Some(true),
            "consume_start shrunk → consumed_predecessor must be true",
        );
    }

    #[test]
    fn forward_bouten_with_intervening_text_keeps_legacy_consume() {
        // Edge case: target appears earlier in the paragraph but NOT
        // immediately before the bracket. We deliberately leave the
        // legacy duplicating behaviour in that case rather than
        // splice a hole into the middle of the pending plain run
        // (the `flush_plain_up_to` API is truncate-only). The Bouten
        // is still emitted with its own target content.
        run!(out, "青空の下を歩く［＃「青空」に傍点］");
        let mut saw_bouten = false;
        let mut saw_plain_with_aozora = false;
        for s in &out.spans {
            match &s.kind {
                SpanKind::Aozora(Node::Bouten(_)) => saw_bouten = true,
                SpanKind::Plain => {
                    let slice = &"青空の下を歩く［＃「青空」に傍点］"
                        [s.source_span.start as usize..s.source_span.end as usize];
                    if slice.contains("青空") {
                        saw_plain_with_aozora = true;
                    }
                }
                _ => {}
            }
        }
        assert!(
            saw_bouten,
            "Bouten must still promote with non-adjacent target"
        );
        assert!(
            saw_plain_with_aozora,
            "non-adjacent target stays in preceding Plain (legacy behaviour preserved)"
        );
    }

    #[test]
    fn forward_bouten_circle_recognized() {
        run!(out, "X［＃「X」に丸傍点］");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
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
        // to `Directive{Unknown}`, lowering the sweep leak rate.
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
            run!(out, &src);
            let Some(b) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
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
        run!(out, "X［＃「X」の左に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
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
            run!(out, &src);
            let Some(b) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
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
        run!(out, "AとB［＃「A」「B」に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
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
        // `Directive{Unknown}` by the catch-all instead, preserving
        // Tier-A without inventing a bouten target.
        run!(out, "A［＃「A」「B」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_)))),
            "Bouten must not promote when any target is unreferenced"
        );
    }

    #[test]
    fn forward_bouten_empty_inner_quotes_are_skipped() {
        // `「」` placeholders in the middle of a multi-quote body do
        // not contribute to the target list. This guards against
        // corpus stragglers like `［＃「A」「」「B」に傍点］`.
        run!(out, "AB［＃「A」「」「B」に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_position_slug_and_segments_render_together() {
        // Regression: the position modifier must be propagated even
        // when the target is a Segments (multi-quote) value.
        run!(out, "AB［＃「A」「B」の左に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.position, BoutenPosition::Left);
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_empty_target_falls_through() {
        run!(out, "［＃「」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_unknown_suffix_falls_through() {
        run!(out, "［＃「X」に未知］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_missing_ni_particle_falls_through() {
        run!(out, "［＃「X」傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_without_preceding_target_falls_through() {
        // Target 可哀想 never appears before the bracket — refusing to
        // promote to Bouten lets the generic Directive classifier
        // wrap the raw `［＃…］` in an aozora-directive span instead of
        // styling a non-existent referent.
        run!(out, "［＃「可哀想」に傍点］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_target_in_preceding_paragraph_still_promotes() {
        // The classifier currently scans the entire preceding source
        // (not just the current paragraph). Preserving that lenient
        // behaviour keeps real Aozora corpora working — authors
        // sometimes refer backwards across paragraph boundaries.
        run!(out, "青空\n\n改行後［＃「青空」に傍点］");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_)))),
        );
    }

    #[test]
    fn forward_tcy_without_preceding_target_falls_through() {
        run!(out, "［＃「29」は縦中横］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::CombineUpright(_)))),
        );
    }

    #[test]
    fn forward_bouten_with_nested_quote_in_target_uses_outer_quote() {
        // Phase 2 balances 「「」」 correctly. The target is the full
        // outer-quote contents including the inner 「inner」 — not
        // truncated at the first 」. The preceding copy of the target
        // is required so the classifier's target-exists check passes.
        run!(out, "A「inner」B［＃「A「inner」B」に傍点］");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.target.as_plain(), Some("A「inner」B"));
    }

    #[test]
    fn forward_tcy_single_recognized() {
        run!(out, "20［＃「20」は縦中横］");
        let tcy = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::CombineUpright(t)) => Some(t),
                _ => None,
            })
            .expect("expected a CombineUpright span");
        assert_eq!(tcy.text.as_plain(), Some("20"));
    }

    #[test]
    fn forward_tcy_wrong_particle_falls_through() {
        // Using に instead of は — not a TCY shape.
        run!(out, "［＃「20」に縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::CombineUpright(_)))),
        );
    }

    #[test]
    fn forward_tcy_empty_target_falls_through() {
        run!(out, "［＃「」は縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::CombineUpright(_)))),
        );
    }

    // ---------------------------------------------------------------
    // Forward-reference heading hints — `［＃「X」は(大|中|小)見出し］`.
    // These tests pin the lexer contract that drives post-process
    // paragraph promotion (docs/plan.md §M2): the classifier emits a
    // `HeadingHint { level: 1..=3 }` when the target is preceded by a
    // matching run in the source, otherwise falls through so the
    // catch-all emits `Directive { Unknown }` and the Tier-A canary
    // ([# never leaks) still holds.
    // ---------------------------------------------------------------

    fn find_heading_hint<'a>(out: &TestClassifyOutput<'a>) -> Option<&'a HeadingHint<'a>> {
        out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(Node::HeadingHint(h)) => Some(h),
            _ => None,
        })
    }

    #[test]
    fn forward_heading_large_recognized() {
        // Spec: 大見出し → Markdown H1 (level 1). The preceding
        // occurrence of the target literal is required — same gate as
        // forward-bouten.
        run!(out, "第一篇［＃「第一篇」は大見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 1);
        assert_eq!(h.target.as_str(), "第一篇");
    }

    #[test]
    fn forward_heading_medium_recognized() {
        // 中見出し → H2.
        run!(out, "一［＃「一」は中見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 2);
        assert_eq!(h.target.as_str(), "一");
    }

    #[test]
    fn forward_heading_small_recognized() {
        // 小見出し → H3.
        run!(out, "小題［＃「小題」は小見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 3);
        assert_eq!(h.target.as_str(), "小題");
    }

    #[test]
    fn forward_heading_without_preceding_target_falls_through() {
        // No 「第一篇」 run in the preceding source — hint has no
        // referent; classifier must reject so the paragraph isn't
        // promoted to an empty heading. The catch-all then emits
        // `Directive { Unknown }` to preserve Tier-A.
        run!(out, "［＃「第一篇」は大見出し］後");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_unknown_keyword_falls_through() {
        // `大見出し` and friends are the only supported heading
        // keywords; anything else (包括的, 飾り見出し, …) should not
        // promote.
        run!(out, "X［＃「X」は飾り見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_wrong_particle_falls_through() {
        // The Aozora annotation spec's heading shape uses `は` as the
        // particle. Using `に` (the bouten particle) must not promote
        // to HeadingHint — otherwise we'd clobber the bouten path.
        run!(out, "X［＃「X」に大見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_empty_target_falls_through() {
        run!(out, "［＃「」は大見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_ruby_split_target_recognized() {
        // The heading text carries ruby (`両頭《りやうとう》`), so the quoted
        // target is the ruby-*stripped* form `○　両頭の蛇` and is not a
        // contiguous source substring. The ruby-tolerant gate strips `《…》`
        // from the look-back and recovers it as a hint (the corpus's
        // single largest Unknown family).
        run!(
            out,
            "○　両頭《りやうとう》の蛇《へび》［＃「○　両頭の蛇」は中見出し］"
        );
        let hit = find_heading_hint(&out)
            .is_some_and(|h| h.level == 2 && h.target.as_str() == "○　両頭の蛇");
        assert!(hit, "expected a 中見出し HeadingHint, got {:?}", out.spans);
    }

    #[test]
    fn forward_heading_explicit_bar_ruby_target_recognized() {
        // Explicit-base ruby `序｜章《しよう》`: the `｜` marker is also
        // stripped so the target `序章` matches the look-back.
        run!(out, "序｜章《しよう》［＃「序章」は大見出し］");
        let hit =
            find_heading_hint(&out).is_some_and(|h| h.level == 1 && h.target.as_str() == "序章");
        assert!(hit, "expected a 大見出し HeadingHint, got {:?}", out.spans);
    }

    #[test]
    fn forward_heading_ruby_strip_does_not_invent_legend_example() {
        // The standard 凡例 line `（例）［＃「第一章」は中見出し］` has no
        // preceding `第一章` run — even ruby-stripped, the look-back is
        // `（例）`, which does not contain the target. It must stay Unknown
        // (a documentation example, not a real heading).
        run!(out, "（例）［＃「第一章」は中見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_all_three_levels_exercised_in_one_paragraph() {
        // A single paragraph could conceivably carry multiple heading
        // hints — the lexer emits one HeadingHint per bracket and
        // post-process handles the first. This test locks the per-
        // bracket classification rather than the post_process policy.
        run!(
            out,
            "A［＃「A」は大見出し］B［＃「B」は中見出し］C［＃「C」は小見出し］"
        );
        let levels: Vec<u8> = out
            .spans
            .iter()
            .filter_map(|s| match aozora_node(s) {
                Some(Node::HeadingHint(h)) => Some(h.level),
                _ => None,
            })
            .collect();
        assert_eq!(levels, vec![1, 2, 3]);
    }

    #[test]
    fn sashie_without_caption_recognized() {
        run!(out, "［＃挿絵（fig01.png）入る］");
        let sashie = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Illustration(s)) => Some(s),
                _ => None,
            })
            .expect("expected a Illustration span");
        assert_eq!(sashie.file.as_str(), "fig01.png");
        assert!(sashie.number.is_none());
        assert!(sashie.caption.is_none());
    }

    /// The numbered illustration form `［＃挿絵{N}（file）入る］` keeps the
    /// figure index verbatim (full-width digits included) and round-trips.
    #[test]
    fn sashie_numbered_form_recognized() {
        for (src, want_num, want_file, want_dims) in [
            (
                "［＃挿絵10（fig01.png、横362×縦489）入る］",
                "10",
                "fig01.png",
                Some("横362×縦489"),
            ),
            (
                "［＃挿絵１（fig194_01.png）入る］",
                "１",
                "fig194_01.png",
                None,
            ),
        ] {
            run!(out, src);
            let sashie = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Illustration(s)) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected a Illustration span for {src:?}"));
            let num = sashie
                .number
                .unwrap_or_else(|| panic!("figure number present for {src:?}"));
            assert_eq!(num.as_str(), want_num, "src={src:?}");
            assert_eq!(sashie.file.as_str(), want_file, "src={src:?}");
            assert_eq!(sashie.dimensions, want_dims, "src={src:?}");
        }
        // A free description before `（…）入る` is the general image form
        // (graphics.html): `女性の挿絵（fig.png）入る` → a Illustration whose leading
        // text is the alt (no 挿絵 keyword, no figure number).
        run!(out, "x［＃女性の挿絵（fig.png）入る］y");
        let general = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Illustration(s)) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| panic!("general image form should be recognised"));
        assert_eq!(general.description, Some("女性の挿絵"));
        assert_eq!(general.file.as_str(), "fig.png");
        assert!(general.number.is_none(), "general form carries no number");
    }

    #[test]
    fn sashie_with_caption_recognized() {
        // Bundled-caption form `挿絵（file）「caption」入る` — the caption is
        // captured as plain content (rendered into <figcaption>, §8).
        run!(out, "［＃挿絵（fig01.png）「キャプション」入る］");
        let found = out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(Node::Illustration(s)) => Some(s),
            _ => None,
        });
        let Some(sashie) = found else {
            panic!("expected a Illustration span");
        };
        assert_eq!(sashie.file.as_str(), "fig01.png");
        let Some(caption) = sashie.caption else {
            panic!("expected a bundled caption");
        };
        assert_eq!(caption.as_plain(), Some("キャプション"));
    }

    #[test]
    fn sashie_with_dimensions_splits_file_and_size() {
        // Bundled corpus form `挿絵（file、横W×縦H）入る` — the pixel-size
        // note rides in `dimensions`, keeping `file` a clean path.
        run!(out, "［＃挿絵（fig42_03.png、横480×縦640）入る］");
        let Some(sashie) = out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(Node::Illustration(s)) => Some(s),
            _ => None,
        }) else {
            panic!("expected a Illustration span");
        };
        assert_eq!(sashie.file.as_str(), "fig42_03.png");
        assert_eq!(sashie.dimensions, Some("横480×縦640"));
        assert!(sashie.caption.is_none());
    }

    #[test]
    fn sashie_empty_caption_falls_through() {
        // `「」入る` is a degenerate empty caption — decline cleanly.
        run!(out, "［＃挿絵（fig01.png）「」入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Illustration(_)))),
        );
    }

    #[test]
    fn sashie_empty_filename_falls_through() {
        run!(out, "［＃挿絵（）入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Illustration(_)))),
        );
    }

    #[test]
    fn sashie_missing_iru_suffix_falls_through() {
        run!(out, "［＃挿絵（fig01.png）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Illustration(_)))),
        );
    }

    #[test]
    fn gaiji_quoted_description_with_mencode() {
        use aozora_encoding::gaiji::Resolved;
        run!(out, "※［＃「木＋吶のつくり」、第3水準1-85-54］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(gaiji.description, "木＋吶のつくり");
        assert_eq!(gaiji.mencode, Some("第3水準1-85-54"));
        // JIS X 0213:2004 plane 1 row 85 cell 54 = 枘 (U+6798).
        // (Pre-regen seed had U+6903 (椃) — that was a different
        // character, U+6903 = 木+室. The corrected mapping is sourced
        // from glibc's EUC-JISX0213 charmap = the spec.)
        assert_eq!(gaiji.ucs, Some(Resolved::Char('\u{6798}')));
    }

    /// A composed-glyph gaiji with a trailing 底本ページ-行 suffix
    /// (`、U+74FC、372-10`): the full mencode is kept verbatim for round-trip,
    /// but the page-line is stripped for resolution so the codepoint still
    /// resolves. Previously the trailing suffix failed `is_mencode_shaped`
    /// and the whole bracket degraded to `Directive{Unknown}`.
    #[test]
    fn gaiji_composed_with_page_line_suffix() {
        use aozora_encoding::gaiji::Resolved;
        run!(
            out,
            "※［＃「瓰」の「扮のつくり」に代えて「里」、U+74FC、372-10］"
        );
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected a Gaiji span, not Unknown"));
        assert_eq!(gaiji.description, "「瓰」の「扮のつくり」に代えて「里」");
        // Full mencode (incl. page-line) kept verbatim for the round-trip.
        assert_eq!(gaiji.mencode, Some("U+74FC、372-10"));
        // The page-line is stripped for resolution → U+74FC still resolves.
        assert_eq!(gaiji.ucs, Some(Resolved::Char('\u{74FC}')));
    }

    #[test]
    fn gaiji_quoted_description_without_mencode() {
        run!(out, "※［＃「試」］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(gaiji.description, "試");
        assert!(gaiji.mencode.is_none());
    }

    #[test]
    fn gaiji_bare_description_with_mencode() {
        run!(out, "※［＃二の字点、1-2-23］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(gaiji.description, "二の字点");
        assert_eq!(gaiji.mencode, Some("1-2-23"));
    }

    #[test]
    fn gaiji_consumes_refmark_and_bracket_as_one_span() {
        let src = "a※［＃「X」、m］b";
        run!(out, src);
        let gaiji_span = out
            .spans
            .iter()
            .find(|s| matches!(aozora_node(s), Some(Node::Gaiji(_))))
            .expect("expected a Gaiji span");
        // span must start at the ※ (after "a"), not at ［.
        assert_eq!(gaiji_span.source_span.slice(src), "※［＃「X」、m］");
    }

    #[test]
    fn gaiji_composed_glyph_kaete_form() {
        // ※［＃「X」の「Y」に代えて「Z」、第N水準…］ — composed-glyph gaiji.
        // The whole pre-mencode body is the verbatim description; the
        // trailing mencode resolves the character. Previously this dropped
        // everything after the first quote (a round-trip data loss).
        run!(out, "※［＃「比」の「ヒ」に代えて「く」、第4水準2-1-23］");
        let Some(g) = out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(Node::Gaiji(g)) => Some(g),
            _ => None,
        }) else {
            panic!("expected Gaiji node for the 代えて composed-glyph form");
        };
        assert_eq!(g.description, "「比」の「ヒ」に代えて「く」");
        assert_eq!(g.mencode, Some("第4水準2-1-23"));
    }

    #[test]
    fn refmark_without_following_bracket_stays_plain() {
        // Bare ※ without ［＃...］ — not a gaiji, emit as Plain.
        run!(out, "a※b");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Gaiji(_)))),
        );
    }

    #[test]
    fn gaiji_without_hash_is_not_recognized() {
        // ※ followed by ［ but no ＃ inside — not a gaiji shape.
        run!(out, "※［普通］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Gaiji(_)))),
        );
    }

    /// Standalone (no-`※`) external-character note (#122): a `［＃…］` whose
    /// body is a gaiji description with a trailing mencode / 底本ページ-行 is
    /// recognised as a Gaiji, not an `Directive{Unknown}`.
    #[test]
    fn standalone_gaiji_is_form_with_mencode_and_page_line() {
        run!(out, "［＃「※」は「祿－示」、第3水準1-84-27、144-上-9］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected a standalone Gaiji span, not Unknown"));
        assert_eq!(gaiji.description, "「※」は「祿－示」");
        assert_eq!(gaiji.mencode, Some("第3水準1-84-27、144-上-9"));
        assert!(gaiji.standalone, "no `※` in source → standalone");
    }

    #[test]
    fn standalone_gaiji_composed_kaete_form() {
        run!(out, "［＃「比」の「ヒ」に代えて「く」、第4水準2-1-23］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected a standalone composed Gaiji span"));
        assert_eq!(gaiji.description, "「比」の「ヒ」に代えて「く」");
        assert!(gaiji.standalone);
    }

    /// The standalone form needs a mencode / page-line tail (or a resolved
    /// glyph): an ordinary quoted `［＃「…」］` note has no such tail and — with
    /// no disambiguating `※` — must NOT be wrongly claimed as a gaiji.
    #[test]
    fn standalone_gaiji_declines_plain_note() {
        run!(out, "［＃「これはただの注記」］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Gaiji(_)))),
            "plain quoted note must not become a gaiji: {:?}",
            out.spans
        );
    }

    /// A standalone gaiji whose tail is a 底本ページ-行 only (`、N-下-N`, no JIS
    /// men-ku-ten) is still recognised as a gaiji (#122) rather than degrading
    /// to `Directive{Unknown}`; the page-line is kept verbatim in `mencode`
    /// (resolution, if any, comes from the description).
    #[test]
    fn standalone_gaiji_page_line_only_tail() {
        run!(out, "あ［＃小書き片仮名ヲ、5-下-3］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Gaiji(g)) => Some(g),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected a page-line-only Gaiji span"));
        assert_eq!(gaiji.description, "小書き片仮名ヲ");
        assert_eq!(gaiji.mencode, Some("5-下-3"));
        assert!(gaiji.standalone);
    }

    #[test]
    fn kaeriten_ichi_recognized() {
        run!(out, "之［＃一］");
        let kaeriten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(Node::Kaeriten(k)) => Some(k),
                _ => None,
            })
            .expect("expected a Kaeriten span");
        assert_eq!(kaeriten.mark.as_str(), "一");
    }

    #[test]
    fn kaeriten_all_twelve_marks_recognized() {
        for mark in [
            "一", "二", "三", "四", "上", "中", "下", "レ", "甲", "乙", "丙", "丁",
        ] {
            let src = format!("［＃{mark}］");
            run!(out, &src);
            let Some(k) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(Node::Kaeriten(k)) => Some(k),
                _ => None,
            }) else {
                panic!("no Kaeriten span for mark {mark:?}");
            };
            assert_eq!(k.mark.as_str(), mark);
        }
    }

    #[test]
    fn kaeriten_unknown_mark_falls_through() {
        run!(out, "［＃甬］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Kaeriten(_)))),
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
            run!(out, &src);
            let k = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Kaeriten(k)) => Some(k),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no Kaeriten span for mark {mark:?}"));
            assert_eq!(k.mark.as_str(), mark, "mark={mark:?}");
        }
    }

    #[test]
    fn kaeriten_okurigana_shape_recognized() {
        // `［＃（X）］` where X is 1–6 Japanese chars is treated as an
        // okurigana marker — same Node::Kaeriten with the
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
            run!(out, &src);
            let k = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(Node::Kaeriten(k)) => Some(k),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no Kaeriten for okurigana {mark:?}"));
            assert_eq!(k.mark.as_str(), mark, "mark={mark:?}");
        }
    }

    #[test]
    fn kaeriten_okurigana_with_long_body_falls_through() {
        // 7+ character parenthesised content is almost always an
        // editorial gloss, not okurigana. Must fall through to
        // Directive{Unknown} so we don't mislabel it as kaeriten.
        run!(out, "［＃（これはおくりがなではない）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Kaeriten(_)))),
            "long parenthesised bodies must not be Kaeriten: {:?}",
            out.spans
        );
    }

    #[test]
    fn kaeriten_okurigana_with_latin_body_falls_through() {
        // Okurigana payload must be hiragana/katakana/kanji. ASCII
        // inside parens is probably an editorial note, not kaeriten.
        run!(out, "［＃（abc）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Kaeriten(_)))),
        );
    }

    #[test]
    fn kaeriten_okurigana_empty_parens_fall_through() {
        run!(out, "［＃（）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(Node::Kaeriten(_)))),
        );
    }

    // ---------------------------------------------------------------
    // Double-angle quotation `≪X≫`.
    // ---------------------------------------------------------------

    #[test]
    fn angle_quote_plain_body_produces_angle_quote_span() {
        run!(out, "前≪強調≫後");
        let aozora = out
            .spans
            .iter()
            .find_map(aozora_node)
            .expect("AngleQuote expected");
        let Node::AngleQuote(d) = aozora else {
            panic!("expected AngleQuote, got {aozora:?}");
        };
        assert_eq!(d.content.as_plain(), Some("強調"));
    }

    #[test]
    fn angle_quote_consumes_entire_source_span() {
        // Source `≪X≫` must fold into ONE Aozora span that covers
        // the angle brackets AND the body. No `≪` characters may
        // leak to the outer `spans` list.
        let src = "≪ABC≫";
        run!(out, src);
        let aozora_count = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .count();
        assert_eq!(
            aozora_count, 1,
            "one AngleQuote span expected: {:?}",
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
    fn angle_quote_with_nested_gaiji_folds_into_segments() {
        // The helper reuses `build_content_from_body`, so a `※［＃…］`
        // inside the angle brackets must surface as `Segment::Gaiji`
        // in the content — same invariant as nested gaiji in ruby.
        run!(out, "≪※［＃「ほ」、第3水準1-85-54］≫");
        let aozora = out
            .spans
            .iter()
            .find_map(aozora_node)
            .expect("Aozora expected");
        let Node::AngleQuote(d) = aozora else {
            panic!("expected AngleQuote, got {aozora:?}");
        };
        let Content::Segments(segs) = &d.content.get() else {
            panic!("expected Segments, got {:?}", d.content.get());
        };
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], Segment::Gaiji(_)));
    }

    #[test]
    fn angle_quote_empty_body_falls_through_to_plain() {
        // `≪≫` with no body is not classified as AngleQuote.
        // The empty payload would violate the
        // `borrowed::NonEmpty<Content>` invariant; instead the bytes
        // flow through as plain text (the catch-all `replay_unrecognised_body`
        // fold). No Aozora span is emitted for the empty case.
        run!(out, "A≪≫B");
        let aozora_count = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .count();
        assert_eq!(
            aozora_count, 0,
            "empty angle-quote must not emit a AngleQuote span — \
             empty content violates the NonEmpty<Content> invariant"
        );
    }

    #[test]
    fn container_open_indent_default_amount_one() {
        run!(out, "［＃ここから字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent {
                amount: 1,
                wrap: None,
                center: false
            })
        ));
    }

    #[test]
    fn container_open_indent_with_amount() {
        run!(out, "［＃ここから３字下げ］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent {
                amount: 3,
                wrap: None,
                center: false
            })
        ));
    }

    #[test]
    fn container_open_wrap_indent_parses_both_amounts() {
        run!(out, "［＃ここから２字下げ、折り返して４字下げ］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent {
                amount: 2,
                wrap: Some(4),
                center: false
            })
        ));
    }

    #[test]
    fn container_close_indent_matches_open_by_variant() {
        run!(out, "［＃ここから字下げ］本文［＃ここで字下げ終わり］");
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
        run!(out, "［＃ここから地付き］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::AlignEnd { offset: 0 })
        ));
        run!(out2, "［＃ここから地から2字上げ］");
        assert!(matches!(
            out2.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::AlignEnd { offset: 2 })
        ));
    }

    #[test]
    fn container_open_close_keigakomi() {
        run!(out, "［＃罫囲み］内部［＃罫囲み終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Framed)
        ));
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::Framed)
        ));
    }

    #[test]
    fn container_open_close_font_size_block() {
        // ここからN段階大きな/小さな文字 — the opener carries the signed
        // magnitude; the direction-only closer pairs by the font-size family.
        run!(
            out,
            "［＃ここから2段階大きな文字］大［＃ここで大きな文字終わり］"
        );
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::FontSize { steps: 2 })
        ));
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::FontSize { steps: 1 })
        ));
        run!(
            out2,
            "［＃ここから1段階小さな文字］小［＃ここで小さな文字終わり］"
        );
        assert!(matches!(
            out2.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::FontSize { steps: -1 })
        ));
    }

    #[test]
    fn warichu_open_close_are_inline_annotations() {
        // Aozora spec: `［＃割り注］…［＃割り注終わり］` is inline
        // (`<span class="aozora-warichu">…</span>`). The legacy block
        // form (`ここから割り注` / `ここで割り注終わり`) is deprecated
        // and not classified here.
        use aozora_syntax::DirectiveKind;
        run!(out, "［＃割り注］内部［＃割り注終わり］");
        let Some(Node::Directive(open)) = aozora_node(&out.spans[0]) else {
            panic!(
                "expected Aozora(Directive) for ［＃割り注］, got {:?}",
                out.spans[0].kind,
            );
        };
        assert_eq!(open.kind, DirectiveKind::WarichuOpen);
        assert_eq!(open.raw.as_str(), "［＃割り注］");

        let Some(Node::Directive(close)) = aozora_node(&out.spans[2]) else {
            panic!(
                "expected Aozora(Directive) for ［＃割り注終わり］, got {:?}",
                out.spans[2].kind,
            );
        };
        assert_eq!(close.kind, DirectiveKind::WarichuClose);
        assert_eq!(close.raw.as_str(), "［＃割り注終わり］");
    }

    #[test]
    fn container_close_without_matching_open_still_emits_close() {
        // Phase 3 does not pair opens with closes — that's `post_process`.
        // A bare `［＃罫囲み終わり］` is still classified.
        run!(out, "［＃罫囲み終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockClose(ContainerKind::Framed)
        ));
    }

    #[test]
    fn container_unknown_here_from_keyword_falls_through() {
        run!(out, "［＃ここから未知］");
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
        run!(out, "\n");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Newline);
        assert_eq!(out.spans[0].source_span, Span::new(0, 1));
    }

    #[test]
    fn diagnostics_from_phase2_are_forwarded() {
        run!(out, "stray］");
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
            run!(out, &src);
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
            run!(out, &src);
            for span in &out.spans {
                prop_assert!(span.source_span.end > span.source_span.start);
            }
        }

        /// Every Newline span covers exactly one byte at a `\n`
        /// position.
        #[test]
        fn proptest_newline_spans_are_single_byte(src in source_strategy()) {
            run!(out, &src);
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
        ///
        /// Determinism is asserted span-by-span; we cannot direct-`==`
        /// the two `ClassifyOutput`s across separate arenas because
        /// `borrowed::Node<'a>` `PartialEq` recurses through the
        /// arena-allocated payload pointers, which differ across runs
        /// even when the logical AST is identical. The pointer-aware
        /// equality is the right semantics — it lets the byte-identical
        /// proptest in `aozora-lex` pin pointer dedup. Here we want
        /// logical equality, so we compare via the `Debug` shape, which
        /// formats payload values rather than addresses.
        #[test]
        fn proptest_classify_is_deterministic(src in source_strategy()) {
            run!(a, &src);
            run!(b, &src);
            prop_assert_eq!(a.spans.len(), b.spans.len());
            for (l, r) in a.spans.iter().zip(b.spans.iter()) {
                prop_assert_eq!(l.source_span, r.source_span);
                prop_assert_eq!(format!("{:?}", l.kind), format!("{:?}", r.kind));
            }
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

    // -----------------------------------------------------------------
    // Forward-target index threshold smoke tests (G.4 / phase3 mod).
    //
    // The forward-reference target index is only built when a source
    // contains at least `FORWARD_QUOTE_BODY_THRESHOLD` (= 64) distinct
    // `「…」` quote bodies. Below the threshold we want to confirm the
    // pipeline still works and the result is identical to a re-run
    // (stability across the threshold gate).
    // -----------------------------------------------------------------

    /// Inputs *below* `FORWARD_QUOTE_BODY_THRESHOLD` skip the index
    /// build altogether. Drive a small input through the full lex
    /// pipeline twice and pin determinism — proves the gate decision
    /// (skip the AC index) doesn't itself perturb output.
    #[test]
    fn forward_target_index_handles_short_corpus() {
        // 5 distinct quote bodies — well below the 64-body threshold.
        let src = "「a」「b」「c」「d」「e」";
        run!(a, src);
        run!(b, src);
        assert_eq!(a.spans.len(), b.spans.len());
        for (l, r) in a.spans.iter().zip(b.spans.iter()) {
            assert_eq!(l.source_span, r.source_span);
            assert_eq!(format!("{:?}", l.kind), format!("{:?}", r.kind));
        }
    }

    /// Forward-reference behaviour DEPENDS on whether the cited target
    /// (`「青空」`) appears earlier in source.
    ///
    /// * With a preceding `「青空」`: the bouten classifier sees the
    ///   prior occurrence and recognises `［＃「青空」に傍点］` as
    ///   a Bouten span.
    /// * Without a preceding occurrence: `forward_target_is_preceded`
    ///   returns `false` and the recogniser falls through to
    ///   `Directive { kind: Unknown }` so the renderer doesn't apply
    ///   styling to a non-existent referent.
    ///
    /// The two outcomes must differ observably — this is the public
    /// behaviour gated on the forward-target lookup. We keep the
    /// assertion shape behavioural rather than poking at the
    /// thread-local index (which is non-public).
    #[test]
    fn forward_target_lookup_changes_output_for_preceded_vs_absent() {
        use aozora_syntax::borrowed::Node;

        // Case A: target exists earlier in source.
        let with_prior = "「青空」が見える。［＃「青空」に傍点］";
        run!(a, with_prior);
        let bouten_in_a = a
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_))));
        let unknown_in_a = a.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(Node::Directive(ann)) if ann.kind == DirectiveKind::Unknown
            )
        });

        // Case B: no prior `「青空」` occurrence.
        let without_prior = "ただの本文。［＃「青空」に傍点］";
        run!(b, without_prior);
        let bouten_in_b = b
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_))));
        let unknown_in_b = b.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(Node::Directive(ann)) if ann.kind == DirectiveKind::Unknown
            )
        });

        assert!(
            bouten_in_a && !unknown_in_a,
            "with prior `「青空」`, expected a Bouten span and no Unknown annotation, \
             got spans={:?}",
            a.spans
        );
        assert!(
            unknown_in_b && !bouten_in_b,
            "without prior `「青空」`, expected fallback Directive{{Unknown}} and no Bouten, \
             got spans={:?}",
            b.spans
        );
    }

    /// Regression: once a document crosses
    /// `FORWARD_QUOTE_BODY_THRESHOLD` the Aho-Corasick forward-target
    /// index installs and takes over from the substring fallback. It
    /// must give the SAME answer — including for the canonical
    /// `語句［＃「語句」に傍点］`, where the referent `語句` is *bare*
    /// text before the bracket and the only `「語句」` pair lives
    /// *inside* the directive (after the `［` cutoff).
    ///
    /// A prior index bug recorded each body's *quote* position rather
    /// than its first substring position; for the canonical shape that
    /// quote is the in-bracket one (past the cutoff), so
    /// `first_pos < cutoff` was always false and the bouten silently
    /// degraded to `Directive{Unknown}`. On real corpus this dropped
    /// ~114k 傍点/見出し/縦中横 occurrences (≈59 % of all
    /// `Directive{Unknown}`) the moment a work grew past 64 quotes —
    /// which every full-length work does. The short synthetic vectors
    /// never crossed the threshold, so the conformance suite stayed
    /// green throughout: this test deliberately crosses it.
    #[test]
    fn forward_bouten_recognised_above_ac_threshold_with_bare_target() {
        use aozora_syntax::borrowed::Node;

        // 70 distinct quote bodies → forces the AC index to install
        // (threshold is 64). None of them is `語句`, so the target's
        // only quoted occurrence is the one inside the directive.
        let mut src = String::new();
        for i in 0..70 {
            src.push_str("「ダミー");
            src.push_str(&i.to_string());
            src.push_str("」\n");
        }
        // Canonical forward bouten with a bare preceding referent.
        src.push_str("語句［＃「語句」に傍点］");

        run!(out, src.as_str());

        let has_bouten = out
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(Node::Bouten(_))));
        let degraded = out.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(Node::Directive(ann)) if ann.kind == DirectiveKind::Unknown
            )
        });
        assert!(
            has_bouten,
            "bare-target 傍点 must be recognised with the AC index installed; spans={:?}",
            out.spans
        );
        assert!(
            !degraded,
            "the 傍点 directive must not degrade to Directive{{Unknown}}; spans={:?}",
            out.spans
        );
    }

    /// Empty input is the "smallest possible corpus"; the pipeline
    /// must short-circuit cleanly without installing any thread-local
    /// state and produce no spans / no diagnostics.
    #[test]
    fn forward_target_index_handles_empty_corpus() {
        run!(out, "");
        assert!(out.spans.is_empty());
        assert!(out.diagnostics.is_empty());
    }
}
