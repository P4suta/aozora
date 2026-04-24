//! Phase 0 — source sanitation.
//!
//! Prepares the raw source text for the downstream lexer phases:
//!
//! 1. **BOM strip** — a leading `U+FEFF` (UTF-8 BOM, 3 bytes) is consumed.
//! 2. **CR/LF normalization** — `\r\n` → `\n`, lone `\r` → `\n`. Aozora
//!    source comes from a variety of encoders; downstream phases assume
//!    `\n` as the one line terminator so they don't have to handle three
//!    variants each.
//! 3. **Accent decomposition inside `〔...〕`** — ASCII accent digraphs
//!    (`fune`+grave-accent → funèbre, `cafe`+apostrophe → café, …) are
//!    rewritten to their Unicode-combined form before any later phase
//!    sees them. ADR-0004 motivates this: comrak's CommonMark inline
//!    parser would otherwise open a code span on a bare backtick and
//!    swallow adjacent `［＃…］` annotations. Scope is deliberately
//!    restricted to tortoiseshell-bracket spans; the function is the
//!    identity outside them.
//! 4. **Decorative rule isolation** — lines composed entirely of 10 or
//!    more `-`, `=`, or `_` characters (a very common visual separator
//!    in Aozora Bunko prose) are forced to sit on their own stanza by
//!    inserting a blank line before them. This defeats CommonMark's
//!    setext-heading rule (paragraph followed by `---` → H2), which
//!    would otherwise turn the preceding prose into a spurious heading
//!    whenever an Aozora text drops a `---...` row right after the
//!    front-matter. The intervention is semantics-preserving for
//!    CommonMark: inserting a blank line before what would otherwise
//!    be a setext underline yields the same AST as if the author had
//!    written the blank line themselves. Short (≤ 9 char) `---` /
//!    `===` runs are untouched so the genuine setext-heading idiom
//!    keeps working verbatim.
//! 5. **PUA sentinel collision scan** — the lexer will shortly inject
//!    [`crate::INLINE_SENTINEL`] / [`crate::BLOCK_LEAF_SENTINEL`] /
//!    [`crate::BLOCK_OPEN_SENTINEL`] / [`crate::BLOCK_CLOSE_SENTINEL`] into
//!    the normalized text (Phase 4). If the source already uses any of
//!    those codepoints, post-comrak splice can't tell source from marker.
//!    This phase emits a [`crate::Diagnostic::SourceContainsPua`] for
//!    each occurrence so the problem surfaces, while still passing the
//!    text through verbatim. A future enhancement can switch to
//!    Unicode-noncharacter sentinels when a collision is detected.
//!
//! The sanitize pass is a pure function: `fn(&str) -> SanitizeOutput<'_>`.
//! The output borrows the input when no transformation fires and owns a
//! normalized copy otherwise.

use std::borrow::Cow;

use afm_syntax::Span;
use afm_syntax::accent::decompose_fragment;

use crate::diagnostic::Diagnostic;
use crate::{BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL};

/// Tortoiseshell-bracket open character — delimits accent-decomposition
/// spans per ADR-0004.
const TORTOISE_OPEN: char = '〔';
/// Tortoiseshell-bracket close character.
const TORTOISE_CLOSE: char = '〕';

/// Minimum run length for a `-` / `=` / `_` line to be treated as a
/// decorative rule rather than a setext underline. Nine characters is
/// the longest setext underline observed in the CommonMark 0.31.2 spec
/// cases; ten is the first length where Aozora's typical `---...---`
/// separator starts to appear in the 17 k-work corpus.
const DECORATIVE_RULE_MIN_LEN: usize = 10;

/// Output of Phase 0. `text` is what downstream phases consume; `diagnostics`
/// carries any non-fatal observations gathered during sanitation.
#[derive(Debug, Clone)]
pub struct SanitizeOutput<'s> {
    pub text: Cow<'s, str>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Apply the four sanitation steps and return the result. See module
