#![expect(
    clippy::expect_used,
    reason = "stack shape is checked before the matching pair is removed"
)]

use core::mem;

use crate::syntax::Span;
use smallvec::SmallVec;

use super::token::{Token, TriggerKind};
use crate::spec::Diagnostic;

pub(crate) use crate::spec::{PairKind, PairLink};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum PairEvent {
    Text { range: Span },

    Solo { kind: TriggerKind, span: Span },

    PairOpen { kind: PairKind, span: Span },

    PairClose { kind: PairKind, span: Span },

    Unclosed { kind: PairKind, span: Span },

    Unmatched { kind: PairKind, span: Span },

    Newline { pos: u32 },
}

impl PairEvent {
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

#[must_use]
pub(crate) fn pair<I>(tokens: I) -> PairStream<I>
where
    I: Iterator<Item = Token>,
{
    PairStream::new(tokens)
}

// This inline stack capacity covers the measured ninety-ninth percentile of corpus delimiter nesting.
#[derive(Debug)]
pub(crate) struct PairStream<I>
where
    I: Iterator<Item = Token>,
{
    tokens: I,
    stack: SmallVec<[(PairKind, Span); 8]>,
    diagnostics: Vec<Diagnostic>,
    links: Vec<PairLink>,
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

    pub(crate) fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        mem::take(&mut self.diagnostics)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn take_links(&mut self) -> Vec<PairLink> {
        mem::take(&mut self.links)
    }

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

            // ADR-0025 makes a bracket close end nested dangling quotes to bound malformed input.
            if pair_kind == PairKind::Bracket
                && let Some(bracket_pos) = self
                    .stack
                    .iter()
                    .rposition(|&(k, _)| k == PairKind::Bracket)
            {
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
                self.pending.reverse();
                return self.pending.pop().expect("hard-scope unwind is non-empty");
            }

            self.diagnostics
                .push(Diagnostic::unmatched_close(span, pair_kind));
            return PairEvent::Unmatched {
                kind: pair_kind,
                span,
            };
        }

