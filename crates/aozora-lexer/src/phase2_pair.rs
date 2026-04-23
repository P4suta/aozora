//! Phase 2 — balanced-stack pairing over the Phase 1 event stream.
//!
//! Consumes the flat [`Token`] stream from Phase 1 and links matching
//! opens/closes — `［…］`, `《…》`, `《《…》》`, `〔…〕`, `「…」` — via
//! a single stack pass. The output is still a flat event vector, but
//! every [`PairEvent::PairOpen`] carries a `close_idx` pointing at its
//! matching [`PairEvent::PairClose`] in the same vector (and vice-versa),
//! so Phase 3 can jump between them in O(1) without rescanning bytes.
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
//! * **Unclosed open**: left on the stack at end-of-input. The event is
//!   rewritten from [`PairEvent::PairOpen`] to [`PairEvent::Unclosed`]
//!   and a [`Diagnostic::UnclosedBracket`] is emitted. The stack entry
//!   itself is *not* used to close anything by force, so later (valid)
//!   close delimiters do not accidentally bind to an earlier, distant
//!   open on a different line.
//! * **Stray close** (empty stack or kind-mismatched top): emitted as
//!   [`PairEvent::Unmatched`] with a [`Diagnostic::UnmatchedClose`].
//!   The stack is *not* popped — this is deliberately conservative, so
//!   a well-formed outer pair like `［...］` still closes correctly even
//!   when an inner stray `》` appears inside the body. Phase 3 sees
//!   `Unmatched` in the body event slice and classifies it as plain
//!   text.
//!
//! The recovery policy is intentionally conservative for C3a; C3b can
//! revisit aggressive stack-unwinding if corpus sweep shows it pays.

use afm_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::token::{Token, TriggerKind};

/// Categories of open/close delimiter pairs recognized by Phase 2.
///
/// [`TriggerKind`] values that appear only in isolation (`｜`, `＃`, `※`)
/// do not have a corresponding [`PairKind`]; they become
/// [`PairEvent::Solo`] in the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PairKind {
    /// `［ … ］` (U+FF3B / U+FF3D). The annotation-body container —
    /// always a bracket pair, with or without the leading `＃`.
    Bracket,

    /// `《 … 》` (U+300A / U+300B). Ruby reading.
    Ruby,

    /// `《《 … 》》`. Double-bracket bouten. Open/close are already
    /// merged into single trigger tokens in Phase 1, so the stack
    /// treats them as an independent `PairKind` (a stray inner `》`
    /// never closes a `《《`).
    DoubleRuby,

    /// `〔 … 〕` (U+3014 / U+3015). Accent-decomposition segment per
    /// ADR-0004.
    Tortoise,

    /// `「 … 」` (U+300C / U+300D). Quoted literal inside annotation
    /// bodies (e.g. `［＃「青空」に傍点］`).
    Quote,
}

/// One event in the Phase 2 output.
///
/// Indices referenced by `close_idx` / `open_idx` are into the same
/// `Vec<PairEvent>` the event is a member of. The invariant enforced
/// by [`pair`] is: if `events[i]` is a `PairOpen` with `close_idx == j`,
/// then `events[j]` is the corresponding `PairClose` with
/// `open_idx == i` (and both share the same [`PairKind`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairEvent {
    /// Unchanged from [`Token::Text`] — a byte run between triggers.
    Text { range: Span },

    /// A trigger with no opposing pair on its own (`｜`, `＃`, `※`).
    Solo { kind: TriggerKind, span: Span },

    /// Matched open delimiter with a back-reference to its close.
    PairOpen {
        kind: PairKind,
        span: Span,
        close_idx: usize,
    },

    /// Matched close delimiter with a back-reference to its open.
    PairClose {
        kind: PairKind,
        span: Span,
        open_idx: usize,
    },

    /// Open delimiter that reached end-of-input with no matching close.
    /// Classifier treats the span as plain text.
    Unclosed { kind: PairKind, span: Span },

    /// Close delimiter that hit an empty stack or a kind-mismatched
    /// stack top. Classifier treats the span as plain text.
    Unmatched { kind: PairKind, span: Span },

    /// Unchanged from [`Token::Newline`] — kept so Phase 3 can attach
    /// line structure to block-level annotations.
    Newline { pos: u32 },
}

