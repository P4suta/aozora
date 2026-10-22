//! Prepares source for tokenization through [`sanitize`].
//! Resolves accent notation before structural parsing under ADR-0003.

use std::borrow::Cow;

use memchr::memmem;

use crate::syntax::Span;
use crate::syntax::accent::decompose_fragment;

use crate::spec::Diagnostic;
use crate::spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};

const TORTOISE_OPEN: char = '〔';
const TORTOISE_OPEN_BYTES: &[u8] = "〔".as_bytes();
const TORTOISE_CLOSE: char = '〕';
const RULE_ISOLATION_EXTRA_CAPACITY: usize = 16;
const SENTINEL_BYTE_LEN: u32 = 3;
const SENTINEL_LEAD_BYTE: u8 = 0xEE;
const SENTINEL_MIDDLE_BYTE: u8 = 0x80;
const INLINE_SENTINEL_TAIL: u8 = 0x81;
const BLOCK_LEAF_SENTINEL_TAIL: u8 = 0x82;
const BLOCK_OPEN_SENTINEL_TAIL: u8 = 0x83;
const BLOCK_CLOSE_SENTINEL_TAIL: u8 = 0x84;

/// Corpus separators exceed the longest setext underline in CommonMark 0.31.2 examples.
const DECORATIVE_RULE_MIN_LEN: usize = 10;

#[derive(Debug, Clone)]
pub(crate) struct SanitizeOutput<'s> {
    pub text: Cow<'s, str>,
    pub diagnostics: Vec<Diagnostic>,
    pub source_unchanged: bool,
}

#[must_use]
pub(crate) fn sanitize(source: &str) -> SanitizeOutput<'_> {
    let mut after_bom = source;
    while let Some(rest) = after_bom.strip_prefix('\u{FEFF}') {
        after_bom = rest;
    }

    let line_normalized: Cow<'_, str> = if after_bom.contains('\r') {
        Cow::Owned(normalize_line_endings(after_bom))
    } else {
        Cow::Borrowed(after_bom)
    };

    let rule_isolated: Cow<'_, str> = if has_long_rule_line(&line_normalized) {
        Cow::Owned(isolate_decorative_rules(&line_normalized))
    } else {
        line_normalized
    };

    let mut accent_diagnostics: Vec<Diagnostic> = Vec::new();
    let text: Cow<'_, str> =
        if memmem::find(rule_isolated.as_bytes(), TORTOISE_OPEN_BYTES).is_some() {
            let owned = rule_isolated.into_owned();
            Cow::Owned(rewrite_accent_spans_collecting(
                &owned,
                &mut accent_diagnostics,
            ))
        } else {
            rule_isolated
        };

    let (text, pua_diagnostics) = neutralize_sentinel_collisions(text);

    let mut diagnostics = accent_diagnostics;
    diagnostics.extend(pua_diagnostics);

    let source_unchanged = matches!(&text, Cow::Borrowed(value) if value.len() == source.len());
    SanitizeOutput {
        text,
        diagnostics,
        source_unchanged,
    }
}

fn neutralize_sentinel_collisions(text: Cow<'_, str>) -> (Cow<'_, str>, Vec<Diagnostic>) {
    let diagnostics = scan_for_sentinel_collisions(&text);
    if diagnostics.is_empty() {
        return (text, diagnostics);
    }

    let src = text.as_ref();
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    for diag in &diagnostics {
        let Diagnostic::SourceContainsPua { span, .. } = diag else {
            continue;
        };
        let start = span.start as usize;
        let end = span.end as usize;
        out.push_str(&src[cursor..start]);
        out.push(REPLACEMENT_CHAR);
        cursor = end;
    }
    out.push_str(&src[cursor..]);

    (Cow::Owned(out), diagnostics)
}

const REPLACEMENT_CHAR: char = '\u{FFFD}';

