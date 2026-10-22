#![expect(
    clippy::expect_used,
    reason = "scanner branch guards establish the selected trigger position"
)]

//! Splits sanitized source into delimiter, newline, and text tokens through [`tokenize`].

use crate::scan::scan_offsets;
use crate::spec::classify_trigger_bytes;
use crate::syntax::Span;

use super::token::{Token, TriggerKind};

const ESTIMATED_LINE_BYTES: usize = 64;

#[must_use]
pub(crate) fn tokenize(source: &str) -> Tokenizer<'_> {
    Tokenizer::new(source)
}

#[derive(Debug)]
pub(crate) struct Tokenizer<'s> {
    source: &'s str,
    trigger_offsets: Vec<u32>,
    newline_offsets: Vec<u32>,
    t_idx: usize,
    n_idx: usize,
    text_start: u32,
    pending: Option<Token>,
    finished: bool,
}

impl<'s> Tokenizer<'s> {
    fn new(source: &'s str) -> Self {
        assert!(
            u32::try_from(source.len()).is_ok(),
            "source too long for u32 span offsets ({} bytes)",
            source.len()
        );
        let trigger_offsets = scan_offsets(source);
        let bytes = source.as_bytes();
        let mut newline_offsets: Vec<u32> = Vec::with_capacity(bytes.len() / ESTIMATED_LINE_BYTES);
        for n in memchr::memchr_iter(b'\n', bytes) {
            newline_offsets.push(u32::try_from(n).unwrap_or(u32::MAX));
        }
        Self {
            source,
            trigger_offsets,
            newline_offsets,
            t_idx: 0,
            n_idx: 0,
            text_start: 0,
            pending: None,
            finished: false,
        }
    }

    fn flush_text(&mut self, end: u32) -> Option<Token> {
        (end > self.text_start).then(|| {
            let tok = Token::Text {
                range: Span::new(self.text_start, end),
            };
            self.text_start = end;
            tok
        })
    }

    fn pair_text_then(&mut self, text: Option<Token>, event: Token) -> Token {
        match text {
            Some(t) => {
                self.pending = Some(event);
                t
            }
            None => event,
        }
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        if let Some(tok) = self.pending.take() {
            return Some(tok);
        }
        if self.finished {
            return None;
        }

        let bytes = self.source.as_bytes();
        let t_offset = self.trigger_offsets.get(self.t_idx).copied();
        let n_offset = self.newline_offsets.get(self.n_idx).copied();

        let next_is_trigger = match (t_offset, n_offset) {
            (Some(t), Some(n)) => t.cmp(&n).is_lt(),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => {
                self.finished = true;
                let total_len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
                return self.flush_text(total_len);
            }
        };

        if next_is_trigger {
            let t_pos = t_offset.expect("checked Some by next_is_trigger arm");
            let kind = trigger_kind_at(bytes, t_pos as usize);
            let byte_len = kind.source_byte_len();
            let trigger = Token::Trigger {
                kind,
                span: Span::new(t_pos, t_pos + byte_len),
            };
            let text = self.flush_text(t_pos);
            self.text_start = t_pos.saturating_add(byte_len);
            let previous = self.t_idx;
            self.t_idx = self.t_idx.saturating_add(1);
            assert!(self.t_idx > previous, "trigger cursor must advance");
            Some(self.pair_text_then(text, trigger))
        } else {
            let n_pos = n_offset.expect("checked Some by !next_is_trigger arm");
            let text = self.flush_text(n_pos);
            let nl = Token::Newline { pos: n_pos };
            self.text_start = n_pos.saturating_add(1);
            let previous = self.n_idx;
            self.n_idx = self.n_idx.saturating_add(1);
            assert!(self.n_idx > previous, "newline cursor must advance");
            Some(self.pair_text_then(text, nl))
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let triggers_left = self.trigger_offsets.len().saturating_sub(self.t_idx);
        let newlines_left = self.newline_offsets.len().saturating_sub(self.n_idx);
        let upper = (triggers_left + newlines_left) * 2 + usize::from(self.pending.is_some()) + 1;
        (0, Some(upper))
    }
}

#[inline]
fn trigger_kind_at(bytes: &[u8], pos: usize) -> TriggerKind {
    let window: [u8; 3] = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
    classify_trigger_bytes(window).expect("scanner only emits classified positions")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(src: &str) -> Vec<Token> {
        tokenize(src).collect()
    }

    fn triggers(tokens: &[Token]) -> Vec<TriggerKind> {
        tokens
            .iter()
            .filter_map(|t| match t {
                Token::Trigger { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plain_text_is_one_text_token() {
        let toks = collect("hello world こんにちは");
        assert_eq!(toks.len(), 1);
        match &toks[0] {
            Token::Text { range } => {
                assert_eq!(range.start, 0);
                assert_eq!(range.end as usize, "hello world こんにちは".len());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        assert_eq!(collect(""), Vec::<Token>::new());
    }

    #[test]
    fn single_newline_emits_newline_token() {
        let toks = collect("\n");
        assert_eq!(toks.len(), 1);
        assert!(matches!(toks[0], Token::Newline { pos: 0 }));
    }

    #[test]
    fn explicit_ruby_emits_bar_open_close() {
        let toks = collect("a｜漢字《かんじ》b");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![
                TriggerKind::Bar,
                TriggerKind::RubyOpen,
                TriggerKind::RubyClose,
            ]
        );
    }

    #[test]
    fn angle_quote_brackets_are_single_triggers() {
        let toks = collect("≪強調≫");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::AngleQuoteOpen, TriggerKind::AngleQuoteClose,]
        );
    }

    #[test]
    fn bracket_annotation_emits_each_component_separately() {
        let toks = collect("［＃改ページ］");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![
                TriggerKind::BracketOpen,
                TriggerKind::Hash,
                TriggerKind::BracketClose,
            ]
        );
    }

    #[test]
    fn gaiji_ref_mark_is_emitted() {
        let toks = collect("※［＃「木」、1-2-3］");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![
                TriggerKind::RefMark,
                TriggerKind::BracketOpen,
                TriggerKind::Hash,
                TriggerKind::QuoteOpen,
                TriggerKind::QuoteClose,
                TriggerKind::BracketClose,
            ]
        );
    }

    #[test]
    fn tortoise_brackets_emit_dedicated_triggers() {
        let toks = collect("〔e^〕");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::TortoiseOpen, TriggerKind::TortoiseClose]
        );
    }

