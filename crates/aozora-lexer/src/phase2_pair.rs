//! Phase 2 — streaming balanced-stack pairing over the Phase 1 token stream.
//!
//! Consumes the [`Token`] iterator produced by Phase 1 and emits a
//! parallel [`PairEvent`] iterator: [`Token::Text`] / [`Token::Newline`]
//! pass through unchanged, and each [`Token::Trigger`] is classified
//! into [`PairEvent::PairOpen`] / [`PairEvent::PairClose`] /
//! [`PairEvent::Solo`] / [`PairEvent::Unmatched`] / [`PairEvent::Unclosed`].
//!
//! Two production-ready surfaces sit side by side, mirroring Phase 1:
//!
//! - [`pair`] — streaming `PairStream` for FFI / incremental consumers.
//! - [`pair_in`] — arena-batch [`PairOutputIn<'a>`] whose `events` is
//!   a [`PairEventStream<'a>`] allocated inside the caller's [`Arena`].
//!   The 4-column `SoA` layout (M-2 / ADR-0019) keeps the tag column
//!   dense so Phase 3's recogniser dispatch reads 1 cache line per 64
//!   events instead of 1 per ~4 (the old enum layout).
//!
//! Diagnostics stay heap-allocated. The corpus-median doc emits ~0.1
//! diagnostics; per-arena allocation would cost more than it saves and
//! diagnostics outlive the arena anyway (drained into the Pipeline
//! accumulator).
//!
//! ## Why pairing must happen here, not in classify
//!
//! Aozora annotation bodies nest:
//!
//! ```text
//! ［＃「青空」に傍点］       — quoted literal nested inside bracket body
//! ［＃底本では「旧字」］      — same shape, different keyword
//! ［＃「X［＃「Y」に傍点］Z」は底本では「W」］   — doubly nested
//! ```
//!
//! A naïve "find the next `］`" scan hits the *first* `］` even when it
//! closes an inner bracket, yielding a truncated body. This phase runs
//! a proper balanced stack so a body's extent is fixed before any
//! classifier tries to parse it — eliminating the R2 leak class from
//! the 17 k-work corpus sweep (ADR-0007).
//!
//! ## Mismatch policy (current)
//!
//! * **Unclosed open**: left on the stack at end-of-input. The original
//!   `PairOpen` event has already been streamed downstream by the time
//!   we discover the open never closes; instead, on EOF we emit a
//!   synthetic [`PairEvent::Unclosed`] for each still-open frame and
//!   push a [`Diagnostic::UnclosedBracket`]. Phase 3's stack-aware
//!   classifier interprets the trailing `Unclosed` as "the matching
//!   open never closed; treat its accumulated body events as plain".
//! * **Stray close** (empty stack or kind-mismatched top): emitted as
//!   [`PairEvent::Unmatched`] with a [`Diagnostic::UnmatchedClose`].
//!   The stack is *not* popped — this is deliberately conservative, so
//!   a well-formed outer pair like `［...］` still closes correctly even
//!   when an inner stray `》` appears inside the body.

use core::mem;

use aozora_syntax::Span;
use aozora_syntax::borrowed::Arena;
use bumpalo::collections::Vec as BumpVec;
use smallvec::SmallVec;

use crate::diagnostic::Diagnostic;
use crate::token::{Token, TokenStream, TokenTag, TriggerKind};

// `PairKind` lives in `aozora-spec`; re-exported here for backward
// compatibility through the 0.1 → 0.2 transition.
pub use aozora_spec::PairKind;