/// Bundle returned by [`pair`].
///
/// Intentionally does *not* derive `PartialEq` — [`Diagnostic`] wraps
/// miette's [`miette::SourceSpan`] which lacks structural equality, and
/// tests never need to compare whole outputs wholesale. Destructure on
/// `events` and `diagnostics` separately instead.
#[derive(Debug, Clone)]
pub struct PairOutput {
    /// The event stream with cross-linked opens/closes.
    pub events: Vec<PairEvent>,
    /// Non-fatal observations (unclosed opens, unmatched closes).
    pub diagnostics: Vec<Diagnostic>,
}

impl PairEvent {
    /// Source byte-range span of this event, or `None` for
    /// [`PairEvent::Newline`] (which has only a single position, not a
    /// range).
    ///
    /// Exists so Phase 3 can walk an event stream uniformly without a
    /// hand-written match for every variant each time it needs a span.
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

impl PairOutput {
    /// Slice of events strictly inside the pair whose open event is at
    /// `open_idx`, or `None` if `open_idx` does not point at a matched
    /// [`PairEvent::PairOpen`].
    ///
    /// Phase 3 uses this to iterate a bracket body's contents without
    /// re-walking the full stream. The returned slice excludes both
    /// the `PairOpen` and `PairClose` events themselves.
    #[must_use]
    pub fn body_events(&self, open_idx: usize) -> Option<&[PairEvent]> {
        let &PairEvent::PairOpen { close_idx, .. } = self.events.get(open_idx)? else {
            return None;
        };
        // An unclosed open is rewritten to `PairEvent::Unclosed` before
        // `pair()` returns, so any surviving `PairOpen` is guaranteed
        // to have a valid `close_idx`. The bounds check below is a
        // cheap defense against a caller passing an `open_idx` drawn
        // from a stale or hand-constructed output.
        if close_idx <= open_idx || close_idx >= self.events.len() {
            return None;
        }
        Some(&self.events[open_idx + 1..close_idx])
    }

