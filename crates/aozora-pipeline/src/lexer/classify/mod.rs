//! Classify stage — classify the pair-stage event stream into [`Node`] spans.
//!
//! Walks the cross-linked [`PairEvent`] stream produced by the pair stage and
//! produces a contiguous vector of [`ClassifiedSpan`] whose
//! `source_span` values tile every byte of the sanitized source
//! end-to-end, in byte-offset order.
//!
//! The span kinds are:
//!
//! * [`SpanKind::Plain`] — a run of text that carries no Aozora
//!   construct. Adjacent un-classified events (text, stray triggers,
//!   unclosed opens, unmatched closes) are merged into one span so
//!   the normalize stage can emit them verbatim in a single write.
//! * [`SpanKind::Aozora`] — a classified Aozora construct, carrying the
//!   concrete [`Node`] that the normalize stage will replace
//!   with a PUA placeholder sentinel (see [`crate::INLINE_SENTINEL`] and friends).
//! * [`SpanKind::Newline`] — a `\n` in the sanitized text, kept as its
//!   own span kind because block-level annotations (normalize-stage block
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
//! The normalize stage relies on this invariant to emit `normalized` text
//! without ever re-scanning `source`.
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
//! plus tokenize-stage `flush_text` / `pair_text_then` / `try_merge_double`)
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
//! between stages, removing iterator chains) rather than attribute
//! hints.

use core::mem;
use core::ops::Range;
use std::collections::VecDeque;

#[cfg(feature = "classify-instrument")]
use super::instrumentation::{
    Subsystem, SubsystemGuard, YieldKind, record_pending_size, record_replay_body_size,
    record_yield,
};

// The classify stage builds the owned AST directly via `Allocator`'s
// inherent methods (single intern, no arena); the produced `Node`s thread
// straight into the lex output's `NodeStore`.
use aozora_syntax::alloc::Allocator;
use aozora_syntax::ast::{Content, Directive, Gaiji, Node, Segment};
use aozora_syntax::{DirectiveKind, RegionClose, RegionFormat, Span, ruby_base_class};

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
pub struct ClassifiedSpan {
    /// What the slice is (plain run, newline, or a concrete Aozora
    /// construct / container marker). Drives which sentinel, if any,
    /// the normalizer emits.
    pub kind: SpanKind,
    /// Half-open sanitized-source byte range this slice covers.
    /// Consecutive spans tile the source contiguously (see the
    /// span-coverage invariant in the module docs).
    pub source_span: Span,
}

/// Classification of a [`ClassifiedSpan`].
///
/// The normalize stage (now folded into `crate::lex`'s
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
/// The `Aozora(Node)` variant is *not* boxed —
/// `Node` is `Copy` and small (a handful of machine words), so
/// storing it inline keeps `SpanKind` to `Aozora`-variant size while
/// avoiding the `Box` indirection the legacy owned shape paid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanKind {
    /// Source bytes that carry no Aozora construct. Emitted verbatim
    /// by the normalizer.
    Plain,
    /// Classified Aozora construct (inline span or block-leaf line).
    /// The normalizer replaces the source span with an `E001` (inline)
    /// or `E002` (block-leaf) sentinel and records the node in the
    /// placeholder registry keyed at the sentinel's normalized
    /// position.
    Aozora(Node),
    /// Paired-container opener — `［＃ここから字下げ］`, `［＃罫囲み］`,
    /// etc. The normalizer emits an `E003` sentinel line; `post_process`
    /// matches it to the corresponding `BlockClose` via a balanced
    /// stack walk of the AST.
    BlockOpen(RegionFormat),
    /// Paired-container closer — `［＃ここで字下げ終わり］`,
    /// `［＃罫囲み終わり］`, etc. The normalizer emits an `E004`
    /// sentinel line; the carried [`RegionClose`] is a hint used by
    /// `post_process` to diagnose `［＃罫囲み終わり］` closing an
    /// `Indent` opener (kind mismatch).
    BlockClose(RegionClose),
    /// A `\n` in the sanitized text. Retained as its own span kind
    /// because block-level recognizers need line boundaries.
    Newline,
}

/// Classify a streaming pair-stage [`PairEvent`] iterator against the
/// sanitized source.
///
/// Returns a [`ClassifyStream`] iterator yielding one [`ClassifiedSpan`]
/// per call to [`Iterator::next`]. After exhaustion, call
/// [`ClassifyStream::take_diagnostics`] to drain non-fatal observations
/// accumulated during recognition. The upstream pair stream's
/// diagnostics are NOT forwarded automatically — the caller is
/// responsible for calling `pair_stream.take_diagnostics()` after the
/// classify stream is dropped (the fused pipeline in `aozora-pipeline` does
/// this).
///
/// Pure function; no I/O. The yielded spans byte-contiguously cover
/// `source` — see the module-level span-coverage invariant.
#[must_use]
pub fn classify<'src, 'al, I>(
    events: I,
    source: &'src str,
    alloc: &'al mut Allocator,
) -> ClassifyStream<'src, 'al, I::IntoIter>
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