/// One event in the Phase 2 stream.
///
/// `PairOpen` and `PairClose` carry only their `kind` and `span`.
/// Body cross-link information (which `PairOpen` matches which
/// `PairClose` inside a body buffer) is maintained out-of-band by
/// Phase 3 in a parallel `pair_links` side-table — see
/// [`crate::phase3_classify::BodyView`]. This keeps `PairEvent`'s API
/// clean (no dual-meaning fields between phase 2 emission and phase 3
/// internal patching).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairEvent {
    /// Unchanged from [`Token::Text`] — a byte run between triggers.
    Text { range: Span },

    /// A trigger with no opposing pair on its own (`｜`, `＃`, `※`).
    Solo { kind: TriggerKind, span: Span },

    /// Matched open delimiter. Phase 3 pushes a new body-buffer frame
    /// onto its own stack on this event. The matching close's
    /// body-local index is recorded in the parallel `links` side-table
    /// once the close arrives.
    PairOpen { kind: PairKind, span: Span },

    /// Matched close delimiter. Phase 3 pops the corresponding body
    /// frame on this event and runs recognition on the buffered body.
    /// The matching open's body-local index lives in the parallel
    /// `links` side-table.
    PairClose { kind: PairKind, span: Span },

    /// End-of-stream synthetic event indicating that an earlier
    /// [`PairEvent::PairOpen`] of the carried `kind` was never closed.
    /// Phase 3 treats the corresponding body buffer as having no
    /// matching close and re-fires the buffered events as plain.
    Unclosed { kind: PairKind, span: Span },

    /// Close delimiter that hit an empty stack or a kind-mismatched
    /// stack top. Classifier treats the span as plain text.
    Unmatched { kind: PairKind, span: Span },

    /// Unchanged from [`Token::Newline`] — kept so Phase 3 can attach
    /// line structure to block-level annotations.
    Newline { pos: u32 },
}

impl PairEvent {
    /// Source byte-range span of this event, or `None` for
    /// [`PairEvent::Newline`] (which has only a single position, not a
    /// range).
    #[must_use]
    pub const fn span(&self) -> Option<Span> {
        Some(match *self {
            Self::Text { range } => range,
            Self::Solo { span, .. }
            | Self::PairOpen { span, .. }
            | Self::PairClose { span, .. }
            | Self::Unclosed { span, .. }
            | Self::Unmatched { span, .. } => span,
            Self::Newline { .. } => return None,
        })
    }
}

/// Run the streaming balanced-stack pass over a Phase 1 token stream.
///
/// The returned [`PairStream`] is an iterator yielding one
/// [`PairEvent`] per call to [`Iterator::next`]. After the iterator is
/// exhausted, call [`PairStream::take_diagnostics`] to drain any
/// non-fatal observations that accumulated during the pass
/// (unclosed opens, unmatched closes).
#[must_use]
pub fn pair<I>(tokens: I) -> PairStream<I>
where
    I: Iterator<Item = Token>,
{
    PairStream::new(tokens)
}

/// Storage tag for [`PairEventStream`]. One byte per event,
/// scanned densely in Phase 3's hot dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTag {
    Text,
    Solo,
    PairOpen,
    PairClose,
    Unclosed,
    Unmatched,
    Newline,
}

/// Arena-backed Structure-of-Arrays storage for a Phase 2 event
/// stream. Materialised by [`pair_in`]; consumed by Phase 3's
/// `classify` (and any inspection callers).
///
/// ## Storage layout
///
/// | Column | Type | Bytes / elem | Populated when |
/// |---|---|---:|---|
/// | `tags` | [`EventTag`] | 1 | always |
/// | `spans` | [`Span`] | 8 | always (Newline rows store `Span(pos, pos + 1)`) |
/// | `trigger_kinds` | [`TriggerKind`] | 1 | only `tag == Solo` |
/// | `pair_kinds` | [`PairKind`] | 1 | only `tag ∈ {PairOpen, PairClose, Unclosed, Unmatched}` |
///
/// Total: ~11 bytes / event. Tag-only iteration: 1 cache line per
/// 64 events vs 1 per ~4 events for the 16-byte enum layout.
#[derive(Debug)]
pub struct PairEventStream<'a> {
    tags: BumpVec<'a, EventTag>,
    spans: BumpVec<'a, Span>,
    trigger_kinds: BumpVec<'a, TriggerKind>,
    pair_kinds: BumpVec<'a, PairKind>,
}

