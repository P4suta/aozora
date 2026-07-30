#![expect(
    clippy::expect_used,
    reason = "stack shape is checked before the matching pair is removed"
)]

//! Pair stage — streaming balanced-stack pairing over the tokenize-stage token stream.
//!
//! Consumes the [`Token`] iterator produced by the tokenize stage and emits a
//! parallel [`PairEvent`] iterator: [`Token::Text`] / [`Token::Newline`]
//! pass through unchanged, and each [`Token::Trigger`] is classified
//! into [`PairEvent::PairOpen`] / [`PairEvent::PairClose`] /
//! [`PairEvent::Solo`] / [`PairEvent::Unmatched`] / [`PairEvent::Unclosed`].
//!
//! Two production-ready surfaces sit side by side, mirroring the tokenize stage:
//!
//! - [`pair`] — streaming `PairStream` for FFI / incremental consumers.
//! - `pair_in` — arena-batch `PairOutputIn<'a>` whose `events` is
//!   a `BumpVec<'a, PairEvent>` allocated inside the caller's `Arena`.
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
//! closes an inner bracket, yielding a truncated body. This stage runs
//! a proper balanced stack so a body's extent is fixed before any
//! classifier tries to parse it.
//!
//! ## Mismatch policy (current)
//!
//! * **Unclosed open**: left on the stack at end-of-input. The original
//!   `PairOpen` event has already been streamed downstream by the time
//!   we discover the open never closes; instead, on EOF we emit a
//!   synthetic [`PairEvent::Unclosed`] for each still-open frame and
//!   push a [`Diagnostic::UnclosedBracket`]. The classify stage's stack-aware
//!   classifier interprets the trailing `Unclosed` as "the matching
//!   open never closed; treat its accumulated body events as plain".
//! * **Stray close** (empty stack or kind-mismatched top): emitted as
//!   [`PairEvent::Unmatched`] with a [`Diagnostic::UnmatchedClose`].
//!   The stack is *not* popped — this is deliberately conservative, so
//!   a well-formed outer pair like `［...］` still closes correctly even
//!   when an inner stray `》` appears inside the body.
//! * **Bracket is a hard pairing scope** (refines the stray-close rule
//!   for `］` only): a `］` closes the *nearest enclosing* `［`, even when
//!   non-bracket opens are stacked above it, force-resolving those opens
//!   as [`PairEvent::Unclosed`] (innermost-first) before the
//!   [`PairEvent::PairClose`]. This keeps an unbalanced `「` inside a
//!   directive body — an image caption `［＃「…（fig）入る］`, a composed-glyph
//!   gaiji `［＃「口＋「皐」…］`, a typo-note quoting literal quotes — from
//!   swallowing the `］` so the bracket never closes and the classifier
//!   sinks the rest of the document to plain. A balanced body never
//!   triggers it (the top *is* the bracket, matched directly). A `」`
//!   still cannot cross a bracket downward — only `］` gets this scope.
//!   See ADR-0025 (`docs/adr/`).
//! * **A stray `［` is line-scoped** (the temporal dual of the hard scope):
//!   a `［＃…］` body never spans a line break, so a `［` still open when a
//!   `Newline` arrives is stray. The pair stage force-resolves the
//!   contiguous top run of such brackets as [`PairEvent::Unclosed`]
//!   (innermost-first) before the newline, so a lone trailing `［`
//!   (江戸川乱歩『影男』L548) no longer keeps its classifier frame open to EOF
//!   and sinks the rest of the document to plain. Ruby / angle-quote resolve
//!   on their own delimiters and dialogue `「」` / kaeriten `〔〕` span lines,
//!   so only `［` gets the line scope. See ADR-0030 (`docs/adr/`).

use core::mem;

use crate::syntax::Span;
use smallvec::SmallVec;

use super::token::{Token, TriggerKind};
use crate::spec::Diagnostic;

// `PairKind` lives in `crate::spec`; re-exported here so the pair pass and
// its consumers can name it locally. `PairLink` is the resolved
// (open, close) view zipped during the pair pass.
pub(crate) use crate::spec::{PairKind, PairLink};