/// documentation for the step order and rationale.
#[must_use]
pub fn sanitize(source: &str) -> SanitizeOutput<'_> {
    let after_bom = source.strip_prefix('\u{FEFF}').unwrap_or(source);

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

    let text: Cow<'_, str> = if rule_isolated.contains(TORTOISE_OPEN) {
        // Move out of the Cow so the rewrite doesn't double-allocate if
        // an earlier pass already owned the buffer.
        let owned = rule_isolated.into_owned();
        Cow::Owned(rewrite_accent_spans(&owned))
    } else {
        rule_isolated
    };

    let diagnostics = scan_for_sentinel_collisions(&text);

    SanitizeOutput { text, diagnostics }
}

/// Rewrite every `〔...〕` span applying accent decomposition to the body.
/// Text outside spans is copied verbatim.
fn rewrite_accent_spans(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while cursor < input.len() {
        let Some(open_rel) = input[cursor..].find(TORTOISE_OPEN) else {
            // No more opens — copy the remainder verbatim and finish.
            out.push_str(&input[cursor..]);
            break;
        };
        let open_abs = cursor + open_rel;
        out.push_str(&input[cursor..open_abs]);

        let after_open = open_abs + TORTOISE_OPEN.len_utf8();
        let Some(close_rel) = input[after_open..].find(TORTOISE_CLOSE) else {
            // Unclosed `〔` — emit the rest verbatim so the author can
            // see the malformed span in the rendered output rather
            // than silently dropping content.
            out.push_str(&input[open_abs..]);
            break;
        };
        let close_abs = after_open + close_rel;

        out.push(TORTOISE_OPEN);
        let body = &input[after_open..close_abs];
        out.push_str(&decompose_fragment(body));
        out.push(TORTOISE_CLOSE);
        cursor = close_abs + TORTOISE_CLOSE.len_utf8();
    }

    out
}

/// Return `true` when at least one line in `input` is a decorative
/// rule (≥ `DECORATIVE_RULE_MIN_LEN` of `-` / `=` / `_`).
///
/// Used as a fast-path gate in [`sanitize`]: when the whole document
/// has no long rule line, the pass is a no-op and [`Cow::Borrowed`]
/// survives.
fn has_long_rule_line(input: &str) -> bool {
    input.lines().any(is_decorative_rule_line)
}

/// Return `true` when `line` is composed of ≥ `DECORATIVE_RULE_MIN_LEN`
/// repeats of a single `-` / `=` / `_` character with no other content
/// (surrounding whitespace is tolerated to match real-world formatting).
fn is_decorative_rule_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < DECORATIVE_RULE_MIN_LEN {
        return false;
    }
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if !matches!(first, '-' | '=' | '_') {
        return false;
    }
    trimmed.chars().all(|c| c == first)
}

/// Insert a blank line before every decorative rule that would
/// otherwise be interpreted by CommonMark as a setext underline for
/// the preceding paragraph. Lines are iterated with their terminating
/// newline preserved (`split_inclusive`), so the output differs from
/// the input *only* in the blank lines inserted.
fn isolate_decorative_rules(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    let mut prev_nonblank = false;
    for line in input.split_inclusive('\n') {
        let without_eol = line.strip_suffix('\n').unwrap_or(line);
        if is_decorative_rule_line(without_eol) && prev_nonblank {
            // Inject a blank line before the rule so the previous
            // paragraph does not fuse with it into a setext heading.
            out.push('\n');
        }
        out.push_str(line);
        // Track whether the line we just emitted was itself visibly
        // non-blank. A rule line counts as non-blank — inserting a
        // blank before the *next* rule still needs the gate to flip
        // to false once a blank line is emitted.
        prev_nonblank = !without_eol.trim().is_empty();
    }
    out
}