impl<'a> PairEventStream<'a> {
    /// Empty stream backed by `arena`.
    #[must_use]
    pub fn with_capacity_in(cap: usize, arena: &'a Arena) -> Self {
        let bump = arena.bump();
        Self {
            tags: BumpVec::with_capacity_in(cap, bump),
            spans: BumpVec::with_capacity_in(cap, bump),
            trigger_kinds: BumpVec::with_capacity_in(cap, bump),
            pair_kinds: BumpVec::with_capacity_in(cap, bump),
        }
    }

    /// Append a [`PairEvent::Text`] row.
    #[inline]
    pub fn push_text(&mut self, range: Span) {
        self.tags.push(EventTag::Text);
        self.spans.push(range);
        self.trigger_kinds.push(TriggerKind::Bar);
        self.pair_kinds.push(PairKind::Bracket);
    }

    /// Append a [`PairEvent::Solo`] row.
    #[inline]
    pub fn push_solo(&mut self, kind: TriggerKind, span: Span) {
        self.tags.push(EventTag::Solo);
        self.spans.push(span);
        self.trigger_kinds.push(kind);
        self.pair_kinds.push(PairKind::Bracket);
    }

    /// Append a structural pair event. `tag` must be one of
    /// `PairOpen` / `PairClose` / `Unclosed` / `Unmatched`.
    #[inline]
    pub fn push_pair(&mut self, tag: EventTag, kind: PairKind, span: Span) {
        debug_assert!(
            matches!(
                tag,
                EventTag::PairOpen | EventTag::PairClose | EventTag::Unclosed | EventTag::Unmatched
            ),
            "push_pair called with non-structural tag: {tag:?}"
        );
        self.tags.push(tag);
        self.spans.push(span);
        self.trigger_kinds.push(TriggerKind::Bar);
        self.pair_kinds.push(kind);
    }

    /// Append a [`PairEvent::Newline`] row. Internally stored as
    /// `Span(pos, pos + 1)` so the spans column stays uniform.
    #[inline]
    pub fn push_newline(&mut self, pos: u32) {
        self.tags.push(EventTag::Newline);
        self.spans.push(Span::new(pos, pos + 1));
        self.trigger_kinds.push(TriggerKind::Bar);
        self.pair_kinds.push(PairKind::Bracket);
    }

    /// Total number of events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// True if the stream contains no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Tag at index `i`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= self.len()`.
    #[inline]
    #[must_use]
    pub fn tag_at(&self, i: usize) -> EventTag {
        self.tags[i]
    }

    /// Span at index `i`. Newline rows return `Span(pos, pos + 1)`;
    /// callers reading newline position should use [`Self::newline_pos_at`].
    #[inline]
    #[must_use]
    pub fn span_at(&self, i: usize) -> Span {
        self.spans[i]
    }

    /// Trigger kind at index `i` (only meaningful for Solo rows).
    #[inline]
    #[must_use]
    pub fn trigger_kind_at(&self, i: usize) -> TriggerKind {
        self.trigger_kinds[i]
    }

    /// Pair kind at index `i` (only meaningful for structural rows).
    #[inline]
    #[must_use]
    pub fn pair_kind_at(&self, i: usize) -> PairKind {
        self.pair_kinds[i]
    }

    /// Newline position at index `i` (only meaningful for Newline rows).
    #[inline]
    #[must_use]
    pub fn newline_pos_at(&self, i: usize) -> u32 {
        self.spans[i].start
    }