/// Streaming classify-stage classifier.
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
#[expect(
    missing_debug_implementations,
    reason = "the &mut Allocator field cannot derive Debug; the iterator is opaque to consumers"
)]
pub struct ClassifyStream<'src, 'al, I>
where
    I: Iterator<Item = PairEvent>,
{
    events: I,
    source: &'src str,
    source_len: u32,
    alloc: &'al mut Allocator,
    /// Buffered ready-to-yield spans drained one-per-`next()` by the
    /// consumer. `VecDeque` (not `SmallVec`) because the consumer pulls
    /// from the front and `SmallVec::remove(0)` is `O(N)`: the
    /// `replay_unrecognised_body` path can push thousands of spans at
    /// once for top-level unrecognised paired containers (e.g. doc 49178
    /// in the corpus emits ~16k pending spans), and the per-yield
    /// front-pop turns into a quadratic memmove storm. `VecDeque` is a
    /// ring buffer with `O(1)` `push_back` / `pop_front`, eliminating
    /// the back-shift entirely.
    pending_outputs: VecDeque<ClassifiedSpan>,
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
    /// A just-recognised gaiji held back one step so an immediately-
    /// following `《…》` ruby can take it as its base (`※［＃…］《みは》`).
    /// A gaiji resolves to a glyph distinct from its `※［＃…］` source and
    /// is emitted as its own node, so a ruby cannot reach back to reclaim
    /// it once yielded — instead the emit is deferred here. Flushed as a
    /// standalone gaiji span by `process_event` / `finalize` when the next
    /// event is not the adjacent ruby that would consume it.
    pending_ruby_base: Option<PendingRubyBase>,
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

/// One deferred gaiji: its ready-to-yield standalone span (used when no
/// ruby follows) and the payload that rebuilds the glyph as a
/// `Segment::Gaiji` inside a ruby base when one does.
struct PendingGaiji {
    span: ClassifiedSpan,
    payload: Gaiji,
}

/// A source-contiguous run of one or more deferred gaiji, held one step so
/// an adjacent `《…》` ruby can adopt the whole run as its base. A single
/// gaiji is the common `※［＃…］《みは》` case; a run of adjacent gaiji is an
/// ateji whose reading spans several glyphs (`※［＃…］※［＃…］《かいがい》`).
///
/// `bar` is the span of an explicit `｜` immediately preceding the FIRST
/// gaiji (`元｜※［＃…］《…》`): it is held out of the preceding plain run so
/// that, if a ruby adopts the run, the redundant base-marker `｜` is dropped
/// instead of leaking (the gaiji run is unambiguously the base) while the
/// ruby's source span still covers it for the tiling invariant. If no ruby
/// follows, the `｜` and every gaiji are re-emitted in source order
/// (`emit_pending_gaiji`).
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

    /// Source offset where the base starts — the held `｜` if any, else the
    /// first gaiji. Used for the adopted ruby's tiling span.
    fn start(&self) -> u32 {
        self.bar
            .map_or(self.segs[0].span.source_span.start, |b| b.start)
    }

    /// Source offset just past the last gaiji — the adjacency anchor for the
    /// next continuation gaiji or the adopting `《…》`.
    fn end(&self) -> u32 {
        self.segs
            .last()
            .expect("a PendingRubyBase always holds ≥1 gaiji")
            .span
            .source_span
            .end
    }
}

/// Outcome of [`ClassifyStream::try_ruby_over_gaiji_base`].
enum GaijiBaseRuby {
    /// No adjacent deferred gaiji — continue to the plain-base path.
    NotApplicable,
    /// A ruby over the gaiji base was formed.
    Emitted(ClassifiedSpan),
    /// The reading was empty (or the pair malformed); the gaiji was
    /// flushed standalone and the `《》` falls through to plain replay.
    Declined,
}

