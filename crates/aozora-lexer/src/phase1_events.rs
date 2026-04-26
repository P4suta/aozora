//! Phase 1 — linear tokenization of sanitized source into a token stream.
//!
//! Walks the Phase 0 sanitized text byte-by-byte and exposes a stateful
//! iterator yielding one [`Token`] per delimiter or contiguous text run.
//! Triggers are the Aozora notation marker characters listed in
//! [`TriggerKind`]; everything else flows into [`Token::Text`] runs.
//!
//! After I-2 (deforestation) the public entry point is
//! [`tokenize`], which returns `impl Iterator<Item = Token>` rather than
//! a materialised `Vec<Token>`. Downstream phases (`pair`, `classify`)
//! consume that stream directly so source bytes flow through CPU
//! registers from `&str` input to arena-landed nodes in a single fused
//! chain — no intermediate `Vec<Token>` allocation between phases.
//!
//! ## Multi-character triggers
//!
//! `《《` and `》》` (double-bracket bouten) are emitted as single
//! [`TriggerKind::DoubleRubyOpen`] / [`TriggerKind::DoubleRubyClose`]
//! tokens covering both constituent characters. Phase 2 therefore
//! never has to look ahead past a single `《` to decide whether it was
//! really a double-bracket opener.
//!
//! `［＃` is NOT emitted as a merged trigger: `Hash` after `BracketOpen`
//! is common but not universal (a stray `［` followed by plain text is
//! legal). Phase 2 inspects the two tokens together.
//!
//! ## T1 investigation note (2026-04, negative result)
//!
//! A SIMD-driven rewrite was attempted: replace this char-by-char
//! walker with an eager `aozora_scan::best_scanner().scan_offsets`
//! pass to find triggers, plus `memchr::memchr_iter(b'\n')` for
//! newlines, then merge-walk the two sorted offset streams. The
//! `aozora-scan` crate (`ScalarScanner` + `Avx2Scanner`) was already
//! in place for exactly this purpose.
//!
//! Result on doc 49178 (232 KB Japanese):
//!   legacy walker: 0.41 ms tokenize  (570 MB/s)
//!   SIMD scanner:  1.50 ms tokenize  (155 MB/s)  — 3.7× SLOWER
//!
//! Root cause: `0xE3` is the leading UTF-8 byte of *every* Japanese
//! codepoint (hiragana, katakana, common kanji). The
//! `memchr3(0xE2, 0xE3, 0xEF)` candidate scan therefore returns
//! ~every third byte of Japanese-heavy source as a candidate, and
//! the per-candidate PHF lookup (`classify_trigger_bytes`) costs
//! roughly the same as the legacy walker's UTF-8 decode + 11-arm
//! `match`. Two passes (eager scan + merge-walk consume) end up
//! doing more work than one (fused decode + classify in
//! `Iterator::next`).
//!
//! The aozora-scan design assumed candidate density `< 0.5 %` (the
//! density of *triggers*), but candidate density is set by the
//! density of `0xE3` in source, which on Aozora corpora is closer
//! to 33 %. Same observation applies to `Avx2Scanner` —
//! `_mm256_cmpeq_epi8` against `0xE3` produces a near-saturated
//! mask on Japanese, and the per-bit validation loop dominates.
//!
//! A follow-up fix is plausible but non-trivial: scan for the
//! *middle* trigger byte (`0x80` for Ruby/Quote/Tortoise/RefMark,
//! `0xBC` for Bracket/Hash, `0xBD` for Bar) which is much rarer in
//! Japanese text than `0xE3`, then validate the surrounding bytes.
//! That requires a redesign of the `aozora-scan` candidate
//! discovery primitive (currently locked to leading-byte scans).
//! Not in scope for T1; deferred until measurement justifies the
//! ~6 hour redesign + cross-validation cost.

use aozora_syntax::Span;

use crate::token::{Token, TriggerKind};

/// Streaming tokeniser over sanitized source text.
///
/// The input is expected to already be Phase 0 output (BOM-stripped,
/// LF-normalized). Giving raw source to this iterator is not wrong but
/// means diagnostics and positions reference pre-normalization bytes,
/// which will confuse downstream phases.
///
/// # Panics
///
/// Panics on construction if `source.len()` exceeds [`u32::MAX`]
/// (≈ 4 GiB). All afm spans use `u32` offsets per the
/// `aozora-syntax::Span` contract; inputs that large are rejected
/// loudly rather than silently truncated.
#[must_use]
pub fn tokenize(source: &str) -> Tokenizer<'_> {
    Tokenizer::new(source)
}