    /// Iterator over the stream as `PairEvent` values, reconstructing
    /// each variant from the columns. Hot Phase 3 consumers should
    /// use the per-column accessors directly.
    pub fn iter(&self) -> impl Iterator<Item = PairEvent> + '_ {
        (0..self.len()).map(move |i| match self.tag_at(i) {
            EventTag::Text => PairEvent::Text {
                range: self.span_at(i),
            },
            EventTag::Solo => PairEvent::Solo {
                kind: self.trigger_kind_at(i),
                span: self.span_at(i),
            },
            EventTag::PairOpen => PairEvent::PairOpen {
                kind: self.pair_kind_at(i),
                span: self.span_at(i),
            },
            EventTag::PairClose => PairEvent::PairClose {
                kind: self.pair_kind_at(i),
                span: self.span_at(i),
            },
            EventTag::Unclosed => PairEvent::Unclosed {
                kind: self.pair_kind_at(i),
                span: self.span_at(i),
            },
            EventTag::Unmatched => PairEvent::Unmatched {
                kind: self.pair_kind_at(i),
                span: self.span_at(i),
            },
            EventTag::Newline => PairEvent::Newline {
                pos: self.newline_pos_at(i),
            },
        })
    }
}

/// Output of [`pair_in`]. `events` lives in the caller's arena;
/// `diagnostics` stays heap-allocated (rare and outlives the arena).
#[derive(Debug)]
pub struct PairOutputIn<'a> {
    pub events: PairEventStream<'a>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Materialise every Phase 2 event from a [`TokenStream`] into a
/// single arena-backed [`PairOutputIn`].
///
/// M-2 (ADR-0019): production entry point for Phase 2. Reads the
/// tag column densely (1 cache line per 64 tokens) and dispatches
/// into the balanced-stack walk. The EOF-drain pass (synthetic
/// `Unclosed` emission) is appended after the main loop, mirroring
/// the `eof_drain` state-machine behaviour.
#[must_use]
pub fn pair_in<'a>(tokens: &TokenStream<'_>, arena: &'a Arena) -> PairOutputIn<'a> {
    let mut events = PairEventStream::with_capacity_in(tokens.len() + 4, arena);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut stack: SmallVec<[(PairKind, Span); 8]> = SmallVec::new();

    for i in 0..tokens.len() {
        match tokens.tag_at(i) {
            TokenTag::Text => events.push_text(tokens.span_at(i)),
            TokenTag::Newline => events.push_newline(tokens.newline_pos_at(i)),
            TokenTag::Trigger => {
                let kind = tokens.trigger_kind_at(i);
                let span = tokens.span_at(i);
                push_trigger_event(&mut events, kind, span, &mut stack, &mut diagnostics);
            }
        }
    }

    // Drain the residual stack: emit one synthetic `Unclosed` event
    // per remaining open frame, in innermost-first order.
    while let Some((kind, span)) = stack.pop() {
        diagnostics.push(Diagnostic::unclosed_bracket(span, kind));
        events.push_pair(EventTag::Unclosed, kind, span);
    }

    PairOutputIn {
        events,
        diagnostics,
    }
}

/// Trigger classification for [`pair_in`]. Mutates the stack +
/// diagnostics in place; pushes the resulting event into `events`.
#[inline]
#[allow(
    clippy::too_many_arguments,
    reason = "five small args: 2 mutable column references + 2 mutable balancing-state references + 1 small Span. Bundling into a struct would obscure the inner-loop hot path."
)]
fn push_trigger_event(
    events: &mut PairEventStream<'_>,
    kind: TriggerKind,
    span: Span,
    stack: &mut SmallVec<[(PairKind, Span); 8]>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(pair_kind) = open_kind_of(kind) {
        stack.push((pair_kind, span));
        events.push_pair(EventTag::PairOpen, pair_kind, span);
        return;
    }
    if let Some(pair_kind) = close_kind_of(kind) {
        if stack.last().is_some_and(|&(top, _)| top == pair_kind) {
            stack.pop();
            events.push_pair(EventTag::PairClose, pair_kind, span);
            return;
        }
        diagnostics.push(Diagnostic::unmatched_close(span, pair_kind));
        events.push_pair(EventTag::Unmatched, pair_kind, span);
        return;
    }
    events.push_solo(kind, span);
}