/// One event in the pair-stage stream.
///
/// `PairOpen` and `PairClose` carry only their `kind` and `span`.
/// Body cross-link information (which `PairOpen` matches which
/// `PairClose` inside a body buffer) is maintained out-of-band by
/// the classify stage in a parallel `pair_links` side-table inside the
/// classifier's `BodyView`. This keeps `PairEvent`'s API clean (no
/// dual-meaning fields between pair-stage emission and classify-stage
/// internal patching).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum PairEvent {
    /// Unchanged from [`Token::Text`] — a byte run between triggers.
    Text {
        /// Sanitized-source byte span of the run; may be empty.
        range: Span,
    },

    /// A trigger with no opposing pair on its own (`｜`, `＃`, `※`).
    Solo {
        /// The standalone trigger's role.
        kind: TriggerKind,
        /// Sanitized-source byte span of the trigger.
        span: Span,
    },

    /// Matched open delimiter. The classify stage pushes a new body-buffer
    /// frame onto its own stack on this event. The matching close's
    /// body-local index is recorded in the parallel `links` side-table
    /// once the close arrives.
    PairOpen {
        /// Bracket family this open belongs to.
        kind: PairKind,
        /// Sanitized-source byte span of the opening delimiter.
        span: Span,
    },

    /// Matched close delimiter. The classify stage pops the corresponding
    /// body frame on this event and runs recognition on the buffered body.
    /// The matching open's body-local index lives in the parallel
    /// `links` side-table.
    PairClose {
        /// Bracket family this close belongs to (matches its open).
        kind: PairKind,
        /// Sanitized-source byte span of the closing delimiter.
        span: Span,
    },

    /// End-of-stream synthetic event indicating that an earlier
    /// [`PairEvent::PairOpen`] of the carried `kind` was never closed.
    /// The classify stage treats the corresponding body buffer as having no
    /// matching close and re-fires the buffered events as plain.
    Unclosed {
        /// Bracket family of the open that never closed.
        kind: PairKind,
        /// Sanitized-source byte span of the original (still-open) open
        /// delimiter — *not* an end-of-input position.
        span: Span,
    },

    /// Close delimiter that hit an empty stack or a kind-mismatched
    /// stack top. Classifier treats the span as plain text.
    Unmatched {
        /// Bracket family the stray close would have closed.
        kind: PairKind,
        /// Sanitized-source byte span of the stray close delimiter.
        span: Span,
    },

    /// Unchanged from [`Token::Newline`] — kept so the classify stage can
    /// attach line structure to block-level annotations.
    Newline {
        /// Sanitized-source byte offset of the `\n`.
        pos: u32,
    },
}

impl PairEvent {
    /// Source byte-range span of this event, or `None` for
    /// [`PairEvent::Newline`] (which has only a single position, not a
    /// range).
    #[must_use]
    pub(crate) const fn span(&self) -> Option<Span> {
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

/// Run the streaming balanced-stack pass over a tokenize-stage token stream.
///
/// The returned [`PairStream`] is an iterator yielding one
/// [`PairEvent`] per call to [`Iterator::next`]. After the iterator is
/// exhausted, call [`PairStream::take_diagnostics`] to drain any
/// non-fatal observations that accumulated during the pass
/// (unclosed opens, unmatched closes).
#[must_use]
pub(crate) fn pair<I>(tokens: I) -> PairStream<I>
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
///   real Aozora text (corpus profile).
/// * `diagnostics`: collected non-fatal observations.
/// * `links`: resolved `(open, close)` pairs accumulated as the
///   stack matches; mirrors `PairOutputIn::links` for streaming
///   callers that don't go through `pair_in`.
/// * `pending`: FIFO queue of events a single trigger produced beyond
///   the one it returns directly. A `］` that closes a bracket buried
///   under dangling non-bracket opens yields several events at once
///   (`Unclosed`… then `PairClose`); the head is returned and the tail
///   is buffered here, drained front-first before the next token.
/// * `eof_drain`: cursor through the residual stack at end-of-input
///   used to emit one `Unclosed` event per remaining open frame.
/// * `finished`: terminal flag set after the eof drain completes,
///   so subsequent `next()` calls return `None` without re-walking
///   the stack.
#[derive(Debug)]
pub(crate) struct PairStream<I>
where
    I: Iterator<Item = Token>,
{
    tokens: I,
    stack: SmallVec<[(PairKind, Span); 8]>,
    diagnostics: Vec<Diagnostic>,
    /// Resolved (open, close) pairs collected as the stack matches.
    /// Mirrors the `PairOutputIn::links` side-table for streaming
    /// callers that don't go through `pair_in`.
    links: Vec<PairLink>,
    /// Events queued by the current trigger to surface on later
    /// `next()` calls, drained front-first. Empty on the fast paths.
    pending: SmallVec<[PairEvent; 4]>,
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
            links: Vec::new(),
            pending: SmallVec::new(),
            eof_drain: false,
            finished: false,
        }
    }