    #[test]
    fn text_between_triggers_is_preserved() {
        let toks = collect("a｜b《c》d");
        let text_ranges: Vec<Span> = toks
            .iter()
            .filter_map(|t| match t {
                Token::Text { range } => Some(*range),
                _ => None,
            })
            .collect();
        assert_eq!(text_ranges.len(), 4);
        assert_eq!(text_ranges[0], Span::new(0, 1));
        assert_eq!(text_ranges[1], Span::new(4, 5));
        assert_eq!(text_ranges[2], Span::new(8, 9));
        assert_eq!(text_ranges[3], Span::new(12, 13));
    }

    #[test]
    fn adjacent_triggers_produce_no_empty_text_tokens() {
        let toks = collect("｜《》");
        for tok in &toks {
            if let Token::Text { range } = tok {
                assert!(
                    range.end > range.start,
                    "empty Text token leaked into stream: {tok:?}"
                );
            }
        }
    }

    #[test]
    fn newline_is_its_own_token_between_text_runs() {
        let toks = collect("line1\nline2");
        assert_eq!(toks.len(), 3);
        match &toks[0] {
            Token::Text { range } => assert_eq!(*range, Span::new(0, 5)),
            other => panic!("expected Text, got {other:?}"),
        }
        assert!(matches!(toks[1], Token::Newline { pos: 5 }));
        match &toks[2] {
            Token::Text { range } => assert_eq!(*range, Span::new(6, 11)),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn size_hint_upper_bound_is_exact_at_construction() {
        let tk = tokenize("｜《》");
        assert_eq!(tk.size_hint(), (0, Some(7)));
        assert_eq!(triggers(&tk.collect::<Vec<_>>()).len(), 3);
    }

    #[test]
    fn trigger_span_covers_all_constituent_bytes() {
        let toks = collect("≪ab≫");
        let open_span = toks
            .iter()
            .find_map(|t| match t {
                Token::Trigger {
                    kind: TriggerKind::AngleQuoteOpen,
                    span,
                } => Some(*span),
                _ => None,
            })
            .expect("AngleQuoteOpen present");
        assert_eq!(open_span, Span::new(0, 3));
    }
}