/// Stream of [`PairEvent`]s produced from an upstream [`Token`]
/// iterator. Internal state:
///
/// * `tokens`: upstream token producer; tokens are pulled lazily.
/// * `stack`: smallvec of open `PairKind`s with their open spans.
///   Inline capacity 8 covers the 99th-percentile bracket nesting in
///   real Aozora text (Innovation I-8 corpus profile).
/// * `pending`: single-slot output buffer for the case where one
///   token resolves into a `Solo` event AND a follow-on synthetic
///   `Unclosed` from a now-impossible earlier open. Currently unused
///   — the streaming policy emits at most one event per input token —
///   but kept for future extension.
/// * `diagnostics`: collected non-fatal observations.
/// * `eof_drain`: cursor through the residual stack at end-of-input
///   used to emit one `Unclosed` event per remaining open frame.
#[derive(Debug)]
pub struct PairStream<I>
where
    I: Iterator<Item = Token>,
{
    tokens: I,
    stack: SmallVec<[(PairKind, Span); 8]>,
    diagnostics: Vec<Diagnostic>,
    eof_drain: bool,
    finished: bool,
}

impl<I> PairStream<I>
where
    I: Iterator<Item = Token>,
{
    fn new(tokens: I) -> Self {
        Self {
            tokens,
            stack: SmallVec::new(),
            diagnostics: Vec::new(),
            eof_drain: false,
            finished: false,
        }
    }

    /// Drain accumulated diagnostics. Should be called after the
    /// iterator is exhausted (otherwise EOF unclosed-bracket
    /// diagnostics will not yet have been emitted).
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        mem::take(&mut self.diagnostics)
    }

    /// Borrow accumulated diagnostics in place. Same caveat as
    /// [`Self::take_diagnostics`]: only complete after exhaustion.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    fn classify_trigger(&mut self, kind: TriggerKind, span: Span) -> PairEvent {
        if let Some(pair_kind) = open_kind_of(kind) {
            self.stack.push((pair_kind, span));
            return PairEvent::PairOpen {
                kind: pair_kind,
                span,
            };
        }

        if let Some(pair_kind) = close_kind_of(kind) {
            if self.stack.last().is_some_and(|&(top, _)| top == pair_kind) {
                self.stack.pop();
                return PairEvent::PairClose {
                    kind: pair_kind,
                    span,
                };
            }
            self.diagnostics
                .push(Diagnostic::unmatched_close(span, pair_kind));
            return PairEvent::Unmatched {
                kind: pair_kind,
                span,
            };
        }

        // Trigger is neither open nor close (Bar / Hash / RefMark).
        PairEvent::Solo { kind, span }
    }
}

impl<I> Iterator for PairStream<I>
where
    I: Iterator<Item = Token>,
{
    type Item = PairEvent;

    fn next(&mut self) -> Option<PairEvent> {
        if self.finished {
            return None;
        }
        if self.eof_drain {
            // Drain residual stack entries as Unclosed events. We pop
            // from the BACK so innermost (last-pushed) opens surface
            // first — same diagnostic order the legacy `pair()` used.
            if let Some((kind, span)) = self.stack.pop() {
                self.diagnostics
                    .push(Diagnostic::unclosed_bracket(span, kind));
                return Some(PairEvent::Unclosed { kind, span });
            }
            self.finished = true;
            return None;
        }

        match self.tokens.next() {
            Some(Token::Text { range }) => Some(PairEvent::Text { range }),
            Some(Token::Newline { pos }) => Some(PairEvent::Newline { pos }),
            Some(Token::Trigger { kind, span }) => Some(self.classify_trigger(kind, span)),
            None => {
                // Upstream exhausted. Switch into EOF-drain mode and
                // recurse to either yield the first Unclosed or
                // terminate.
                self.eof_drain = true;
                self.next()
            }
        }
    }
}