    /// Drain accumulated diagnostics. Should be called after the
    /// iterator is exhausted (otherwise EOF unclosed-bracket
    /// diagnostics will not yet have been emitted).
    pub(crate) fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        mem::take(&mut self.diagnostics)
    }

    /// Borrow accumulated diagnostics in place. Same caveat as
    /// [`Self::take_diagnostics`]: only complete after exhaustion.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Drain the resolved [`PairLink`] side-table. Same exhaustion
    /// caveat as [`Self::take_diagnostics`].
    pub(crate) fn take_links(&mut self) -> Vec<PairLink> {
        mem::take(&mut self.links)
    }

    /// Borrow the resolved [`PairLink`] list in place. Same caveat
    /// applies — only complete after exhaustion.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn links(&self) -> &[PairLink] {
        &self.links
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
            if let Some(&(top, open_span)) = self.stack.last()
                && top == pair_kind
            {
                self.stack.pop();
                self.links.push(PairLink::new(pair_kind, open_span, span));
                return PairEvent::PairClose {
                    kind: pair_kind,
                    span,
                };
            }

            // A `］` treats its bracket as a *hard pairing scope*: it closes
            // the nearest enclosing `［`, force-resolving any non-bracket opens
            // stacked above that bracket as `Unclosed` (innermost-first). This
            // is what stops an unbalanced `「` inside a directive body — e.g.
            // `［＃「…（fig）入る］` or the composed-glyph gaiji
            // `［＃「口＋「皐」…］` — from burying the `］` so the bracket never
            // closes and the classifier sinks the rest of the document to plain.
            // The balanced case never reaches here (the fast path above returns).
            if pair_kind == PairKind::Bracket
                && let Some(bracket_pos) = self
                    .stack
                    .iter()
                    .rposition(|&(k, _)| k == PairKind::Bracket)
            {
                // Everything above `bracket_pos` is non-bracket by construction
                // (it is the top-most bracket). Pop those top-first so the
                // innermost dangling open surfaces first, matching the EOF-drain
                // order (`next`), then close the bracket itself.
                while self.stack.len() > bracket_pos + 1 {
                    let (k, open_span) = self.stack.pop().expect("len > bracket_pos + 1");
                    self.diagnostics
                        .push(Diagnostic::unclosed_bracket(open_span, k));
                    self.pending.push(PairEvent::Unclosed {
                        kind: k,
                        span: open_span,
                    });
                }
                let (_, open_span) = self.stack.pop().expect("bracket at bracket_pos");
                self.links
                    .push(PairLink::new(PairKind::Bracket, open_span, span));
                self.pending.push(PairEvent::PairClose {
                    kind: PairKind::Bracket,
                    span,
                });
                // `pending` now holds [Unclosed…(innermost-first), PairClose];
                // surface the head and let `next` drain the rest in order.
                return self.pending.remove(0);
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

    /// Emit a `Newline`, force-resolving any stray `［` bracket that is still
    /// open when the line ends.
    ///
    /// A `［＃…］` directive body never spans a line break: a real directive
    /// closes its `］` on the same line, so a `［` still on the stack at a
    /// newline is *stray* — an unmatched open with no partner (江戸川乱歩
    /// 『影男』L548's trailing `［` is the corpus archetype). Left on the stack
    /// the bracket's classifier frame buffers the *entire rest of the document*
    /// and, never closing, sinks it to plain at EOF, so every following ruby,
    /// heading and directive renders as literal source — the F21 leak cascade.
    ///
    /// Force-resolve the contiguous top run of stray brackets as `Unclosed`
    /// (innermost-first, mirroring the EOF drain and the ADR-0025 hard-scope
    /// unwind order) *before* the newline, so the pair events after the line
    /// break pair at a clean scope and classify normally on the live stream.
    /// Ruby / angle-quote resolve on their own delimiters and dialogue `「」` /
    /// kaeriten `〔〕` legitimately span lines, so only `［` gets this line
    /// scope — the dual of ADR-0025's `］` hard scope.
    fn newline_event(&mut self, pos: u32) -> PairEvent {
        while self
            .stack
            .last()
            .is_some_and(|&(kind, _)| kind == PairKind::Bracket)
        {
            let (kind, open_span) = self.stack.pop().expect("checked last is a Bracket");
            self.diagnostics
                .push(Diagnostic::unclosed_bracket(open_span, kind));
            self.pending.push(PairEvent::Unclosed {
                kind,
                span: open_span,
            });
        }
        if self.pending.is_empty() {
            return PairEvent::Newline { pos };
        }
        // Surface the unwound `Unclosed` events first, then the newline; the
        // head is returned now and `next` drains the tail in order.
        self.pending.push(PairEvent::Newline { pos });
        self.pending.remove(0)
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
        // A single `］` may have produced several events (dangling-open
        // unwind + the bracket close); surface the buffered tail before
        // pulling the next token so ordering is preserved.
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
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
            Some(Token::Newline { pos }) => Some(self.newline_event(pos)),
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
        TriggerKind::AngleQuoteOpen => PairKind::AngleQuote,
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
        TriggerKind::AngleQuoteClose => PairKind::AngleQuote,
        TriggerKind::TortoiseClose => PairKind::Tortoise,
        TriggerKind::QuoteClose => PairKind::Quote,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::pipeline::lexer::tokenize::tokenize;

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
    fn angle_quote_is_its_own_pair_kind() {
        let (events, _diagnostics) = run("≪X≫");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::AngleQuote),
                ("close", PairKind::AngleQuote),
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

    /// A `［＃…］` directive body never spans a line break, so a `［` still
    /// open when the newline arrives is stray (F21). The pair stage
    /// force-resolves it as `Unclosed` *before* the `Newline`, so the events
    /// after the line break pair at a clean scope and the classifier does not
    /// sink the rest of the document. Pins 江戸川乱歩『影男』L548's shape:
    /// a trailing `［` followed by a line whose ruby must survive.
    #[test]
    fn stray_bracket_resolves_at_line_break() {
        let (events, diagnostics) = run("［\n《か》");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::Bracket),
                ("unclosed", PairKind::Bracket),
                ("open", PairKind::Ruby),
                ("close", PairKind::Ruby),
            ]
        );
        // The Unclosed sits between the open and the newline.
        let idx = |pred: fn(&PairEvent) -> bool| events.iter().position(pred).unwrap();
        let open = idx(|e| {
            matches!(
                e,
                PairEvent::PairOpen {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        });
        let unclosed = idx(|e| {
            matches!(
                e,
                PairEvent::Unclosed {
                    kind: PairKind::Bracket,
                    ..
                }
            )
        });
        let newline = idx(|e| matches!(e, PairEvent::Newline { .. }));
        assert!(
            open < unclosed && unclosed < newline,
            "Unclosed must sit between the open and the newline: {events:?}"
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(matches!(
            diagnostics[0],
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        ));
    }

    /// After the stray `［` is line-scoped, a later `］` no longer closes it —
    /// it is a stray `Unmatched`, so the bracket cannot capture the
    /// intervening document (the F21 cascade). The dual of the ADR-0025 `］`
    /// hard scope.
    #[test]
    fn line_scoped_bracket_does_not_capture_later_close() {
        let (events, _) = run("［\n本文］");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::Bracket),
                ("unclosed", PairKind::Bracket),
                ("unmatched", PairKind::Bracket),
            ]
        );
    }

    /// A nested run of stray `［` opens at a line break resolves innermost
    /// first, matching the EOF-drain and hard-scope unwind order.
    #[test]
    fn nested_stray_brackets_resolve_innermost_first_at_line_break() {
        let (events, _) = run("［＃［＃\ntail");
        let unclosed: Vec<&PairEvent> = events
            .iter()
            .filter(|e| matches!(e, PairEvent::Unclosed { .. }))
            .collect();
        assert_eq!(unclosed.len(), 2, "both stray opens resolve: {events:?}");
        let starts: Vec<u32> = unclosed
            .iter()
            .map(|e| e.span().expect("Unclosed has a span").start)
            .collect();
        assert!(
            starts[0] > starts[1],
            "innermost (later start) resolves first; got {starts:?}"
        );
    }

    /// The line scope is additive: a never-closed `［` with NO line break
    /// still drains as `Unclosed` at EOF, exactly as before.
    #[test]
    fn bracket_without_line_break_still_unclosed_at_eof() {
        let (events, _) = run("［＃novel");
        assert!(events.iter().any(|e| matches!(
            e,
            PairEvent::Unclosed {
                kind: PairKind::Bracket,
                ..
            }
        )));
    }

    /// Only `［` gets the line scope. A `《` reading legitimately never spans a
    /// line, but resolving it at a newline would rewrite authorial `《…》`
    /// pairing; dialogue `「」` spans lines outright. So a `《` open across a
    /// line break is left to pair with its later `》` — unchanged behaviour.
    #[test]
    fn ruby_and_quote_are_not_line_scoped() {
        assert_eq!(
            pair_kinds(&run("《か\nに》").0),
            vec![("open", PairKind::Ruby), ("close", PairKind::Ruby)]
        );
        assert_eq!(
            pair_kinds(&run("「せりふ\nつづき」").0),
            vec![("open", PairKind::Quote), ("close", PairKind::Quote)]
        );
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
    /// every byte. Exercises the tokenize → pair pass-through path.
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

    /// `links()` borrows the resolved `(open, close)` side-table in
    /// place. A single balanced ruby pair must record exactly one
    /// `PairLink` carrying the open and close spans — pinning the
    /// non-empty, correctly-populated return so a stub that always
    /// hands back an empty slice is caught.
    #[test]
    fn pair_stream_links_records_resolved_pair() {
        // 《=0..3, かんじ=3..12, 》=12..15.
        let mut stream = pair(tokenize("《かんじ》"));
        // Drive to exhaustion so the close resolves and pushes a link.
        while stream.next().is_some() {}
        assert_eq!(
            stream.links(),
            &[PairLink::new(
                PairKind::Ruby,
                Span::new(0, 3),
                Span::new(12, 15),
            )],
            "one resolved Ruby link with open/close spans, got {:?}",
            stream.links()
        );
    }

    /// `take_links()` drains the resolved side-table: the first call on
    /// an exhausted stream yields the populated links, and — because it
    /// is a drain — a second immediate call yields empty. A stub that
    /// always returns `vec![]` fails the first assertion.
    #[test]
    fn pair_stream_take_links_drains_resolved_pairs() {
        // ［=0..3, ＃=3..6, body=6..12, ］=12..15. The bracket pairs;
        // ＃ is Solo, body is Text — only the bracket produces a link.
        let mut stream = pair(tokenize("［＃青空］"));
        while stream.next().is_some() {}
        let links = stream.take_links();
        assert_eq!(
            links,
            vec![PairLink::new(
                PairKind::Bracket,
                Span::new(0, 3),
                Span::new(12, 15),
            )],
            "take_links must yield the resolved bracket link, got {links:?}"
        );
        // Drain semantics: the buffer is now empty.
        assert!(
            stream.take_links().is_empty(),
            "second take_links must be empty after the prior drain"
        );
    }

    /// `diagnostics()` borrows the accumulated diagnostics in place
    /// (distinct from the draining `take_diagnostics`). After an
    /// unclosed `［＃` reaches EOF, the borrow must expose the one
    /// synthesised `UnclosedBracket` — a stub returning an empty slice
    /// is caught here.
    #[test]
    fn pair_stream_diagnostics_borrows_accumulated() {
        let mut stream = pair(tokenize("［＃unclosed"));
        while stream.next().is_some() {}
        let diags = stream.diagnostics();
        assert_eq!(
            diags.len(),
            1,
            "one unclosed-bracket diagnostic, got {diags:?}"
        );
        assert!(matches!(
            diags[0],
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        ));
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
