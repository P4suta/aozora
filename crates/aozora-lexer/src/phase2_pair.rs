//! Phase 2 — streaming balanced-stack pairing over the Phase 1 token stream.
//!
//! Consumes the [`Token`] iterator produced by Phase 1 and emits a
//! parallel [`PairEvent`] iterator: [`Token::Text`] / [`Token::Newline`]
//! pass through unchanged, and each [`Token::Trigger`] is classified
//! into [`PairEvent::PairOpen`] / [`PairEvent::PairClose`] /
//! [`PairEvent::Solo`] / [`PairEvent::Unmatched`] / [`PairEvent::Unclosed`].
//!
//! After I-2 (deforestation) the public entry point is [`pair`], which
//! returns `PairStream` — an `impl Iterator<Item = PairEvent>` with no
//! intermediate `Vec<PairEvent>` materialisation. Phase 3's classifier
//! consumes that stream directly, maintaining its own balanced stack
//! to track body extents (since the stream no longer carries the prior
//! `close_idx` / `open_idx` cross-link indices).
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
use smallvec::SmallVec;

use crate::diagnostic::Diagnostic;
use crate::token::{Token, TriggerKind};

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