#[cfg(test)]
#[must_use]
pub(crate) fn rewrite_accent_spans(input: &str) -> String {
    let mut sink = Vec::new();
    rewrite_accent_spans_collecting(input, &mut sink)
}

/// Diagnostic spans refer to the rewritten output.
fn rewrite_accent_spans_collecting(input: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(rest) = input.get(cursor..).filter(|rest| !rest.is_empty()) {
        let Some(open_rel) = rest.find(TORTOISE_OPEN) else {
            out.push_str(rest);
            break;
        };
        let open_abs = cursor.saturating_add(open_rel);
        out.push_str(&input[cursor..open_abs]);

        let after_open = open_abs.saturating_add(TORTOISE_OPEN.len_utf8());
        let Some(close_rel) = input[after_open..].find(TORTOISE_CLOSE) else {
            out.push_str(&input[open_abs..]);
            break;
        };
        let close_abs = after_open.saturating_add(close_rel);

        let body = &input[after_open..close_abs];
        let decomposed = decompose_fragment(body);

        let out_open = out.len();
        out.push(TORTOISE_OPEN);
        out.push_str(&decomposed);
        out.push(TORTOISE_CLOSE);
        let out_close = out.len();

        if decomposed.as_ref() != body {
            diagnostics.push(Diagnostic::accent_decomposition_applied(Span::new(
                u32::try_from(out_open).unwrap_or(u32::MAX),
                u32::try_from(out_close).unwrap_or(u32::MAX),
            )));
        }

        let previous = cursor;
        cursor = close_abs.saturating_add(TORTOISE_CLOSE.len_utf8());
        assert!(cursor > previous, "accent rewrite must advance");
    }

    out
}

pub(crate) fn has_long_rule_line(input: &str) -> bool {
    input.lines().any(is_decorative_rule_line)
}

fn is_decorative_rule_line(line: &str) -> bool {
    is_rule_line_trimmed(line.trim())
}

#[must_use]
pub(crate) fn is_rule_line_trimmed(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    if bytes.len() < DECORATIVE_RULE_MIN_LEN {
        return false;
    }
    let first = bytes[0];
    if !matches!(first, b'-' | b'=' | b'_') {
        return false;
    }
    bytes.iter().all(|&b| b == first)
}

#[must_use]
pub(crate) fn isolate_decorative_rules(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len() + RULE_ISOLATION_EXTRA_CAPACITY);
    let mut line_start: usize = 0;
    let mut copy_from: usize = 0;
    let mut prev_nonblank = false;

    for nl_pos in memchr::memchr_iter(b'\n', bytes) {
        let line_no_eol = &input[line_start..nl_pos];
        let trimmed = line_no_eol.trim();
        if is_rule_line_trimmed(trimmed) && prev_nonblank {
            out.push_str(&input[copy_from..line_start]);
            out.push('\n');
            copy_from = line_start;
        }
        prev_nonblank = !trimmed.is_empty();
        let previous = line_start;
        line_start = nl_pos.saturating_add(1);
        assert!(line_start > previous, "line scan must advance");
    }
    if let Some(tail) = input.get(line_start..).filter(|tail| !tail.is_empty()) {
        let tail_trimmed = tail.trim();
        if is_rule_line_trimmed(tail_trimmed) && prev_nonblank {
            out.push_str(&input[copy_from..line_start]);
            out.push('\n');
            copy_from = line_start;
        }
    }
    if let Some(tail) = input.get(copy_from..).filter(|tail| !tail.is_empty()) {
        out.push_str(tail);
    }
    out
}

#[must_use]
pub(crate) fn normalize_line_endings(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    for cr_pos in memchr::memchr_iter(b'\r', bytes) {
        out.push_str(&input[cursor..cr_pos]);
        out.push('\n');
        let after_cr = cr_pos.saturating_add(1);
        cursor = if bytes.get(after_cr) == Some(&b'\n') {
            after_cr.saturating_add(1)
        } else {
            after_cr
        };
    }
    if let Some(tail) = input.get(cursor..).filter(|tail| !tail.is_empty()) {
        out.push_str(tail);
    }
    out
}