/// Body window passed to recogniser helpers.
///
/// `events` is a contiguous body slice (between matched
/// `PairOpen`/`PairClose`); `links[i]` gives the body-local index of
/// the matching `PairOpen`/`PairClose` for `events[i]` if it's a
/// paired event (`u32::MAX` otherwise). Both slices are the same
/// length and are constructed by [`ClassifyStream`]'s frame buffers.
///
/// The split keeps [`PairEvent`] free of cross-link fields (the pair stage
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
/// Bundles the (allocator, sanitized source) pair that every classify-stage
/// recogniser / classifier helper needs but doesn't vary per call
/// within a single recognise pass. Threading it as a single `&mut`
/// argument keeps each helper's signature at ≤4 args (project-rule
/// `clippy.toml::too-many-arguments-threshold = 4`) without losing
/// the per-call positional clarity of the body-window indices.
///
/// The two lifetimes deliberately stay distinct:
/// - `'al` — the borrow lifetime of the `&mut alloc` reference
/// - `'s`  — the sanitized source lifetime
///
/// Strings the recognisers intern are owned by the allocator's
/// `NodeStore` (not borrowed from the source), so there is no arena
/// lifetime to thread; keeping `'al` and `'s` separate still avoids
/// over-constraining helpers that thread synthetic source slices
/// through `Cow`.
pub(crate) struct RecogniseCtx<'al, 's> {
    pub alloc: &'al mut Allocator,
    pub source: &'s str,
    /// Non-fatal diagnostics raised while building *nested* content
    /// (a gaiji inside a ruby / annotation reading). The owning
    /// `ClassifyStream` drains this into its own sink after each
    /// recognise call — a `RecogniseCtx` is a short-lived per-call view,
    /// so an owned `Vec` avoids threading a `&mut` sink (and a fourth
    /// lifetime) through every recogniser.
    pub diagnostics: Vec<Diagnostic>,
    /// Byte offset where the enclosing top-level pending plain run began,
    /// or `None` when no plain run is open (or this is a nested-content
    /// view, where forward references never resolve). Read by the forward
    /// recognizers to locate a non-adjacent referent *inside* that run and
    /// splice a styled decoration at it (#333).
    pub pending_plain_start: Option<u32>,
    /// Output channel: a styled decoration leaf a forward recognizer carved
    /// out of the pending plain run at its interior referent, plus that
    /// referent's source span (#333). `try_bracket_emit` drains it and
    /// splices the leaf into the plain run before flushing the tail. `None`
    /// for every other outcome (adjacent / self-contained / declined /
    /// non-forward).
    pub pending_decoration: Option<(Node, Span)>,
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

/// What [`ClassifyStream::append_to_frame`] tells its caller to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameOutcome {
    /// The outermost pair is still open — keep buffering.
    Open,
    /// The outermost pair just closed on a real `PairClose` — run
    /// recognition on the now-complete body buffer.
    Closed,
    /// The outermost open was force-resolved as `Unclosed` (a stray `［`
    /// crossing a line break, or the EOF drain) — abandon the frame,
    /// folding its buffered fragment to plain (F21). See
    /// [`ClassifyStream::abandon_frame`].
    Abandoned,
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

/// The synthetic event stream, its parallel link table, and the index of
/// the synthetic `《` open, as returned by [`build_synth_ruby_view`]. Both
/// buffers are inline-16 `SmallVec`s, so a ruby (~5 events) allocates no
/// heap.
type SynthRubyView = (
    smallvec::SmallVec<[PairEvent; 16]>,
    smallvec::SmallVec<[u32; 16]>,
    usize,
);

/// Build the synthetic event / link stream `recognize_ruby` expects in the
/// streaming model: `[optional Solo(Bar), Text(base), ...body events...]`,
/// with links shifted to account for the prepended prefix. Returns the
/// stream, its parallel link table, and the index of the synthetic `《`
/// open. `prev_text_range` is the preceding plain run (`start` = its byte
/// offset, `end` = the `《` open); `bar_byte_offset` is the position of a
/// `｜` inside it for the explicit form. Returns `None` when an explicit
/// `｜` base would leave the base text empty.
///
/// The two buffers are `SmallVec<[_; 16]>`: a ruby is `[Text(base),
/// PairOpen, reading…, PairClose]` — ~5 events — so the synth stays inline
/// and allocates zero heap for the ~200 ruby/file that dominate the parse.
/// (Plain `Vec::with_capacity` here was ~67% of ALL owned-pipeline heap
/// allocations — two mallocs per ruby.) The `[16]` inline size matches
/// `Frame::body`, whose contents this is a superset of.
fn build_synth_ruby_view(
    body: BodyView<'_>,
    prev_text_range: Span,
    bar_byte_offset: Option<usize>,
) -> Option<SynthRubyView> {
    let mut synth: smallvec::SmallVec<[PairEvent; 16]> =
        smallvec::SmallVec::with_capacity(body.events.len() + 2);
    let mut synth_links: smallvec::SmallVec<[u32; 16]> =
        smallvec::SmallVec::with_capacity(body.events.len() + 2);
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

impl<'src, 'al, I> ClassifyStream<'src, 'al, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn new(events: I, source: &'src str, alloc: &'al mut Allocator) -> Self {
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
            pending_ruby_base: None,
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
    /// narrower scope or stricter ladder flags valid kanbun (per-
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

    fn push_output(&mut self, span: ClassifiedSpan) {
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
        let _classify_guard = SubsystemGuard::new(Subsystem::FlushPlain);
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

    /// Splice a styled decoration leaf into the *middle* of the pending plain
    /// run at an interior referent (#333). Where [`Self::flush_plain_up_to`]
    /// only truncates the tail, this opens an interior hole:
    /// `Plain[pending_start, deco.start]` · `deco` · then re-seeds the pending
    /// run at `deco.end` so the tail (up to the bracket's `consume_start`)
    /// flushes normally afterward. Pre: the caller has verified the pending
    /// run is open and `pending_start <= deco_span.start`.
    fn splice_plain_around(&mut self, deco: Node, deco_span: Span) {
        // Head plain run before the referent (often empty → emits nothing).
        self.flush_plain_up_to(deco_span.start);
        self.push_output(ClassifiedSpan {
            kind: SpanKind::Aozora(deco),
            source_span: deco_span,
        });
        // Re-seed: the tail from the referent's end up to the bracket stays
        // plain and is flushed by the caller's `flush_plain_up_to`.
        self.pending_plain_start = Some(deco_span.end);
    }

    /// Open a new top-level frame. `gaiji_refmark` is `Some(span)` when
    /// the outer open was preceded by a `Solo(RefMark)` waiting to be
    /// absorbed (the gaiji shape).
    fn open_frame(&mut self, open_event: PairEvent, gaiji_refmark: Option<Span>) {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::OpenFrame);
        let mut body: smallvec::SmallVec<[PairEvent; 16]> = smallvec::SmallVec::new();
        let mut links: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
        // Inner stack tracks NESTED opens; the outer open lives at
        // body[0], so we record its position there.
        let mut inner_stack = smallvec::SmallVec::new();
        let &PairEvent::PairOpen { kind, .. } = &open_event else {
            // INVARIANT(classify): open_frame is only ever entered with a
            // PairOpen — established by its two call sites, both of which pass
            // a freshly built PairEvent::PairOpen; exercised by the `classify`
            // fuzz target's span-tiling assertions.
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
    /// needed. The [`FrameOutcome`] tells the caller whether to keep
    /// buffering, run recognition on the now-complete buffer, or
    /// abandon the frame as a stray open (fold its fragment to plain).
    fn append_to_frame(&mut self, event: PairEvent) -> FrameOutcome {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::FrameAppend);
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
                // Find the matching open via the inner stack. The pair stage
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
                    // happen because the pair stage's stack-balance contract
                    // means a PairClose only arrives when the outer
                    // stack matches; but be defensive and append as-is.
                    frame.body.push(event);
                    frame.links.push(u32::MAX);
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                // The pair stage force-resolved an open. Two shapes reach here:
                //
                // * A hard-scope `］` (or a line break, see `pair.rs`) unwound a
                //   NON-outermost open buried above a bracket. Drop its
                //   inner-stack entry so the buffer can still close on the
                //   following `PairClose(Bracket)`; the event stays in the body
                //   with no link (`u32::MAX`) so forward recognisers see an
                //   unresolved pair and decline — the body round-trips raw.
                //
                // * The frame's OUTERMOST open (inner-stack position 0) is being
                //   force-resolved, because the pair stage crossed a line break
                //   with a stray `［` still open (F21) or drained it at EOF. This
                //   open may never be closed by a real `PairClose`, so the frame
                //   is a stray delimiter, not an annotation body: abandon it —
                //   fold the buffered fragment to plain and resume top-level
                //   classification on the live stream (see `abandon_frame`).
                if let Some(pos) = frame.inner_stack.iter().rposition(|&(k, _)| k == *kind) {
                    if pos == 0 {
                        return FrameOutcome::Abandoned;
                    }
                    frame.inner_stack.remove(pos);
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

    /// Run recognition on the current frame's body buffer and emit the
    /// resulting span. Called when the OUTERMOST pair has just closed.
    fn recognize_and_emit(&mut self) {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::RecognizeAndEmit);
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
            // INVARIANT(classify): body[0] is the frame's outer open —
            // established by open_frame, which pushes the PairOpen at body[0]
            // before any other event is appended; exercised by the `classify`
            // fuzz target.
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
                    if let Some((gaiji, bar)) = self.try_gaiji_emit(view, open_idx, rm_span) {
                        // Defer the emit one step: an immediately-following
                        // `《…》` ruby can adopt this gaiji as its base
                        // (`※［＃…］《みは》`). When this gaiji is
                        // source-adjacent to an already-pending run, extend
                        // that run instead of replacing it, so an ateji
                        // spanning several gaiji (`※…※…《かいがい》`) forms one
                        // multi-segment base. `process_event` / `finalize`
                        // flush the run as standalone spans when the next
                        // event neither continues nor adopts it.
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
                    // Gaiji recognition declined. A run held for a possible
                    // ruby base must flush first so it precedes this non-gaiji
                    // directive in source order (the held refmark looked like
                    // a continuation until recognition declined). Then fold
                    // the refmark bytes into the pending plain run and attempt
                    // a normal bracket annotation on the original body.
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
        let _classify_guard = SubsystemGuard::new(Subsystem::ReplayBody);
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

    /// Abandon the current frame because its outermost open was force-resolved
    /// as `Unclosed` — the pair stage either crossed a line break with a stray
    /// `［` still open (F21, mid-stream) or drained the still-open outer at EOF.
    /// Either way the open may never be closed by a real `PairClose`, so the
    /// frame is a stray delimiter, not an annotation body (江戸川乱歩『影男』L548's
    /// trailing `［` is the corpus archetype).
    ///
    /// The win over the old EOF-only drain is timing: resolving a stray `［` at
    /// the line break — rather than buffering the entire remainder of the
    /// document into its frame and sinking it to plain at EOF — folds only the
    /// buffered *line fragment* to plain and clears the frame, so the pair
    /// events after the newline classify normally on the *live* stream, exactly
    /// as if the stray open were not there. That stops the leak cascade where
    /// every following ruby, heading and directive rendered as literal source.
    ///
    /// This never re-classifies already-paired events (that is unsound: their
    /// pairing reflects the stray open's stack context); the tail is processed
    /// once, forward, off the real event stream. Byte coverage is unchanged —
    /// the fragment folds to plain, identical to the EOF replay path — so the
    /// verbatim / round-trip invariants hold; only the visible render improves.
    fn abandon_frame(&mut self) {
        let frame = self
            .frame
            .take()
            .expect("abandon_frame requires an active frame");
        let refmark = frame.gaiji_refmark;
        self.replay_unrecognised_body(frame.body, refmark);
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
    /// Fold a held `※` refmark into the plain run unless the very next
    /// event is the `［` that consumes it (the gaiji shape). The streaming
    /// mirror of the top-level refmark block in `process_event`, which is
    /// skipped while streaming because the stream-through check routes to
    /// `handle_stream_event` first — so an orphan `※` inside a quote
    /// round-trips as plain, exactly as at top level.
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

    fn handle_stream_event(&mut self, event: PairEvent) {
        self.fold_held_refmark(&event);

        // A nested Ruby / AngleQuote / Bracket inside the streamed
        // Quote/Tortoise is recognised WITHOUT touching the streaming depth
        // — so do it before borrowing the streaming frame. This is what
        // makes `「駄目《だめ》」` form a ruby, a nested `≪…≫` resolve, and an
        // in-quote directive / gaiji — `「…［＃「X」に傍点］…」`,
        // `「…※［＃…水準…］《みは》…」` — resolve instead of leaking as literal
        // text: the sub-frame runs the same recogniser as at top level, and
        // `pending_plain_start` (kept live through the quote) supplies the
        // ruby base or the forward-reference target. `process_event` checks
        // `frame` before `streaming`, so the sub-frame's body buffers
        // correctly and streaming resumes once it closes.
        if let PairEvent::PairOpen { kind, span } = &event
            && matches!(
                kind,
                PairKind::Ruby | PairKind::AngleQuote | PairKind::Bracket
            )
        {
            // None of the three flush `pending_plain_start`, so the
            // recogniser can pull `consume_start` back over the preceding
            // text. Only a Bracket absorbs a held `※` refmark (the gaiji
            // shape); Ruby / AngleQuote never do. Routing an in-quote gaiji
            // through this Bracket sub-frame lets `try_gaiji_emit` null
            // `pending_plain_start`, which is exactly the invariant
            // `try_ruby_over_gaiji_base` needs to adopt an adjacent `《…》`.
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
                // The pair stage emits synthetic Unclosed events when EOF
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
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } => {
                // Hold the `※` pending the next event — the streaming
                // mirror of the top-level `handle_top_level` refmark arm.
                // The block at the top of this method folds it to plain if
                // no `［` follows, or the Bracket sub-frame absorbs it as
                // the gaiji shape.
                self.pending_refmark = Some(span);
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

    /// Form a ruby whose base is a deferred gaiji held in
    /// `pending_ruby_base` (`※［＃…］《みは》`) — the gaiji resolves to a glyph
    /// distinct from its source and was emitted as its own node, so there
    /// is no plain run to walk back over. Adopts the gaiji as a
    /// `Segment::Gaiji` base; the reading is built from the `《…》` body as
    /// for a plain-base ruby. See [`GaijiBaseRuby`] for the outcomes.
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
            // Empty `《》` reading — not a ruby. The gaiji stands alone and
            // the empty pair falls through to plain replay.
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
                    events: open_idx + 1..close_idx,
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
            // Cover the whole gaiji run and a dropped `｜` base-marker so the
            // ruby region still tiles the source gap-free (the marker is
            // consumed, not rendered).
            source_span: Span::new(base_start, close_span.end),
        })
    }

    fn try_ruby_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::TryRubyEmit);
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

        // Gaiji-base ruby: the immediately-preceding construct is a
        // deferred gaiji (`※［＃…］《みは》`) rather than a plain run. Fall
        // through to the plain-base path only when not applicable.
        match self.try_ruby_over_gaiji_base(body, open_idx, close_idx) {
            GaijiBaseRuby::Emitted(span) => return Some(span),
            GaijiBaseRuby::Declined => return None,
            GaijiBaseRuby::NotApplicable => {}
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
        let node = self.alloc.ruby(base_content, m.reading);
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
    /// here would violate the `NonEmpty` invariant on the
    /// `Content` payload.
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
            // INVARIANT(classify): open_idx addresses the frame's outer open —
            // established by open_frame (PairOpen at body[0]); exercised by the
            // `classify` fuzz target.
            unreachable!("body[open_idx] must be PairOpen");
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            // INVARIANT(classify): close_idx addresses the frame's outer close —
            // established by append_to_frame, which appends the PairClose that
            // closes the outermost pair at body[len - 1]; exercised by the
            // `classify` fuzz target.
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
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );
        // Drain diagnostics raised while building the nested angle-quote body.
        self.diagnostics.append(&mut ctx.diagnostics);
        // Empty `≪≫` is not a valid AngleQuote — let the bytes
        // flow through as plain text. The caller's fall-through path
        // (`replay_unrecognised_body`) handles the plain emission.
        let content_is_empty = match content {
            Content::Plain(s) => self.alloc.store().resolve_str(s).is_empty(),
            Content::Segments(segs) => segs.len == 0,
            _ => false,
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
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::TryBracketEmit);
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
            diagnostics: Vec::new(),
            // The forward recognizers resolve a non-adjacent referent inside
            // the current pending plain run (#333); hand them its start.
            pending_plain_start: self.pending_plain_start,
            pending_decoration: None,
        };
        let m = ctx.recognize_annotation(body, open_idx, close_idx)?;
        // Drain diagnostics raised while building nested reading content
        // (a gaiji inside a left-ruby / annotation reading) into our sink,
        // and take the decoration the forward recognizer may have carved out
        // (#333). Both reads are the last use of `ctx`, so its reborrow of
        // `self.alloc` ends here (NLL) and the splice below gets full `self`.
        self.diagnostics.append(&mut ctx.diagnostics);
        let decoration = ctx.pending_decoration.take();
        // #333: if the recognizer resolved a non-adjacent interior referent,
        // splice a styled decoration leaf into the pending plain run *before*
        // flushing the tail up to the bracket. The window invariant is
        // re-checked defensively (the recognizer computed the span against the
        // same `pending_plain_start`, so this holds unless a pending refmark
        // moved the run — in which case we decline and leave today's bytes).
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
        // An explicit `｜` (U+FF5C) immediately before the gaiji is a
        // base-start marker for a following ruby. Hold it out of the plain
        // run so `try_ruby_over_gaiji_base` can drop the redundant marker on
        // adoption (the gaiji is unambiguously the base), or `emit_pending_gaiji`
        // can re-emit it as plain when the gaiji stands alone. Consume the
        // WHOLE trailing `｜` run, not just the last bar: dropping only one
        // per parse would leave the next `｜` adjacent to the gaiji, so
        // re-serialising `｜｜※…《…》` would keep peeling one bar off each pass
        // and never reach a fixed point (fmt-idempotence).
        let before = &self.source[..m.consume_start as usize];
        let bar_start = before.trim_end_matches('\u{ff5c}').len();
        let bar = (bar_start < before.len()).then(|| {
            Span::new(
                u32::try_from(bar_start).expect("bar-run start is within the source (u32)"),
                m.consume_start,
            )
        });
        self.flush_plain_up_to(bar.map_or(m.consume_start, |b| b.start));
        let node = self.alloc.gaiji(m.payload);
        self.pending_plain_start = None;
        // The gaiji still renders best-effort (as its description text)
        // when resolution misses; flag the miss so authors know the glyph
        // won't appear. `m.payload` is a `Copy` value and the
        // `ctx` reborrow of `self.alloc` ended at the `gaiji()` call above,
        // so reading `ucs` and pushing onto `self.diagnostics` is clear of
        // the borrow. Scope: this `unresolved-gaiji` warning fires for
        // top-level `※［＃…］` only. Gaiji nested in a ruby reading / annotation
        // body are still resolved + rendered (by `build_content_from_body`),
        // but without this diagnostic; a gaiji buried in a forward-reference
        // quote target (nested `［＃…］` breaks pairing) falls to `Unknown`.
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

    /// Emit a deferred gaiji run that no ruby adopted: re-emit its held `｜`
    /// base-marker (if any) as plain first — restoring the `元｜※［＃…］`
    /// shape — then every gaiji span in source order.
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

impl<I> Iterator for ClassifyStream<'_, '_, I>
where
    I: Iterator<Item = PairEvent>,
{
    type Item = ClassifiedSpan;

    fn next(&mut self) -> Option<ClassifiedSpan> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::IterDispatch);
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
                let _classify_loop_guard = SubsystemGuard::new(Subsystem::LoopBody);
                self.process_event(event);
            } else {
                // Upstream exhausted. A deferred gaiji with no following
                // ruby is a standalone span — flush it FIRST (it precedes
                // any dangling frame's body in source order). Then close
                // any active frame as unclosed (its body events fold back
                // to plain; a gaiji-mode refmark also falls into plain),
                // then run final flush.
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
        // Frame buffering comes first — checked BEFORE `streaming` so a
        // sub-frame opened mid-stream for a nested Ruby / AngleQuote
        // (`《…》` / `≪…≫` inside a `「…」` quote — see
        // `handle_stream_event`) actually accumulates its body instead of
        // the quote's stream-through path swallowing it. Once the
        // sub-frame's outer pair closes, recognition runs and the frame
        // clears, so `streaming` resumes on the next event.
        if self.frame.is_some() {
            // A pending refmark cannot coexist with an open frame: it is
            // absorbed into the frame's `gaiji_refmark` or folded to
            // plain before the frame opens.
            debug_assert!(
                self.pending_refmark.is_none(),
                "a pending refmark should have been absorbed or flushed before frame entry"
            );
            match self.append_to_frame(event) {
                FrameOutcome::Closed => self.recognize_and_emit(),
                FrameOutcome::Abandoned => self.abandon_frame(),
                FrameOutcome::Open => {}
            }
            return;
        }

        // A deferred gaiji run (`pending_ruby_base`) is HELD when the next
        // event continues or adopts it — an adjacent `《…》` ruby adopts it as
        // a base, and an adjacent `※` refmark (or its Bracket, once the
        // refmark is held) starts a continuation gaiji that extends the run.
        // Any other event flushes the run as standalone spans first,
        // preserving source order. Checked after the frame guard so the
        // ruby's own body events (while its sub-frame buffers) never flush it
        // early.
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

        // Stream-through path for top-level Quote / Tortoise — see
        // `StreamingFrame` for the rationale. Body events flow straight
        // through, except a nested Ruby / AngleQuote opens a sub-frame
        // (buffered by the frame check above).
        if self.streaming.is_some() {
            self.handle_stream_event(event);
            return;
        }

        // Top level. A `※` refmark held from the previous event is
        // absorbed only by an immediately-following Bracket open (gaiji);
        // otherwise it folds into the plain run (its span is picked up
        // because `pending_plain_start` is set to `rm.start` first).
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
/// the renderer) keeps the [`Node`] payload self-contained:
/// The normalize stage stamps one PUA sentinel over the whole `｜…《…》` source
/// span, and the inner gaiji/annotation never reach the top-level
/// `spans` list or downstream consumers.
struct RubyMatch<'s> {
    base: &'s str,
    reading: Content,
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
impl<'s> RecogniseCtx<'_, 's> {
    fn recognize_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<RubyMatch<'s>> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::Ruby);
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
                consume_start: bar_span.start,
                consume_end: close_span.end,
            });
        }

        // Implicit form: the trailing same-class base run of the preceding
        // Text (kanji, or a non-kanji word/letter run — see
        // `trailing_ruby_base_start`).
        let base_offset = trailing_ruby_base_start(prev_text);
        if base_offset == prev_text.len() {
            return None;
        }
        let consume_start =
            prev_range.start + u32::try_from(base_offset).expect("base offset fits in u32");
        Some(RubyMatch {
            base: &prev_text[base_offset..],
            reading,
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
/// recognition jumps to `close_idx + 1` using the pair stage's pre-linked
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
struct ContentBuild {
    segments: Vec<Segment>,
    /// Byte position of the earliest source byte not yet committed
    /// to a `Segment::Text`. Each successful gaiji / annotation emit
    /// advances this past the consumed bracket.
    text_start: u32,
}

impl RecogniseCtx<'_, '_> {
    /// Build a [`Content`] for the body
    /// window, recognising any nested gaiji / annotation constructs in
    /// a single forward sweep.
    ///
    /// Fast path returns when the body has no `※` and no `［` —
    /// emits the raw byte run as a single `Plain`. The slow path
    /// dispatches each event index through two per-shape recognise
    /// helpers and falls through (advancing `i`) when neither claims
    /// the slot.
    fn build_content_from_body(&mut self, view: BodyView<'_>, window: &BodyWindow) -> Content {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::BuildContent);
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
        if g.payload.resolve(self.alloc.store()).is_none() {
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
        if close_idx >= body.window.events.end {
            return None;
        }
        let a = self.recognize_annotation(body.view, i, close_idx)?;
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.view.events[close_idx]
        else {
            // INVARIANT(classify): a PairOpen's link always targets a matching
            // PairClose — established by the pair stage's link side-table, which
            // only ever points an open at its resolved close; exercised by the
            // `classify` fuzz target.
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
        if let EmitKind::Aozora(Node::Gaiji(g)) = a.emit {
            build.segments.push(self.alloc.seg_gaiji(g));
            if g.resolve(self.alloc.store()).is_none() {
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

/// Whether `body` could host a nested gaiji / annotation. The pair-stage
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

/// Byte offset where the trailing kanji run in `text` begins.
///
/// Walks chars right-to-left, keeping track of the earliest byte
/// offset reached while every char is a ruby-base char. Returns
/// `text.len()` if the final char is not a ruby-base char (→ no
/// implicit base available).
/// Byte offset where the trailing implicit-ruby base run of `text` starts
/// — the maximal run of a single `RubyBaseClass` ending at `text`'s end.
/// Returns `text.len()` when the last char is not a base char (no implicit
/// base). For a kanji-ending run this is byte-for-byte the historical
/// `trailing_kanji_start` (the `Kanji` class equals the old set); a
/// non-kanji-ending run now yields its same-class base (`Yahoo《ヤフー》`,
/// `α《アルファ》`), never crossing into an adjacent class.
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
struct AnnotationMatch {
    emit: EmitKind,
    annotation_payload: Option<Directive>,
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
enum EmitKind {
    /// Inline or block-leaf — becomes [`SpanKind::Aozora`].
    Aozora(Node),
    /// Paired-container opener — becomes [`SpanKind::BlockOpen`]. Carries the
    /// authoritative open [`RegionFormat`].
    BlockOpen(RegionFormat),
    /// Paired-container closer — becomes [`SpanKind::BlockClose`]. Carries the
    /// [`RegionClose`] discriminant (the open payload stays authoritative).
    BlockClose(RegionClose),
}

// `ruby_base_class` lives in `aozora_syntax` (single source of truth
// shared with the serializer's bare-vs-`｜` decision); imported at the top
// of this module and driven here by `trailing_ruby_base_start`.

#[cfg(test)]
mod tests {
    //! Owned-native classify-stage tests.
    //!
    //! The classifier now builds the owned AST directly, so these tests assert
    //! on `Node` spans and resolve their payloads through the
    //! `Allocator`'s `NodeStore`. End-to-end byte-identity of the rendered
    //! output is pinned separately by the conformance vectors, the corpus
    //! verbatim gate, and the render byte-identity gates — the frozen authority.

    use super::*;
    use aozora_syntax::ast::{Content, ContentRange, Node, NodeStore, Segment, StrId};
    use aozora_syntax::{BoutenKind, BoutenPosition, ForwardAttr, ForwardOrigin, SectionKind};

    use crate::lexer::pair::pair;
    use crate::lexer::tokenize::tokenize;

    /// Materialised classify output plus the backing store the owned payloads
    /// resolve against.
    #[derive(Debug)]
    struct TestClassifyOutput {
        spans: Vec<ClassifiedSpan>,
        diagnostics: Vec<Diagnostic>,
        store: NodeStore,
    }

    impl TestClassifyOutput {
        /// Resolve a length-1 content run to its plain text (`None` for a
        /// `Segments`/multi run) — the owned analogue of `Content::as_plain`.
        fn plain(&self, range: ContentRange) -> Option<&str> {
            self.store.content_range_as_plain(range)
        }

        /// Resolve a `StrId` to its interned bytes.
        fn s(&self, id: StrId) -> &str {
            self.store.resolve_str(id)
        }

        /// Resolve a content run to its `Content` slice.
        fn contents(&self, range: ContentRange) -> Vec<Content> {
            self.store.resolve_content_range(range).to_vec()
        }

        /// The single `SpanKind::Aozora` node, panicking if not exactly one.
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

    /// Materialise a fresh owned classify run, binding `out` to a
    /// [`TestClassifyOutput`].
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

    /// F21: a stray `［` that the pair stage line-scopes must not sink the
    /// ruby on the next line. The classifier abandons the never-closing frame
    /// at the line break — folding the `［` to a literal Plain span — so the
    /// following `｜…《…》` still classifies as a Ruby node instead of leaking as
    /// literal source (江戸川乱歩『影男』L548's cascade). Pinned end-to-end by
    /// `fixtures/render/stray_bracket_line_scope`.
    #[test]
    fn stray_bracket_does_not_sink_following_ruby() {
        run!(out, "［\n｜漢字《かんじ》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("post-stray ruby must survive, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("漢字"));
        assert_eq!(out.plain(r.reading), Some("かんじ"));
        // The stray `［` (U+FF3B, 3 bytes) folds to a literal Plain span.
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
    fn unknown_annotation_is_directive_not_bare_bracket() {
        run!(out, "［＃まったく未知の注記です］");
        let Node::Directive(d) = out.only_aozora() else {
            panic!("expected a Directive, got {:?}", out.only_aozora());
        };
        assert_eq!(d.kind, DirectiveKind::Unknown);
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

    // ---- mutation-survivor kills (classify/mod.rs) ----

    impl TestClassifyOutput {
        /// Count spans whose kind is `SpanKind::Plain`.
        fn plain_count(&self) -> usize {
            self.spans
                .iter()
                .filter(|s| s.kind == SpanKind::Plain)
                .count()
        }

        /// The single `SpanKind::Aozora` span (its `source_span` + node),
        /// panicking if not exactly one.
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

    /// A deferred gaiji is adopted as a ruby base (`※［＃…］《みは》`). Pins that
    /// the result is a `Ruby` (not a standalone gaiji + plain `《…》`), that its
    /// reading is `みは`, and that the ruby span starts at the held gaiji's `※`
    /// (byte 3, after the leading `あ`) rather than at 0/1. Kills the
    /// `PendingRubyBase::start`/`end` stubs, the gaiji-base applicability /
    /// empty-reading comparisons, the `open_idx+1` reading-window offset, the
    /// `try_gaiji_emit -> None` stub, and the `pending_ruby_base` continuation
    /// arms in `process_event`.
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

    /// A run of two adjacent deferred gaiji is adopted as ONE multi-segment
    /// ruby base (`※…※…《かい》`). Under the mutations the run is split, so the
    /// first gaiji leaks as its own standalone span alongside the ruby. Pins
    /// that exactly one Aozora span (the ruby) is emitted. Kills the
    /// `adjacent` equality and the two `pending_ruby_base` continuation arms /
    /// their `== end` guards.
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

    /// An explicit `｜base《reading》` at the very start of the source is a
    /// single Ruby span covering `[0, end]` — no empty leading Plain, and the
    /// explicit `｜` form must be taken (not the implicit trailing-kanji form,
    /// which would drop the `｜` into a separate Plain span). Kills the
    /// `flush_plain_up_to` `>`→`>=` empty-span mutation and the
    /// `open_idx - 2` explicit-bar index mutations.
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

    /// #333 interior forward-reference: the bouten target `青空` occurs at the
    /// start of the pending plain run but is not byte-adjacent to the bracket
    /// (`青空だ。［＃「青空」に傍点］`), so the classifier splices a `Detached`
    /// decoration at `[0, 6)` into the middle of the plain run. Kills the
    /// `splice_plain_around -> ()` stub and the two decoration-window
    /// `<=`→`>` comparisons in `try_bracket_emit`.
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

    /// A top-level `\n` covers exactly one byte at its own position (not zero
    /// bytes). Kills the `pos + 1`→`pos * 1` Newline-span mutation.
    #[test]
    fn top_level_newline_span_covers_one_byte() {
        run!(out, "a\nb");
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].source_span, Span::new(1, 2));
    }

    /// A `\n` inside a streamed top-level quote also covers exactly one byte.
    /// Kills the streaming Newline-span `pos + 1`→`pos - 1`/`pos * 1`
    /// mutations in `handle_stream_event`.
    #[test]
    fn streamed_newline_span_covers_one_byte() {
        run!(out, "「a\nb」");
        let nl = out
            .spans
            .iter()
            .find(|s| s.kind == SpanKind::Newline)
            .unwrap_or_else(|| panic!("no Newline span: {:?}", out.spans));
        // 「 is 3 bytes, `a` 1 byte → the `\n` sits at byte 4.
        assert_eq!(nl.source_span, Span::new(4, 5));
    }

    /// Nested same-kind quote opens/closes must track depth so the whole
    /// `「A「B」「C」」` streams as ONE plain run. If the nested-open depth
    /// increment is dropped, streaming exits early and re-enters on a later
    /// open, splitting the plain run. Kills the `1193` `PairOpen` guard
    /// (→false / `==`→`!=`).
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

    /// Two SEPARATE top-level quotes (`「A」「B」`) each open a fresh stream,
    /// flushing the plain run at the second open — so there are two plain
    /// spans. If a same-kind close never decrements depth (or the
    /// depth-zero exit is inverted), streaming never ends and the two runs
    /// merge into one. Kills the `1199` `PairClose` guard (→false / `==`→`!=`)
    /// and the `1204` `depth == 0` exit.
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

    /// A Tortoise close inside a quote stream must NOT be treated as the
    /// quote's own close. If the `PairClose` guard is forced true, the `〕`
    /// exits streaming early and the following `「` re-enters, splitting the
    /// run. Normal keeps one plain run. Kills the `1199` `PairClose`
    /// guard-→true mutation.
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

    /// A Tortoise open inside a quote stream must NOT bump the quote depth.
    /// If the `PairOpen` guard is forced true, the `〔` over-increments depth so
    /// the outer `」` no longer exits streaming, and the following top-level
    /// `「C」` is swallowed instead of flushing — merging what should be two
    /// plain runs. Kills the `1193` `PairOpen` guard-→true mutation.
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

    /// An explicit `｜` immediately before a gaiji is held out of the plain
    /// run as its own span so a following ruby could drop it; with no ruby it
    /// is re-emitted as its own Plain `[3, 6)`. Kills the `bar_start < len`
    /// `<`→`>` / `<`→`==` mutations (which lose the bar and merge it into the
    /// preceding run).
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

    /// With NO `｜` before the gaiji there is no held bar, so no zero-length
    /// span is ever emitted. Kills the `bar_start < len`→`==`/`<=` mutations,
    /// which fabricate an empty `[3, 3)` bar span.
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

    /// A `［＃…］` annotation nested inside a ruby reading is recognised as a
    /// `Segment::Directive`, so the reading is a 3-segment run. Kills the
    /// `try_emit_annotation_at` `close_link == MAX`→`!=` and
    /// `close_idx >= end`→`<` bounds mutations (either declines the nested
    /// annotation, collapsing the reading to plain).
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

    /// Implicit ruby takes the trailing SAME-CLASS run of the preceding text
    /// as its base: `お漢字《かんじ》` → base `漢字` (the `お` stays plain).
    /// Kills the `trailing_ruby_base_start -> 0` stub (which would take the
    /// whole `お漢字`) and the run-extension `==`→`!=` guard (which would find
    /// no base and drop the ruby entirely).
    #[test]
    fn implicit_ruby_base_is_trailing_same_class_run() {
        run!(out, "お漢字《かんじ》");
        let Node::Ruby(r) = out.only_aozora() else {
            panic!("expected Ruby, got {:?}", out.only_aozora());
        };
        assert_eq!(out.plain(r.base), Some("漢字"));
        assert_eq!(out.plain(r.reading), Some("かんじ"));
    }

    /// An orphan `※` inside a quote stream, arriving after a ruby has reset
    /// the pending plain run, must be folded to plain so the spans stay
    /// gap-free. If `fold_held_refmark` is a no-op the `※` bytes are dropped,
    /// leaving a tiling gap. Kills the `fold_held_refmark -> ()` stub.
    #[test]
    fn orphan_refmark_in_quote_keeps_tiling() {
        run!(out, "「駄目《だめ》※あ」");
        let mut cursor = 0u32;
        for span in &out.spans {
            assert_eq!(span.source_span.start, cursor, "tiling gap before {span:?}");
            cursor = span.source_span.end;
        }
    }

    /// A gaiji inside a quote (`「※［＃…］」`) must let the Bracket absorb the
    /// held `※` (gaiji shape), so the gaiji span starts at the `※` (byte 3),
    /// not at the `［` (byte 6). If the fold guard's `!` is dropped, the
    /// refmark is folded away before the bracket and the gaiji degrades to a
    /// standalone `#122` form. Kills the `fold_held_refmark` `delete !`.
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

    /// A top-level `※［＃…］` gaiji is a SINGLE span covering the `※`. If the
    /// top-level `RefMark` hold-arm is disabled, the `※` folds to plain first
    /// and the bracket degrades to a standalone gaiji, yielding two spans.
    /// Kills the `1030` `!replay` guard (→false / `delete !`).
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

    /// A `※` replayed out of a declined bracket body must NOT be held across
    /// to the next real bracket (which would wrongly absorb it as a gaiji
    /// refmark). In `［※］［＃…］` the second bracket is a standalone gaiji
    /// starting at byte 9; if the replayed `※` is held (guard forced true) it
    /// is absorbed and the gaiji wrongly starts at byte 3. Kills the `1030`
    /// `!replay` guard-→true mutation.
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
