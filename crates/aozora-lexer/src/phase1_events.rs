//! Phase 1 — linear tokenization of sanitized source into an event stream.
//!
//! Consumes the output of Phase 0 (sanitize) and walks it byte by byte,
//! emitting one [`Token`] per delimiter or contiguous text run. Triggers
//! are the Aozora notation marker characters listed in [`TriggerKind`];
//! everything else flows into [`Token::Text`] runs.
//!
//! The phase is a pure function: `fn(&str) -> Vec<Token>`. The resulting
//! stream is the input to Phase 2 (balanced-stack pairing).
//!
//! ## Why a token stream rather than direct string indexing
//!
//! The subsequent phases (pair, classify) need to repeatedly ask "where
//! is the next `］` after position P?" or "are these two `《` adjacent?".
//! Doing that directly on the string means re-scanning bytes. Fixing it
//! once here, in an `O(n)` linear walk that produces a `Vec<Token>`,
//! lets downstream phases iterate the vector without reparsing —
//! classical compiler-front-end discipline (lex → parse → …).
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

use aozora_syntax::Span;

use crate::token::{Token, TriggerKind};

/// Linear-time tokenize over sanitized source text.
///
/// The input is expected to already be Phase 0 output (BOM-stripped,
/// LF-normalized). Giving raw source to this phase is not wrong but
/// means diagnostics and positions reference pre-normalization bytes,
/// which will confuse downstream phases.
///
/// # Panics
///
/// Panics if `source.len()` exceeds [`u32::MAX`] (≈ 4 GiB). All afm spans
/// use `u32` offsets per the `afm-syntax::Span` contract; inputs that
/// large are rejected loudly rather than silently truncated.
#[must_use]
pub fn tokenize(source: &str) -> Vec<Token> {
    assert!(
        u32::try_from(source.len()).is_ok(),
        "source too long for u32 span offsets ({} bytes)",
        source.len()
    );

    let mut out = Vec::with_capacity(source.len() / 32);
    let mut cursor: u32 = 0;
    let mut text_start: u32 = 0;

    while (cursor as usize) < source.len() {
        let rest = &source[cursor as usize..];
        let ch = rest.chars().next().expect("not at end");
        let ch_len = u32::try_from(ch.len_utf8()).expect("char len 1..=4");

        if let Some(kind) = classify_single(ch) {
            // Look ahead for double-trigger merge (《《 / 》》).
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

            push_text(&mut out, text_start, cursor);
            out.push(Token::Trigger {
                kind: emit_kind,
                span: Span::new(cursor, cursor + consumed),
            });
            cursor += consumed;
            text_start = cursor;
            continue;
        }

        if ch == '\n' {
            push_text(&mut out, text_start, cursor);
            out.push(Token::Newline { pos: cursor });
            cursor += ch_len;
            text_start = cursor;
            continue;
        }

        cursor += ch_len;
    }

    push_text(&mut out, text_start, cursor);
    out
}

fn push_text(out: &mut Vec<Token>, start: u32, end: u32) {
    if end > start {
        out.push(Token::Text {
            range: Span::new(start, end),
        });
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
        let toks = tokenize("hello world こんにちは");
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
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn single_newline_emits_newline_token() {
        let toks = tokenize("\n");
        assert_eq!(toks.len(), 1);
        assert!(matches!(toks[0], Token::Newline { pos: 0 }));
    }

    #[test]
    fn explicit_ruby_emits_bar_open_close() {
        let toks = tokenize("a｜漢字《かんじ》b");
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
        let toks = tokenize("《《強調》》");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::DoubleRubyOpen, TriggerKind::DoubleRubyClose,]
        );
    }

    #[test]
    fn bracket_annotation_emits_each_component_separately() {
        let toks = tokenize("［＃改ページ］");
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
        let toks = tokenize("※［＃「木」、1-2-3］");
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
        let toks = tokenize("〔e^〕");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::TortoiseOpen, TriggerKind::TortoiseClose]
        );
    }

    #[test]
    fn text_between_triggers_is_preserved() {
        let toks = tokenize("a｜b《c》d");
        let text_ranges: Vec<Span> = toks
            .iter()
            .filter_map(|t| match t {
                Token::Text { range } => Some(*range),
                _ => None,
            })
            .collect();
        // "a"(0..1) before ｜, "b"(4..5) between ｜ and 《, "c"(8..11 wait...
        // Actually: ｜ is 3 bytes (U+FF5C). "a" at 0..1. ｜ at 1..4. "b" at 4..5.
        // 《 (U+300A) = 3 bytes at 5..8. "c" at 8..9. 》 (U+300B) at 9..12. "d" at 12..13.
        assert_eq!(text_ranges.len(), 4);
        assert_eq!(text_ranges[0], Span::new(0, 1));
        assert_eq!(text_ranges[1], Span::new(4, 5));
        assert_eq!(text_ranges[2], Span::new(8, 9));
        assert_eq!(text_ranges[3], Span::new(12, 13));
    }

    #[test]
    fn adjacent_triggers_produce_no_empty_text_tokens() {
        let toks = tokenize("｜《》");
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
        let toks = tokenize("line1\nline2");
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
        let toks = tokenize("《《ab》》");
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