#[must_use]
pub(crate) fn scan_for_sentinel_collisions(text: &str) -> Vec<Diagnostic> {
    let bytes = text.as_bytes();
    let mut diagnostics = Vec::new();
    for cand in memchr::memchr_iter(SENTINEL_LEAD_BYTE, bytes) {
        if cand + SENTINEL_BYTE_LEN as usize > bytes.len() {
            continue;
        }
        if bytes[cand + 1] != SENTINEL_MIDDLE_BYTE {
            continue;
        }
        let third = bytes[cand + 2];
        let codepoint = match third {
            INLINE_SENTINEL_TAIL => INLINE_SENTINEL,
            BLOCK_LEAF_SENTINEL_TAIL => BLOCK_LEAF_SENTINEL,
            BLOCK_OPEN_SENTINEL_TAIL => BLOCK_OPEN_SENTINEL,
            BLOCK_CLOSE_SENTINEL_TAIL => BLOCK_CLOSE_SENTINEL,
            _ => continue,
        };
        let abs_start = u32::try_from(cand).unwrap_or(u32::MAX);
        diagnostics.push(Diagnostic::source_contains_pua(
            Span::new(abs_start, abs_start + SENTINEL_BYTE_LEN),
            codepoint,
        ));
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_is_borrowed_and_unchanged() {
        let input = "hello world";
        let out = sanitize(input);
        assert!(matches!(out.text, Cow::Borrowed(_)));
        assert_eq!(out.text.as_ref(), input);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn leading_bom_is_stripped() {
        let input = "\u{FEFF}hello";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "hello");
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn bom_only_inside_source_is_not_stripped() {
        let input = "abc\u{FEFF}def";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn stacked_leading_boms_are_all_stripped() {
        let input = "\u{FEFF}\u{FEFF}\u{FEFF}hello";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "hello");
    }

    #[test]
    fn leading_boms_only_resolve_to_empty() {
        let out = sanitize("\u{FEFF}\u{FEFF}");
        assert_eq!(out.text.as_ref(), "");
    }

    #[test]
    fn crlf_is_normalized_to_lf() {
        let input = "line1\r\nline2\r\nline3";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "line1\nline2\nline3");
        assert!(matches!(out.text, Cow::Owned(_)));
    }

    #[test]
    fn lone_cr_is_normalized_to_lf() {
        let input = "old-mac\rstyle";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "old-mac\nstyle");
    }

    #[test]
    fn mixed_cr_and_crlf_both_become_single_lf() {
        let input = "a\r\nb\rc\r\nd";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "a\nb\nc\nd");
    }

    #[test]
    fn pua_inline_sentinel_emits_one_diagnostic_and_neutralizes_to_fffd() {
        let input = "plain\u{E001}text";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 1);
        let Diagnostic::SourceContainsPua { codepoint, .. } = &out.diagnostics[0] else {
            panic!("expected SourceContainsPua, got {:?}", out.diagnostics[0]);
        };
        assert_eq!(*codepoint, '\u{E001}');
        assert_eq!(out.text.as_ref(), "plain\u{FFFD}text");
        assert!(!out.text.contains('\u{E001}'));
        assert!(matches!(out.text, Cow::Owned(_)));
    }

    #[test]
    fn pua_all_four_sentinels_emit_four_diagnostics_and_all_become_fffd() {
        let input = "\u{E001}\u{E002}\u{E003}\u{E004}";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 4);
        assert_eq!(out.text.as_ref(), "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}");
        for raw in ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
            assert!(!out.text.contains(raw), "raw sentinel {raw:?} leaked");
        }
        assert_eq!(out.text.len(), input.len());
    }

    #[test]
    fn non_sentinel_pua_codepoints_do_not_emit_diagnostics_and_stay_borrowed() {
        let input = "\u{E000}\u{E100}\u{F8FF}";
        let out = sanitize(input);
        assert!(out.diagnostics.is_empty());
        assert!(matches!(out.text, Cow::Borrowed(_)));
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn mixed_sentinel_and_non_sentinel_pua_only_neutralizes_sentinels() {
        let input = "\u{E000}\u{E002}\u{F8FF}";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.text.as_ref(), "\u{E000}\u{FFFD}\u{F8FF}");
    }

    #[test]
    fn plain_text_without_sentinels_skips_neutralization_allocation() {
        let input = "ふつうの日本語 and some ASCII.";
        let out = sanitize(input);
        assert!(out.diagnostics.is_empty());
        assert!(matches!(out.text, Cow::Borrowed(_)));
    }

    #[test]
    fn pua_diagnostic_span_points_at_sentinel_position_after_neutralization() {
        let input = "ab\u{E002}cd";
        let out = sanitize(input);
        let Diagnostic::SourceContainsPua { span, .. } = &out.diagnostics[0] else {
            panic!("expected SourceContainsPua, got {:?}", out.diagnostics[0]);
        };
        assert_eq!(span.start, 2);
        assert_eq!(span.end, 5);
        assert_eq!(out.text.as_ref(), "ab\u{FFFD}cd");
    }

    #[test]
    fn bom_plus_crlf_plus_sentinel_all_applied() {
        let input = "\u{FEFF}hello\r\n\u{E003}world";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "hello\n\u{FFFD}world");
        assert_eq!(out.diagnostics.len(), 1);
        assert!(!out.text.contains('\u{E003}'));
    }

    #[test]
    fn empty_input_produces_empty_output() {
        let out = sanitize("");
        assert!(out.text.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn bom_only_input_produces_empty_output() {
        let out = sanitize("\u{FEFF}");
        assert!(out.text.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn pure_japanese_is_not_accent_rewritten_and_stays_borrowed() {
        let input = "これはただの日本語の文章です。";
        let out = sanitize(input);
        assert!(matches!(out.text, Cow::Borrowed(_)));
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn plain_commonmark_without_tortoiseshell_stays_borrowed() {
        let input = "# heading\n\nParagraph with `code` and *emph*.\n";
        let out = sanitize(input);
        assert!(matches!(out.text, Cow::Borrowed(_)));
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn accent_digraph_inside_tortoiseshell_is_decomposed() {
        let input = "〔oraison fune`bre〕";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "〔oraison funèbre〕");
        assert!(!out.text.contains('`'));
    }

    #[test]
    fn tortoiseshell_brackets_are_preserved_after_decomposition() {
        let input = "〔Où〕";
        let out = sanitize(input);
        assert!(out.text.contains('〔'));
        assert!(out.text.contains('〕'));
    }

    #[test]
    fn text_outside_tortoiseshell_spans_is_not_decomposed() {
        let input = "text, 〔cafe'〕, rest";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "text, 〔café〕, rest");
        assert!(out.text.starts_with("text,"));
    }

    #[test]
    fn multiple_tortoiseshell_spans_are_each_rewritten() {
        let input = "前〔a`〕中〔e'〕後";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "前〔à〕中〔é〕後");
    }

    #[test]
    fn unclosed_tortoiseshell_span_passes_through_verbatim() {
        let input = "tail 〔fune`bre without close";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn empty_tortoiseshell_span_is_idempotent() {
        let input = "〔〕 empty";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn nested_tortoiseshell_honours_outer_then_inner() {
        let input = "〔outer 〔inner`〕〕";
        let out = sanitize(input);
        assert!(out.text.contains('〔'));
        assert!(out.text.contains('〕'));
    }

    #[test]
    fn tortoiseshell_plus_crlf_plus_bom_all_applied() {
        let input = "\u{FEFF}〔fune`\r\nbre〕end";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "〔funè\nbre〕end");
        assert!(!out.text.contains('`'), "grave accent must be consumed");
    }

    #[test]
    fn tortoiseshell_does_not_interact_with_pua_sentinel_scan() {
        let input = "〔a\u{E001}b〕";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.text.as_ref(), "〔a\u{FFFD}b〕");
        assert!(!out.text.contains('\u{E001}'));
    }

    #[test]
    fn long_hyphen_rule_gets_blank_line_before_it() {
        let input = "前置き\n-----------\n本文";
        let out = sanitize(input);
        assert!(
            out.text.contains("前置き\n\n-----------"),
            "expected blank line inserted; got {:?}",
            out.text
        );
    }

    #[test]
    fn long_equals_rule_gets_blank_line_before_it() {
        let input = "前置き\n===============\n本文";
        let out = sanitize(input);
        assert!(
            out.text.contains("前置き\n\n==============="),
            "expected blank line before long-equals rule; got {:?}",
            out.text
        );
    }

    #[test]
    fn long_underscore_rule_gets_blank_line_before_it() {
        let input = "前置き\n____________\n本文";
        let out = sanitize(input);
        assert!(
            out.text.contains("前置き\n\n____________"),
            "expected blank line before long-underscore rule; got {:?}",
            out.text
        );
    }

    #[test]
    fn short_hyphen_setext_underline_is_not_split() {
        let input = "Heading\n---\nbody";
        let out = sanitize(input);
        assert_eq!(
            out.text.as_ref(),
            input,
            "short setext underline must not gain a blank line"
        );
    }

    #[test]
    fn nine_char_hyphen_row_stays_as_setext_underline() {
        let input = "Heading\n---------\nbody";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn ten_char_hyphen_row_is_isolated() {
        let input = "Heading\n----------\nbody";
        let out = sanitize(input);
        assert!(
            out.text.contains("Heading\n\n----------"),
            "expected 10-char rule to be isolated; got {:?}",
            out.text
        );
    }

    #[test]
    fn rule_already_preceded_by_blank_line_is_unchanged() {
        let input = "前置き\n\n-----------\n本文";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn document_without_any_rule_stays_borrowed() {
        let input = "plain paragraph\n\nsecond paragraph";
        let out = sanitize(input);
        assert!(
            matches!(out.text, Cow::Borrowed(_)),
            "documents without a long rule must pass through borrowed"
        );
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn rule_at_document_start_is_unchanged() {
        let input = "-----------\n本文";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn mixed_character_rule_is_not_isolated() {
        let input = "text\n---===---\ntail";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn consecutive_rule_rows_each_get_isolated() {
        let input = "前置き\n----------\n==========\n本文";
        let out = sanitize(input);
        assert_eq!(
            out.text.as_ref(),
            "前置き\n\n----------\n\n==========\n本文"
        );
    }

    #[test]
    fn aozora_style_long_rule_fixture_shape() {
        let rule: String = "-".repeat(55);
        let input = format!("凡例です。\n{rule}\n本文");
        let out = sanitize(&input);
        let expected = format!("凡例です。\n\n{rule}\n本文");
        assert_eq!(out.text.as_ref(), expected);
    }

    #[test]
    fn every_backtick_inside_vowel_span_collapses() {
        for base in ['a', 'e', 'i', 'o', 'u'] {
            let input = format!("〔x{base}`y〕");
            let out = sanitize(&input);
            assert!(
                !out.text.contains('`'),
                "backtick survived for base {base:?}: {:?}",
                out.text
            );
        }
    }

    #[test]
    fn rewrite_accent_spans_direct_call_returns_decomposed_body() {
        assert_eq!(
            rewrite_accent_spans("〔oraison fune`bre〕"),
            "〔oraison funèbre〕"
        );
        assert_eq!(rewrite_accent_spans("plain text"), "plain text");
    }

    #[test]
    fn tail_rule_line_without_trailing_newline_is_isolated() {
        let input = "前置き\n----------";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "前置き\n\n----------");
    }
}