/// Streaming Phase 1 tokeniser. Maintains a single-byte cursor and a
/// `text_start` watermark so a Text run is flushed exactly once when
/// the next Trigger / Newline arrives or at end-of-stream.
///
/// A single-slot `pending` buffer holds a Trigger / Newline that was
/// produced *together* with a closing Text run on the same `next()`
/// call: [`Iterator::next`] returns the Text first, then the buffered
/// trigger on the following call, preserving the legacy Phase 1
/// emission order without paying for a `Vec` accumulator.
#[derive(Debug)]
pub struct Tokenizer<'s> {
    source: &'s str,
    cursor: u32,
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
        Self {
            source,
            cursor: 0,
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
        loop {
            if (self.cursor as usize) >= bytes.len() {
                self.finished = true;
                return self.flush_text(self.cursor);
            }
            let b = bytes[self.cursor as usize];

            // ASCII fast path — no Aozora trigger has an ASCII lead
            // byte; only `\n` is structural here.
            if b < 0x80 {
                if b == b'\n' {
                    let pos = self.cursor;
                    let text = self.flush_text(pos);
                    let nl = Token::Newline { pos };
                    self.cursor = pos + 1;
                    self.text_start = self.cursor;
                    return match text {
                        Some(t) => {
                            self.pending = Some(nl);
                            Some(t)
                        }
                        None => Some(nl),
                    };
                }
                self.cursor += 1;
                continue;
            }

            // Multi-byte char: full UTF-8 decode + trigger classify.
            let rest = &self.source[self.cursor as usize..];
            let ch = rest.chars().next().expect("not at end");
            let ch_len = u32::try_from(ch.len_utf8()).expect("char len 1..=4");

            if let Some(kind) = classify_single(ch) {
                let merged = match kind {
                    TriggerKind::RubyOpen if rest[ch.len_utf8()..].starts_with('《') => {
                        Some(TriggerKind::DoubleRubyOpen)
                    }
                    TriggerKind::RubyClose if rest[ch.len_utf8()..].starts_with('》') => {
                        Some(TriggerKind::DoubleRubyClose)
                    }
                    _ => None,
                };
                let (emit_kind, consumed) = merged.map_or((kind, ch_len), |merged_kind| {
                    (merged_kind, merged_kind.source_byte_len())
                });

                let trigger_pos = self.cursor;
                let text = self.flush_text(trigger_pos);
                let trigger = Token::Trigger {
                    kind: emit_kind,
                    span: Span::new(trigger_pos, trigger_pos + consumed),
                };
                self.cursor += consumed;
                self.text_start = self.cursor;
                return match text {
                    Some(t) => {
                        self.pending = Some(trigger);
                        Some(t)
                    }
                    None => Some(trigger),
                };
            }

            self.cursor += ch_len;
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Lower bound 0 (we may be at EOF after a final flush). Upper
        // bound is at most one event per byte (every byte either advances
        // text or starts a new trigger), plus the pending slot.
        let remaining = (self.source.len()).saturating_sub(self.text_start as usize);
        let upper = remaining + usize::from(self.pending.is_some());
        (0, Some(upper))
    }
}

/// Classify a single character into a trigger kind if one applies,
/// otherwise `None`. Double-character triggers (`《《`) are detected
/// by the caller looking ahead after this returns `Some(RubyOpen)`.
const fn classify_single(ch: char) -> Option<TriggerKind> {
    Some(match ch {
        '｜' => TriggerKind::Bar,
        '《' => TriggerKind::RubyOpen,
        '》' => TriggerKind::RubyClose,
        '［' => TriggerKind::BracketOpen,
        '］' => TriggerKind::BracketClose,
        '＃' => TriggerKind::Hash,
        '※' => TriggerKind::RefMark,
        '〔' => TriggerKind::TortoiseOpen,
        '〕' => TriggerKind::TortoiseClose,
        '「' => TriggerKind::QuoteOpen,
        '」' => TriggerKind::QuoteClose,
        _ => return None,
    })
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
        assert!(collect("").is_empty());
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
    fn double_bouten_brackets_merge_into_double_triggers() {
        let toks = collect("《《強調》》");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::DoubleRubyOpen, TriggerKind::DoubleRubyClose,]
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
        // "a"(0..1) before ｜, "b"(4..5) between ｜ and 《, "c"(8..9), "d"(12..13).
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
    fn trigger_span_covers_all_constituent_bytes() {
        let toks = collect("《《ab》》");
        let open_span = toks
            .iter()
            .find_map(|t| match t {
                Token::Trigger {
                    kind: TriggerKind::DoubleRubyOpen,
                    span,
                } => Some(*span),
                _ => None,
            })
            .expect("DoubleRubyOpen present");
        // Double《 → 6 bytes starting at 0.
        assert_eq!(open_span, Span::new(0, 6));
    }
}