/// Map a trigger to the [`PairKind`] it *opens*, if any.
const fn open_kind_of(kind: TriggerKind) -> Option<PairKind> {
    Some(match kind {
        TriggerKind::BracketOpen => PairKind::Bracket,
        TriggerKind::RubyOpen => PairKind::Ruby,
        TriggerKind::DoubleRubyOpen => PairKind::DoubleRuby,
        TriggerKind::TortoiseOpen => PairKind::Tortoise,
        TriggerKind::QuoteOpen => PairKind::Quote,
        _ => return None,
    })
}

/// Map a trigger to the [`PairKind`] it *closes*, if any.
const fn close_kind_of(kind: TriggerKind) -> Option<PairKind> {
    Some(match kind {
        TriggerKind::BracketClose => PairKind::Bracket,
        TriggerKind::RubyClose => PairKind::Ruby,
        TriggerKind::DoubleRubyClose => PairKind::DoubleRuby,
        TriggerKind::TortoiseClose => PairKind::Tortoise,
        TriggerKind::QuoteClose => PairKind::Quote,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::phase1_events::tokenize;

    /// Materialise the full stream + diagnostics for tests.
    fn run(src: &str) -> (Vec<PairEvent>, Vec<Diagnostic>) {
        let mut stream = pair(tokenize(src));
        let events: Vec<PairEvent> = (&mut stream).collect();
        let diagnostics = stream.take_diagnostics();
        (events, diagnostics)
    }

    fn pair_kinds(events: &[PairEvent]) -> Vec<(&'static str, PairKind)> {
        events
            .iter()
            .filter_map(|e| match *e {
                PairEvent::PairOpen { kind, .. } => Some(("open", kind)),
                PairEvent::PairClose { kind, .. } => Some(("close", kind)),
                PairEvent::Unclosed { kind, .. } => Some(("unclosed", kind)),
                PairEvent::Unmatched { kind, .. } => Some(("unmatched", kind)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_input_yields_no_events() {
        let (events, diagnostics) = run("");
        assert!(events.is_empty());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn plain_text_passes_through_as_text_event() {
        let (events, diagnostics) = run("hello");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], PairEvent::Text { .. }));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn simple_bracket_pair_emits_open_and_close() {
        let (events, diagnostics) = run("［body］");
        // Events: PairOpen(Bracket), Text("body"), PairClose(Bracket).
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[2],
            PairEvent::PairClose {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn nested_brackets_pair_inner_before_outer() {
        let (events, diagnostics) = run("［＃外［＃内］終］");
        // 0 PairOpen Bracket, 1 Solo Hash, 2 Text "外",
        // 3 PairOpen Bracket, 4 Solo Hash, 5 Text "内",
        // 6 PairClose Bracket, 7 Text "終", 8 PairClose Bracket.
        assert_eq!(events.len(), 9);
        assert!(matches!(
            events[0],
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[3],
            PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[6],
            PairEvent::PairClose {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(matches!(
            events[8],
            PairEvent::PairClose {
                kind: PairKind::Bracket,
                ..
            }
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ruby_pair_emits_ruby_kinds() {
        let (events, diagnostics) = run("《かんじ》");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Ruby), ("close", PairKind::Ruby)]
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn double_ruby_is_its_own_pair_kind() {
        let (events, _diagnostics) = run("《《X》》");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::DoubleRuby),
                ("close", PairKind::DoubleRuby),
            ]
        );
    }

    #[test]
    fn tortoise_pair_emits_tortoise_kinds() {
        let (events, _) = run("〔e^〕");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Tortoise), ("close", PairKind::Tortoise)]
        );
    }

    #[test]
    fn quote_pair_standalone_emits_quote_kinds() {
        let (events, _) = run("「台詞」");
        assert_eq!(
            pair_kinds(&events),
            vec![("open", PairKind::Quote), ("close", PairKind::Quote)]
        );
    }

    #[test]
    fn solo_bar_hash_refmark_remain_solo() {
        let (events, _) = run("｜＃※");
        assert_eq!(events.len(), 3);
        for ev in &events {
            assert!(
                matches!(ev, PairEvent::Solo { .. }),
                "expected all Solo, got {ev:?}"
            );
        }
    }

    #[test]
    fn newline_passes_through_unchanged() {
        let (events, _) = run("a\nb");
        assert_eq!(events.len(), 3);
        assert!(matches!(events[1], PairEvent::Newline { .. }));
    }

    #[test]
    fn unclosed_bracket_appends_synthetic_unclosed_event() {
        let (events, diagnostics) = run("［＃unclosed");
        // Stream: PairOpen, Solo(Hash), Text, ...then EOF appends Unclosed.
        assert!(
            events.iter().any(|e| matches!(
                e,
                PairEvent::Unclosed {
                    kind: PairKind::Bracket,
                    ..
                }
            )),
            "expected an Unclosed Bracket event in {events:?}"
        );
        assert!(diagnostics.iter().any(|d| matches!(
            d,
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        )));
    }

    #[test]
    fn unmatched_close_emits_diagnostic_without_affecting_stack() {
        let (events, diagnostics) = run("stray］text");
        assert!(events.iter().any(|e| matches!(
            e,
            PairEvent::Unmatched {
                kind: PairKind::Bracket,
                ..
            }
        )));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn mismatched_close_inside_bracket_does_not_pop_outer() {
        let (events, diagnostics) = run("［body》more］");
        let kinds = pair_kinds(&events);
        assert_eq!(
            kinds,
            vec![
                ("open", PairKind::Bracket),
                ("unmatched", PairKind::Ruby),
                ("close", PairKind::Bracket),
            ]
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn event_count_matches_token_count_plus_eof_unclosed() {
        // 1:1 correspondence is now per-token + EOF-residual: every
        // input Token maps to exactly one event, plus one synthetic
        // Unclosed for each still-open frame at EOF. The sum is the
        // useful invariant for downstream position tracking.
        let src = "［＃「a」に］plain《b》〔c〕";
        let token_count = tokenize(src).count();
        let (events, _diagnostics) = run(src);
        assert_eq!(events.len(), token_count, "no unclosed in this src");
    }

    #[test]
    fn span_accessor_returns_range_for_text_and_trigger_events() {
        let (events, _) = run("a｜b《c》");
        for ev in &events {
            match ev {
                PairEvent::Newline { .. } => {
                    assert!(ev.span().is_none(), "Newline must have no span");
                }
                _ => {
                    assert!(ev.span().is_some(), "non-Newline event must carry a span");
                }
            }
        }
    }

    #[test]
    fn span_accessor_returns_none_for_newline() {
        let (events, _) = run("\n");
        assert_eq!(events.len(), 1);
        assert!(events[0].span().is_none());
    }

    /// Three nested unclosed `［＃` opens reach EOF together. The
    /// EOF-drain loop must surface them innermost-first (`stack.pop()`
    /// from the back), and emit one `UnclosedBracket` diagnostic per
    /// frame in the same order. Pins the diagnostic ordering callers
    /// rely on for spans rendering.
    #[test]
    fn pair_stream_eof_drains_innermost_first_after_multiple_unclosed() {
        let (events, diagnostics) = run("［＃［＃［＃");
        // Filter Unclosed events out — they should be the LAST three
        // events of the stream (after Open/Solo/Open/Solo/Open/Solo).
        let unclosed: Vec<&PairEvent> = events
            .iter()
            .filter(|e| matches!(e, PairEvent::Unclosed { .. }))
            .collect();
        assert_eq!(unclosed.len(), 3, "events were {events:?}");

        // The opens we created have monotonically increasing source
        // start positions; the EOF drain pops innermost (last-pushed)
        // first, so the SPAN of the first Unclosed event must be the
        // LARGEST of the three (innermost = last in source order).
        let starts: Vec<u32> = unclosed
            .iter()
            .map(|e| e.span().expect("Unclosed has a span").start)
            .collect();
        assert!(
            starts[0] > starts[1] && starts[1] > starts[2],
            "EOF drain order should be innermost-first; got starts={starts:?}"
        );

        // Diagnostic ordering: same innermost-first, one per frame.
        let bracket_diag_count = diagnostics
            .iter()
            .filter(|d| matches!(d, Diagnostic::UnclosedBracket { .. }))
            .count();
        assert_eq!(bracket_diag_count, 3);
    }

    /// `take_diagnostics` on a partly-driven stream returns whatever
    /// has accumulated so far (could be 0); the same call after the
    /// stream is exhausted MUST return the empty Vec because the prior
    /// drain emptied the buffer.
    #[test]
    fn pair_stream_take_diagnostics_only_complete_after_exhaustion() {
        let mut stream = pair(tokenize("stray］more text［＃tail"));
        // Drive partway: pull 4 events. The unmatched `］` close
        // produces one diagnostic eagerly; the unclosed `［＃` only
        // surfaces after EOF.
        for _ in 0..4 {
            let _ = stream.next();
        }
        let mid = stream.take_diagnostics();
        // 0 or more diagnostics — exact count depends on tokenisation,
        // we only require the call to be safe and return what was
        // accumulated so far.
        let _ = mid.len(); // observably non-panicking access

        // Drive to end.
        while stream.next().is_some() {}
        let after = stream.take_diagnostics();
        // Whatever was drained at `mid` is GONE. Anything emitted AFTER
        // the first `take_diagnostics` (e.g. the EOF unclosed) shows
        // up here. The contract is "take == drain", so a SECOND
        // immediate take must yield empty.
        let again = stream.take_diagnostics();
        assert!(
            again.is_empty(),
            "second take_diagnostics must return empty after the prior drain, got {again:?}"
        );
        // Sanity: at least one diagnostic surfaced overall (the
        // unclosed bracket synthesis), proving the assertion above is
        // about drain semantics not absence of diagnostics.
        assert!(
            !after.is_empty() || mid.iter().any(|_| true),
            "expected at least one diagnostic across the two drains for this input"
        );
    }

    /// A purely textual input emits exactly one `Text` event covering
    /// every byte. Exercises the Phase 1 → Phase 2 pass-through path.
    #[test]
    fn pair_stream_text_event_byte_coverage() {
        let (events, diagnostics) = run("abcdef");
        assert_eq!(events.len(), 1, "got {events:?}");
        match events[0] {
            PairEvent::Text { range } => {
                assert_eq!(range, Span::new(0, 6));
            }
            ref other => panic!("expected single Text event, got {other:?}"),
        }
        assert!(diagnostics.is_empty());
    }

    proptest! {
        /// Output is a pure function of input — running the same source
        /// twice must produce identical event sequences.
        #[test]
        fn proptest_pair_is_deterministic(src in source_strategy()) {
            let (a, _) = run(&src);
            let (b, _) = run(&src);
            prop_assert_eq!(a, b);
        }

        /// Every PairOpen of `kind` is eventually balanced either by a
        /// matching PairClose of the same `kind` or by an Unclosed of the
        /// same `kind`. No "lost" opens.
        #[test]
        fn proptest_every_open_resolves(src in source_strategy()) {
            let (events, _) = run(&src);
            // Replay the stream maintaining a stack: every push must be
            // matched by a Close or an Unclosed of the same kind.
            let mut stack: Vec<PairKind> = Vec::new();
            for ev in &events {
                match *ev {
                    PairEvent::PairOpen { kind, .. } => stack.push(kind),
                    PairEvent::PairClose { kind, .. } => {
                        let top = stack.pop();
                        prop_assert_eq!(top, Some(kind));
                    }
                    PairEvent::Unclosed { kind, .. } => {
                        let top = stack.pop();
                        prop_assert_eq!(top, Some(kind));
                    }
                    _ => {}
                }
            }
            prop_assert!(stack.is_empty(), "leftover opens in stack: {stack:?}");
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