/// Replace every `\r\n` with `\n`, then every remaining `\r` with `\n`.
///
/// Done in two passes for clarity rather than a single `replace` with a
/// regex: CRLF must collapse to one LF (not two), which a naive
/// `replace('\r', "\n")` would miss. The two-pass form is also the one
/// the CommonMark spec prescribes (§2.1 Line endings), matching comrak's
/// own internal expectations.
fn normalize_line_endings(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

/// Walk the text character-by-character, emitting one diagnostic per
/// source-side occurrence of any sentinel codepoint.
fn scan_for_sentinel_collisions(text: &str) -> Vec<Diagnostic> {
    let sentinels = [
        INLINE_SENTINEL,
        BLOCK_LEAF_SENTINEL,
        BLOCK_OPEN_SENTINEL,
        BLOCK_CLOSE_SENTINEL,
    ];
    let mut diagnostics = Vec::new();
    let mut byte_pos: u32 = 0;
    for ch in text.chars() {
        // `char::len_utf8` is 1..=4, safely fits u32.
        let len = u32::try_from(ch.len_utf8()).expect("char length is 1..=4 bytes");
        if sentinels.contains(&ch) {
            diagnostics.push(Diagnostic::source_contains_pua(
                Span::new(byte_pos, byte_pos + len),
                ch,
            ));
        }
        byte_pos += len;
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
        // Only a *leading* BOM gets stripped; interior U+FEFF is left as
        // zero-width no-break space (the other meaning of the codepoint).
        assert_eq!(out.text.as_ref(), input);
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
    fn pua_inline_sentinel_emits_one_diagnostic() {
        let input = "plain\u{E001}text";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 1);
        let Diagnostic::SourceContainsPua { codepoint, .. } = &out.diagnostics[0] else {
            panic!("expected SourceContainsPua, got {:?}", out.diagnostics[0]);
        };
        assert_eq!(*codepoint, '\u{E001}');
    }

    #[test]
    fn pua_all_four_sentinels_emit_four_diagnostics() {
        let input = "\u{E001}\u{E002}\u{E003}\u{E004}";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 4);
    }

    #[test]
    fn non_sentinel_pua_codepoints_do_not_emit_diagnostics() {
        // U+E000 is inside PUA but not a sentinel; other PUA codepoints
        // likewise. Only the reserved sentinel set triggers.
        let input = "\u{E000}\u{E100}\u{F8FF}";
        let out = sanitize(input);
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn pua_diagnostic_span_points_at_sentinel_position() {
        let input = "ab\u{E002}cd";
        let out = sanitize(input);
        let Diagnostic::SourceContainsPua { span, .. } = &out.diagnostics[0] else {
            panic!("expected SourceContainsPua, got {:?}", out.diagnostics[0]);
        };
        // 'a','b' each 1 byte; U+E002 is 3 bytes in UTF-8.
        assert_eq!(span.start, 2);
        assert_eq!(span.end, 5);
    }

    #[test]
    fn bom_plus_crlf_plus_sentinel_all_applied() {
        let input = "\u{FEFF}hello\r\n\u{E003}world";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "hello\n\u{E003}world");
        assert_eq!(out.diagnostics.len(), 1);
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

    // -----------------------------------------------------------------
    // Accent-decomposition inside 〔...〕 (ADR-0004).
    // -----------------------------------------------------------------

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
        // The 罪と罰 canary: the grave-accent digraph `e`` must collapse
        // to `è` inside the span so comrak never sees the lone backtick.
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
        // `text,` stays as-is; only `cafe'` inside the span becomes `café`.
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
        // Graceful degradation — don't panic, emit the rest as-is so a
        // later phase can surface a diagnostic.
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
        // Outer span's body is "outer 〔inner`"; decompose_fragment
        // leaves `〔` alone (not a table base) and `inner`` similarly
        // untouched — the exact output shape is documented here so any
        // drift in the accent table surfaces.
        let input = "〔outer 〔inner`〕〕";
        let out = sanitize(input);
        assert!(out.text.contains('〔'));
        assert!(out.text.contains('〕'));
    }

    #[test]
    fn tortoiseshell_plus_crlf_plus_bom_all_applied() {
        // Exercise all three transformation steps in one shot: leading
        // BOM, CRLF inside a span, accent digraph. The BOM is stripped
        // and the CRLF becomes LF before accent decomposition runs —
        // decomposition then matches `e``on the `e` side of the LF,
        // producing `è` and leaving the LF as the next char.
        let input = "\u{FEFF}〔fune`\r\nbre〕end";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "〔funè\nbre〕end");
        assert!(!out.text.contains('`'), "grave accent must be consumed");
    }

    #[test]
    fn tortoiseshell_does_not_interact_with_pua_sentinel_scan() {
        // PUA scan runs on the accent-decomposed text, so a sentinel
        // appearing inside a `〔...〕` span is still caught.
        let input = "〔a\u{E001}b〕";
        let out = sanitize(input);
        assert_eq!(out.diagnostics.len(), 1);
    }

    // -------------------------------------------------------------
    // Decorative rule isolation — long `-` / `=` / `_` rows must not
    // be misread as setext underlines for a preceding paragraph.
    //
    // Background: Aozora Bunko prose frequently inserts
    // `---------------------------------------------------------`
    // as a visual separator between front matter and body. Without
    // this pass, CommonMark would swallow the front-matter paragraph
    // into an H2. These tests pin both halves of the contract — long
    // runs are isolated, short runs (the genuine setext idiom) are
    // untouched — so future refactors cannot silently regress either
    // direction.
    // -------------------------------------------------------------

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
        // The genuine setext-heading idiom uses `---` or `===` rows
        // of modest length (typically < 10 chars). Those must reach
        // comrak unmodified so the H1/H2 promotion still fires.
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
        // Nine characters: still inside the setext-heading length
        // range per our DECORATIVE_RULE_MIN_LEN threshold.
        let input = "Heading\n---------\nbody";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn ten_char_hyphen_row_is_isolated() {
        // Ten characters — the first length at which we classify the
        // row as decorative rather than setext.
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
        // Idempotence: if the author already put a blank line before
        // the rule, we must not add a second.
        let input = "前置き\n\n-----------\n本文";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn document_without_any_rule_stays_borrowed() {
        // The fast-path gate (`has_long_rule_line`) must keep the
        // common case allocation-free.
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
        // With no preceding non-blank line, the setext-heading
        // confusion cannot arise — no blank line needed.
        let input = "-----------\n本文";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn mixed_character_rule_is_not_isolated() {
        // `---===---` is neither a valid setext underline nor a
        // homogeneous rule; leave it alone so CommonMark handles it
        // as a plain paragraph line.
        let input = "text\n---===---\ntail";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn consecutive_rule_rows_each_get_isolated() {
        // Author stacks two rules back-to-back for a thick border.
        // Current policy isolates every decorative rule uniformly;
        // the extra blank line between two rules is a no-op in
        // CommonMark (both become `<hr>` regardless), so the simpler
        // uniform behaviour is preferred over a conditional that
        // special-cases rule-after-rule. Test documents the shape so
        // a future tightening that skips the second isolation has to
        // update this expectation deliberately.
        let input = "前置き\n----------\n==========\n本文";
        let out = sanitize(input);
        assert_eq!(
            out.text.as_ref(),
            "前置き\n\n----------\n\n==========\n本文"
        );
    }

    #[test]
    fn aozora_style_long_rule_fixture_shape() {
        // Direct analogue of the `spec/aozora/fixtures/56656/input.utf8.txt`
        // front-matter: a prose paragraph (here condensed) immediately
        // followed by a 55-char `-` row. The promotion would otherwise
        // turn the prose into a setext H2; the isolation pass must
        // separate them so the paragraph reaches comrak as a
        // paragraph.
        let rule: String = "-".repeat(55);
        let input = format!("凡例です。\n{rule}\n本文");
        let out = sanitize(&input);
        let expected = format!("凡例です。\n\n{rule}\n本文");
        assert_eq!(out.text.as_ref(), expected);
    }

    #[test]
    fn every_backtick_inside_vowel_span_collapses() {
        // Every vowel base + grave accent digraph has a table entry,
        // so no backtick survives inside a `〔<vowel>`〕` span.
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
}