    /// Byte span strictly between the open's end and the close's start
    /// — the *contents* of the pair in the source, excluding the
    /// delimiters themselves. `None` if `open_idx` does not point at a
    /// matched [`PairEvent::PairOpen`].
    #[must_use]
    pub fn body_byte_span(&self, open_idx: usize) -> Option<Span> {
        let &PairEvent::PairOpen {
            span: open_span,
            close_idx,
            ..
        } = self.events.get(open_idx)?
        else {
            return None;
        };
        let &PairEvent::PairClose {
            span: close_span, ..
        } = self.events.get(close_idx)?
        else {
            return None;
        };
        Some(Span::new(open_span.end, close_span.start))
    }
}

/// Mutable state carried through the pairing loop.
///
/// Bundling `events`, `stack`, and `diagnostics` together keeps the
/// trigger-handling helpers below the clippy `too_many_arguments` limit
/// and makes it obvious which state a given helper can touch.
struct PairState {
    events: Vec<PairEvent>,
    diagnostics: Vec<Diagnostic>,
    /// `(kind, event_idx_of_open)`. The `event_idx` always points at a
    /// [`PairEvent::PairOpen`] until that open either gets closed
    /// (entry popped) or rewritten to [`PairEvent::Unclosed`] at EOF.
    stack: Vec<(PairKind, usize)>,
}

/// Run the balanced-stack pass over a Phase 1 token stream.
///
/// Pure function; no I/O. Output event count equals input token count
/// (each input token maps to exactly one output event) — downstream
/// phases rely on this 1:1 correspondence for position tracking.
#[must_use]
pub fn pair(tokens: &[Token]) -> PairOutput {
    let mut state = PairState {
        events: Vec::with_capacity(tokens.len()),
        diagnostics: Vec::new(),
        stack: Vec::new(),
    };

    for tok in tokens {
        match *tok {
            Token::Text { range } => state.events.push(PairEvent::Text { range }),
            Token::Newline { pos } => state.events.push(PairEvent::Newline { pos }),
            Token::Trigger { kind, span } => push_trigger(&mut state, kind, span),
        }
    }

    // Anything still on the stack is an unclosed open; rewrite in place
    // so the caller sees a consistent stream. Emit diagnostics in stack
    // order (innermost last-pushed → outermost first) so miette renders
    // them stably.
    while let Some((kind, idx)) = state.stack.pop() {
        // The stack only ever holds indices of freshly-pushed PairOpen
        // events; an entry pointing at any other variant would be a
        // bug in `push_trigger`.
        let PairEvent::PairOpen { span, .. } = state.events[idx] else {
            unreachable!("stack entry must point at a PairOpen event")
        };
        state.events[idx] = PairEvent::Unclosed { kind, span };
        state
            .diagnostics
            .push(Diagnostic::unclosed_bracket(span, kind));
    }

    PairOutput {
        events: state.events,
        diagnostics: state.diagnostics,
    }
}

/// Handle a single [`Token::Trigger`]: classify as open / close / solo,
/// update the stack, append the corresponding event, and patch
/// cross-link indices when a close finds its open.
fn push_trigger(state: &mut PairState, kind: TriggerKind, span: Span) {
    if let Some(pair_kind) = open_kind_of(kind) {
        let open_idx = state.events.len();
        state.events.push(PairEvent::PairOpen {
            kind: pair_kind,
            span,
            // Patched when the matching close is seen. `usize::MAX` is
            // a sentinel that must never leak: either it is overwritten
            // with the real close index, or the event is rewritten to
            // `Unclosed` at end-of-input.
            close_idx: usize::MAX,
        });
        state.stack.push((pair_kind, open_idx));
        return;
    }

    if let Some(pair_kind) = close_kind_of(kind) {
        if state.stack.last().is_some_and(|&(top, _)| top == pair_kind) {
            let (_, open_idx) = state.stack.pop().expect("last() was Some");
            let close_idx = state.events.len();
            state.events.push(PairEvent::PairClose {
                kind: pair_kind,
                span,
                open_idx,
            });
            let PairEvent::PairOpen {
                close_idx: slot, ..
            } = &mut state.events[open_idx]
            else {
                // Same invariant as in `pair`'s tail loop: the stack
                // only points at PairOpen events.
                unreachable!("stack entry must point at a PairOpen event")
            };
            *slot = close_idx;
        } else {
            state.events.push(PairEvent::Unmatched {
                kind: pair_kind,
                span,
            });
            state
                .diagnostics
                .push(Diagnostic::unmatched_close(span, pair_kind));
        }
        return;
    }

    // Trigger is neither open nor close (Bar / Hash / RefMark).
    state.events.push(PairEvent::Solo { kind, span });
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

    fn run(src: &str) -> PairOutput {
        pair(&tokenize(src))
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
        let out = pair(&[]);
        assert!(out.events.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn plain_text_passes_through_as_text_event() {
        let out = run("hello");
        assert_eq!(out.events.len(), 1);
        assert!(matches!(out.events[0], PairEvent::Text { .. }));
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn simple_bracket_pair_cross_links() {
        let out = run("［body］");
        // Events: PairOpen(Bracket), Text("body"), PairClose(Bracket).
        assert_eq!(out.events.len(), 3);
        let PairEvent::PairOpen {
            kind, close_idx, ..
        } = out.events[0]
        else {
            panic!("expected PairOpen, got {:?}", out.events[0]);
        };
        assert_eq!(kind, PairKind::Bracket);
        assert_eq!(close_idx, 2);
        let PairEvent::PairClose {
            kind: c_kind,
            open_idx,
            ..
        } = out.events[2]
        else {
            panic!("expected PairClose, got {:?}", out.events[2]);
        };
        assert_eq!(c_kind, PairKind::Bracket);
        assert_eq!(open_idx, 0);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn nested_brackets_pair_inner_before_outer() {
        // Two annotation bodies, one nested inside the other.
        let out = run("［＃外［＃内］終］");
        // Expected event sequence (indices):
        //   0 PairOpen Bracket (outer ［)    close_idx = 8
        //   1 Solo Hash
        //   2 Text "外"
        //   3 PairOpen Bracket (inner ［)    close_idx = 6
        //   4 Solo Hash
        //   5 Text "内"
        //   6 PairClose Bracket (inner ］)   open_idx  = 3
        //   7 Text "終"
        //   8 PairClose Bracket (outer ］)   open_idx  = 0
        assert_eq!(out.events.len(), 9);
        let PairEvent::PairOpen {
            close_idx: outer_close,
            ..
        } = out.events[0]
        else {
            panic!();
        };
        let PairEvent::PairOpen {
            close_idx: inner_close,
            ..
        } = out.events[3]
        else {
            panic!();
        };
        assert_eq!(outer_close, 8);
        assert_eq!(inner_close, 6);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn annotation_body_with_quoted_literal_pairs_correctly() {
        // The R2-class case: a 「…」 literal inside a ［＃…］ body.
        let out = run("［＃「青空」に傍点］");
        // Events: ［ #  「 青空 」 に傍点 ］
        //   0 PairOpen Bracket
        //   1 Solo Hash
        //   2 PairOpen Quote
        //   3 Text "青空"
        //   4 PairClose Quote -> open_idx 2
        //   5 Text "に傍点"
        //   6 PairClose Bracket -> open_idx 0
        assert_eq!(out.events.len(), 7);
        let PairEvent::PairOpen {
            kind: outer_kind,
            close_idx: outer_close,
            ..
        } = out.events[0]
        else {
            panic!();
        };
        let PairEvent::PairOpen {
            kind: inner_kind,
            close_idx: inner_close,
            ..
        } = out.events[2]
        else {
            panic!();
        };
        assert_eq!(outer_kind, PairKind::Bracket);
        assert_eq!(inner_kind, PairKind::Quote);
        assert_eq!(outer_close, 6);
        assert_eq!(inner_close, 4);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn ruby_pair_links() {
        let out = run("《かんじ》");
        assert_eq!(
            pair_kinds(&out.events),
            vec![("open", PairKind::Ruby), ("close", PairKind::Ruby)]
        );
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn double_ruby_is_its_own_pair_kind() {
        let out = run("《《X》》");
        assert_eq!(
            pair_kinds(&out.events),
            vec![
                ("open", PairKind::DoubleRuby),
                ("close", PairKind::DoubleRuby),
            ]
        );
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn tortoise_pair_links() {
        let out = run("〔e^〕");
        assert_eq!(
            pair_kinds(&out.events),
            vec![("open", PairKind::Tortoise), ("close", PairKind::Tortoise)]
        );
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn quote_pair_standalone_links() {
        let out = run("「台詞」");
        assert_eq!(
            pair_kinds(&out.events),
            vec![("open", PairKind::Quote), ("close", PairKind::Quote)]
        );
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn solo_bar_hash_refmark_remain_solo() {
        let out = run("｜＃※");
        // Bar and RefMark are Solo always. Hash is Solo here because it
        // is not adjacent to a ［ (that binding is a Phase-3 concern; the
        // pair phase treats every Hash uniformly).
        assert_eq!(out.events.len(), 3);
        for ev in &out.events {
            assert!(
                matches!(ev, PairEvent::Solo { .. }),
                "expected all Solo, got {ev:?}"
            );
        }
    }

    #[test]
    fn newline_passes_through_unchanged() {
        let out = run("a\nb");
        assert_eq!(out.events.len(), 3);
        assert!(matches!(out.events[1], PairEvent::Newline { .. }));
    }

    #[test]
    fn unclosed_bracket_rewrites_event_and_emits_diagnostic() {
        let out = run("［＃unclosed");
        // The only trigger is ［; it never closes.
        assert!(
            out.events.iter().any(|e| matches!(
                e,
                PairEvent::Unclosed {
                    kind: PairKind::Bracket,
                    ..
                }
            )),
            "expected an Unclosed Bracket event in {:?}",
            out.events
        );
        assert!(out.diagnostics.iter().any(|d| matches!(
            d,
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        )),);
    }

    #[test]
    fn unmatched_close_emits_diagnostic_without_affecting_stack() {
        let out = run("stray］text");
        assert!(out.events.iter().any(|e| matches!(
            e,
            PairEvent::Unmatched {
                kind: PairKind::Bracket,
                ..
            }
        )),);
        assert_eq!(out.diagnostics.len(), 1);
    }

    #[test]
    fn mismatched_close_inside_bracket_does_not_pop_outer() {
        // The stray 》 must not accidentally pop the outer ［.
        let out = run("［body》more］");
        // Unmatched Ruby in the middle, but the bracket still closes cleanly.
        let kinds = pair_kinds(&out.events);
        assert_eq!(
            kinds,
            vec![
                ("open", PairKind::Bracket),
                ("unmatched", PairKind::Ruby),
                ("close", PairKind::Bracket),
            ]
        );
        assert_eq!(out.diagnostics.len(), 1);
    }

    #[test]
    fn every_pair_open_has_close_idx_matching_close_open_idx() {
        // Covers the cross-link invariant across several shapes.
        let out = run("［＃「a《b》c」に傍点］");
        for (i, ev) in out.events.iter().enumerate() {
            if let PairEvent::PairOpen {
                kind, close_idx, ..
            } = *ev
            {
                let PairEvent::PairClose {
                    kind: c_kind,
                    open_idx,
                    ..
                } = out.events[close_idx]
                else {
                    panic!("close_idx {close_idx} did not point at a PairClose");
                };
                assert_eq!(kind, c_kind, "kind mismatch at open {i}");
                assert_eq!(open_idx, i, "back-link mismatch at close {close_idx}");
            }
        }
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn event_count_equals_token_count() {
        // 1:1 correspondence invariant — important for downstream
        // position tracking.
        let src = "［＃「a」に］plain《b》〔c〕";
        let toks = tokenize(src);
        let out = pair(&toks);
        assert_eq!(out.events.len(), toks.len());
    }

    #[test]
    fn span_accessor_returns_range_for_text_and_trigger_events() {
        let out = run("a｜b《c》");
        for ev in &out.events {
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
        let out = run("\n");
        assert_eq!(out.events.len(), 1);
        assert!(out.events[0].span().is_none());
    }

    #[test]
    fn body_events_covers_contents_strictly_between_open_and_close() {
        let out = run("［＃「青空」］");
        // Events (indices): 0 ［, 1 ＃, 2 「, 3 "青空", 4 」, 5 ］.
        let body = out.body_events(0).expect("body events for matched ［");
        // Body should be indices 1..5 (Solo#, PairOpenQuote, Text, PairCloseQuote).
        assert_eq!(body.len(), 4);
        assert!(matches!(body[0], PairEvent::Solo { .. }));
        assert!(matches!(
            body[1],
            PairEvent::PairOpen {
                kind: PairKind::Quote,
                ..
            }
        ));
        assert!(matches!(body[2], PairEvent::Text { .. }));
        assert!(matches!(
            body[3],
            PairEvent::PairClose {
                kind: PairKind::Quote,
                ..
            }
        ));
    }

    #[test]
    fn body_events_empty_pair_returns_empty_slice() {
        let out = run("《》");
        let body = out.body_events(0).expect("body events for empty pair");
        assert!(body.is_empty());
    }

    #[test]
    fn body_events_returns_none_for_non_pair_open_index() {
        let out = run("text");
        // Index 0 is a Text event, not a PairOpen.
        assert!(out.body_events(0).is_none());
    }

    #[test]
    fn body_events_returns_none_for_unclosed_open() {
        let out = run("［unclosed");
        // The ［ at idx 0 was rewritten to PairEvent::Unclosed — not a
        // PairOpen, so body_events returns None.
        assert!(out.body_events(0).is_none());
    }

    #[test]
    fn body_events_returns_none_for_out_of_range_index() {
        let out = run("text");
        assert!(out.body_events(9999).is_none());
    }

    #[test]
    fn body_byte_span_is_the_range_between_open_and_close_delimiters() {
        let src = "［ab］";
        // Byte layout: ［(0..3) a(3..4) b(4..5) ］(5..8).
        let out = run(src);
        let span = out.body_byte_span(0).expect("body span for matched ［");
        assert_eq!(span, Span::new(3, 5));
        // Sanity: slicing the original source by this span yields the
        // body text.
        assert_eq!(span.slice(src), "ab");
    }

    #[test]
    fn body_byte_span_returns_none_for_unclosed_open() {
        let out = run("［unclosed");
        assert!(out.body_byte_span(0).is_none());
    }

    #[test]
    fn body_byte_span_returns_none_for_non_open_index() {
        let out = run("text");
        assert!(out.body_byte_span(0).is_none());
    }

    #[test]
    fn no_sentinel_close_idx_escapes_from_pair_output() {
        // Every PairOpen surviving the tail pass must have a real
        // close_idx; usize::MAX must never leak.
        let inputs = [
            "plain",
            "［＃a］",
            "［＃外［＃内］終］",
            "《《《x》》",
            "［unclosed",
            "stray］",
        ];
        for src in inputs {
            let out = run(src);
            for ev in &out.events {
                if let PairEvent::PairOpen { close_idx, .. } = *ev {
                    assert_ne!(close_idx, usize::MAX, "sentinel leaked for {src:?}");
                    assert!(close_idx < out.events.len(), "out-of-range for {src:?}");
                }
            }
        }
    }

    proptest! {
        /// Output is a pure function of input — running the same token
        /// stream twice must produce identical event sequences.
        #[test]
        fn proptest_pair_is_deterministic(src in source_strategy()) {
            let toks = tokenize(&src);
            let a = pair(&toks);
            let b = pair(&toks);
            prop_assert_eq!(a.events, b.events);
        }

        /// 1:1 correspondence: Phase 2 never drops or splits a Phase 1
        /// token.
        #[test]
        fn proptest_event_count_matches_token_count(src in source_strategy()) {
            let toks = tokenize(&src);
            let out = pair(&toks);
            prop_assert_eq!(out.events.len(), toks.len());
        }

        /// No PairOpen survives with the `usize::MAX` placeholder — it
        /// is either cross-linked to a real PairClose or rewritten to
        /// PairEvent::Unclosed.
        #[test]
        fn proptest_no_sentinel_close_idx(src in source_strategy()) {
            let out = pair(&tokenize(&src));
            for ev in &out.events {
                if let PairEvent::PairOpen { close_idx, .. } = *ev {
                    prop_assert_ne!(close_idx, usize::MAX);
                    prop_assert!(close_idx < out.events.len());
                }
            }
        }

        /// Cross-link consistency: for every matched PairOpen at index
        /// `i` with close_idx `c`, events[c] is a PairClose with
        /// matching kind and open_idx back to `i`.
        #[test]
        fn proptest_cross_links_are_consistent(src in source_strategy()) {
            let out = pair(&tokenize(&src));
            for (i, ev) in out.events.iter().enumerate() {
                let &PairEvent::PairOpen { kind, close_idx, .. } = ev else {
                    continue;
                };
                let close = &out.events[close_idx];
                let &PairEvent::PairClose { kind: c_kind, open_idx, .. } = close else {
                    prop_assert!(
                        false,
                        "close_idx {close_idx} at open {i} did not point at a PairClose"
                    );
                    unreachable!();
                };
                prop_assert_eq!(c_kind, kind);
                prop_assert_eq!(open_idx, i);
            }
        }

        /// body_byte_span for every matched pair is inside the span
        /// running from the open's start to the close's end, and is a
        /// valid substring of the source (sliceable without panic).
        #[test]
        fn proptest_body_spans_slice_source_safely(src in source_strategy()) {
            let out = pair(&tokenize(&src));
            for (i, ev) in out.events.iter().enumerate() {
                let &PairEvent::PairOpen { span: open, close_idx, .. } = ev else {
                    continue;
                };
                let body = out.body_byte_span(i).expect("matched pair must have body span");
                let PairEvent::PairClose { span: close, .. } = out.events[close_idx] else {
                    unreachable!("close_idx must point at PairClose by the cross-link test");
                };
                prop_assert!(open.end <= body.start);
                prop_assert!(body.end <= close.start);
                // Must round-trip as a valid UTF-8 substring of the
                // source (i.e. span aligns to char boundaries).
                prop_assert!(
                    src.get(body.start as usize..body.end as usize).is_some(),
                    "body span {body:?} is not a valid str slice of {src:?}"
                );
            }
        }
    }

    fn source_strategy() -> impl Strategy<Value = String> {
        // Healthy mix of plain text, every trigger, and newlines. Cap
        // the character count so shrinking stays fast.
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