        PairEvent::Solo { kind, span }
    }

    // ADR-0030 ends stray brackets at newlines to keep malformed directives within one line.
    fn newline_event(&mut self, pos: u32) -> PairEvent {
        if let Some(bracket_pos) = self
            .stack
            .iter()
            .position(|&(kind, _)| kind == PairKind::Bracket)
        {
            while self.stack.len() > bracket_pos {
                let (kind, open_span) = self.stack.pop().expect("frame at bracket position");
                self.diagnostics
                    .push(Diagnostic::unclosed_bracket(open_span, kind));
                self.pending.push(PairEvent::Unclosed {
                    kind,
                    span: open_span,
                });
            }
        }
        if self.pending.is_empty() {
            return PairEvent::Newline { pos };
        }
        self.pending.push(PairEvent::Newline { pos });
        self.pending.reverse();
        self.pending.pop().expect("newline unwind is non-empty")
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
        if let Some(event) = self.pending.pop() {
            return Some(event);
        }
        if self.eof_drain {
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
                self.eof_drain = true;
                self.next()
            }
        }
    }
}

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
        assert_eq!(events, Vec::<PairEvent>::new());
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

    #[test]
    fn stray_bracket_unwinds_inner_frames_but_preserves_outer_scope() {
        let (events, diagnostics) = run("「外［＃「未閉じ\n《か》続き」");
        assert_eq!(
            pair_kinds(&events),
            vec![
                ("open", PairKind::Quote),
                ("open", PairKind::Bracket),
                ("open", PairKind::Quote),
                ("unclosed", PairKind::Quote),
                ("unclosed", PairKind::Bracket),
                ("open", PairKind::Ruby),
                ("close", PairKind::Ruby),
                ("close", PairKind::Quote),
            ]
        );
        assert_eq!(diagnostics.len(), 2);
        assert!(matches!(
            diagnostics[0],
            Diagnostic::UnclosedBracket {
                kind: PairKind::Quote,
                ..
            }
        ));
        assert!(matches!(
            diagnostics[1],
            Diagnostic::UnclosedBracket {
                kind: PairKind::Bracket,
                ..
            }
        ));

        let newline = events
            .iter()
            .position(|event| matches!(event, PairEvent::Newline { .. }))
            .expect("newline event");
        assert!(
            events
                .iter()
                .enumerate()
                .filter(|(_, event)| matches!(event, PairEvent::Unclosed { .. }))
                .all(|(index, _)| index < newline)
        );
    }

    #[test]
    fn malformed_directive_line_does_not_capture_following_ruby() {
        let document = crate::Document::new("［＃「oops\n漢《かん》");
        let snapshot = document.snapshot();
        assert!(
            snapshot
                .nodes()
                .iter()
                .any(|node| node.kind() == crate::NodeKind::Ruby)
        );
        assert!(
            snapshot
                .to_html()
                .contains("<ruby>漢<rp>(</rp><rt>かん</rt><rp>)</rp></ruby>")
        );
        let unclosed = snapshot
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| match diagnostic {
                Diagnostic::UnclosedBracket { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(unclosed, vec![PairKind::Bracket, PairKind::Quote]);
    }

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

    #[test]
    fn deep_scope_unwind_drains_in_emission_order() {
        let depth = 4_096;
        for terminator in ["］", "\n"] {
            let mut source = String::from("［");
            source.push_str(&"「".repeat(depth));
            source.push_str(terminator);

            let (events, diagnostics) = run(&source);
            let starts = events
                .iter()
                .filter_map(|event| match event {
                    PairEvent::Unclosed { span, .. } => Some(span.start),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let expected = depth + usize::from(terminator == "\n");
            assert_eq!(starts.len(), expected);
            assert_eq!(diagnostics.len(), expected);
            assert!(starts.windows(2).all(|window| window[0] > window[1]));
            assert!(
                matches!(
                    events.last(),
                    Some(PairEvent::PairClose {
                        kind: PairKind::Bracket,
                        ..
                    }) if terminator == "］"
                ) || matches!(events.last(), Some(PairEvent::Newline { .. }))
            );
        }
    }

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

    #[test]
    fn pair_stream_eof_drains_innermost_first_after_multiple_unclosed() {
        let (events, diagnostics) = run("［＃［＃［＃");
        let unclosed: Vec<&PairEvent> = events
            .iter()
            .filter(|e| matches!(e, PairEvent::Unclosed { .. }))
            .collect();
        assert_eq!(unclosed.len(), 3, "events were {events:?}");

        let starts: Vec<u32> = unclosed
            .iter()
            .map(|e| e.span().expect("Unclosed has a span").start)
            .collect();
        assert!(
            starts[0] > starts[1] && starts[1] > starts[2],
            "EOF drain order should be innermost-first; got starts={starts:?}"
        );

        let bracket_diag_count = diagnostics
            .iter()
            .filter(|d| matches!(d, Diagnostic::UnclosedBracket { .. }))
            .count();
        assert_eq!(bracket_diag_count, 3);
    }

    #[test]
    fn pair_stream_take_diagnostics_only_complete_after_exhaustion() {
        let mut stream = pair(tokenize("stray］more text［＃tail"));
        for _ in 0..4 {
            let _ = stream.next();
        }
        let mid = stream.take_diagnostics();
        let _ = mid.len();

        while stream.next().is_some() {}
        let after = stream.take_diagnostics();
        let again = stream.take_diagnostics();
        assert!(
            again.is_empty(),
            "second take_diagnostics must return empty after the prior drain, got {again:?}"
        );
        assert!(
            !after.is_empty() || mid.iter().any(|_| true),
            "expected at least one diagnostic across the two drains for this input"
        );
    }

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

    #[test]
    fn pair_stream_links_records_resolved_pair() {
        let mut stream = pair(tokenize("《かんじ》"));
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

    #[test]
    fn pair_stream_take_links_drains_resolved_pairs() {
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
        assert!(
            stream.take_links().is_empty(),
            "second take_links must be empty after the prior drain"
        );
    }

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
        #[test]
        fn proptest_pair_is_deterministic(src in source_strategy()) {
            let (a, _) = run(&src);
            let (b, _) = run(&src);
            prop_assert_eq!(a, b);
        }

        #[test]
        fn proptest_every_open_resolves(src in source_strategy()) {
            let (events, _) = run(&src);
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
