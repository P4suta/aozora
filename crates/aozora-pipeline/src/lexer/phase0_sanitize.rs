//! Phase 0 — source sanitation.
//!
//! Prepares the raw source text for the downstream lexer phases:
//!
//! 1. **BOM strip** — every leading `U+FEFF` (UTF-8 BOM, 3 bytes each)
//!    is consumed. Both single (`U+FEFF`) and stacked (`U+FEFF`+
//!    `U+FEFF`+…) leading sequences resolve to the same empty prefix
//!    so that `serialize(serialize(x))` round-trips byte-equal — a
//!    single-strip would peel off one BOM per pass and break I3
//!    fixed-point on inputs that carry more than one. Interior
//!    `U+FEFF` (zero-width no-break space) is still preserved.
//! 2. **CR/LF normalization** — `\r\n` → `\n`, lone `\r` → `\n`. Aozora
//!    source comes from a variety of encoders; downstream phases assume
//!    `\n` as the one line terminator so they don't have to handle three
//!    variants each.
//! 3. **Accent decomposition inside `〔...〕`** — ASCII accent digraphs
//!    (`fune`+grave-accent → funèbre, `cafe`+apostrophe → café, …) are
//!    rewritten to their Unicode-combined form before any later phase
//!    sees them. Scope is deliberately restricted to tortoiseshell-
//!    bracket spans; the function is the identity outside them.
//! 4. **Decorative rule isolation** — lines composed entirely of 10 or
//!    more `-`, `=`, or `_` characters (a very common visual separator
//!    in Aozora Bunko prose) are forced to sit on their own stanza by
//!    inserting a blank line before them, so downstream Markdown
//!    layers (e.g. the sibling `afm` repo's CommonMark integration)
//!    do not promote the preceding paragraph into a setext heading.
//! 5. **PUA sentinel collision scan** — the lexer will shortly inject
//!    [`crate::INLINE_SENTINEL`] / [`crate::BLOCK_LEAF_SENTINEL`] /
//!    [`crate::BLOCK_OPEN_SENTINEL`] / [`crate::BLOCK_CLOSE_SENTINEL`] into
//!    the normalized text (Phase 4). If the source already uses any of
//!    those codepoints, post-process splice can't tell source from marker.
//!    This phase emits a [`aozora_spec::Diagnostic::SourceContainsPua`] for
//!    each occurrence so the problem surfaces, while still passing the
//!    text through verbatim. A future enhancement can switch to
//!    Unicode-noncharacter sentinels when a collision is detected.
//!
//! The sanitize pass is a pure function: `fn(&str) -> SanitizeOutput<'_>`.
//! The output borrows the input when no transformation fires and owns a
//! normalized copy otherwise.

use std::borrow::Cow;

use memchr::memmem;

use aozora_syntax::Span;
use aozora_syntax::accent::decompose_fragment;

use crate::{BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL};
use aozora_spec::Diagnostic;

/// Tortoiseshell-bracket open character — delimits accent-decomposition
/// spans.
const TORTOISE_OPEN: char = '〔';
/// UTF-8 byte encoding of [`TORTOISE_OPEN`] for `memmem`-based scans.
/// `'〔'` (U+3014) → `0xE3 0x80 0x94`.
const TORTOISE_OPEN_BYTES: &[u8] = "〔".as_bytes();
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
    // Strip every leading `U+FEFF`. CommonMark / WHATWG-text-encoding
    // both consider only one BOM, but the `serialize` round-trip would
    // peel one off per pass without this loop, breaking the I3
    // fixed-point invariant `serialize(serialize(x)) == serialize(x)`
    // on inputs that carry stacked BOMs (e.g. `\u{feff}\u{feff}` →
    // first pass yields `\u{feff}`, second yields `""`).
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

    // Gate via `memmem::find` on the UTF-8 byte sequence rather than
    // `str::contains(char)`, which falls back to a per-codepoint
    // scan via `Pattern::is_contained_in` and pays full UTF-8 decode
    // cost on every char of the input. memmem uses Two-Way / SIMD on
    // the 3-byte needle and zooms through Japanese prose at memory-
    // bandwidth speed.
    let text: Cow<'_, str> =
        if memmem::find(rule_isolated.as_bytes(), TORTOISE_OPEN_BYTES).is_some() {
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
#[doc(hidden)]
#[must_use]
pub fn rewrite_accent_spans(input: &str) -> String {
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
#[doc(hidden)]
pub fn has_long_rule_line(input: &str) -> bool {
    input.lines().any(is_decorative_rule_line)
}

/// Return `true` when `line` is composed of ≥ `DECORATIVE_RULE_MIN_LEN`
/// repeats of a single `-` / `=` / `_` character with no other content
/// (surrounding whitespace is tolerated to match real-world formatting).
fn is_decorative_rule_line(line: &str) -> bool {
    is_rule_line_trimmed(line.trim())
}

/// Byte-level rule-line check on a string the caller has already
/// trimmed. Used by [`isolate_decorative_rules`] which also needs
/// the trimmed length for the blank-line bookkeeping — sharing the
/// trim avoids the duplicate work the prior split called for.
///
/// `-` / `=` / `_` are ASCII single-byte characters, so the
/// `bytes().all(...)` comparison is a `memcmp`-class scan. For lines
/// whose first byte is multi-byte UTF-8 (every Japanese line in the
/// corpus, the dominant case) the leading `matches!` check rejects
/// in 2–3 ops and the rest of the function is skipped entirely.
fn is_rule_line_trimmed(trimmed: &str) -> bool {
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

/// Insert a blank line before every decorative rule that would
/// otherwise be interpreted by CommonMark as a setext underline for
/// the preceding paragraph. The output differs from the input *only*
/// in the blank lines inserted.
///
/// ## Algorithm
///
/// `memchr::memchr_iter(b'\n', ...)` walks every newline position via
/// SIMD byte scan. For each line we run [`is_decorative_rule_line`]
/// (which exits in O(1) when the trimmed first char isn't `-=_`,
/// covering ≥99% of Aozora lines). Only when a rule line needs an
/// inserted blank line does the algorithm break the running bulk-copy
/// to flush `[copy_from..line_start)` and emit a `\n`.
///
/// Replaces a previous `for line in input.split_inclusive('\n')` /
/// `out.push_str(line)` loop that paid one `push_str` (one `memcpy`)
/// per line. Real Aozora corpora have ~10⁴ short lines per document
/// and typically only 1–5 rule line insertions, so the new path
/// collapses ~10⁴ small `memcpy`s into a small handful of large ones.
#[doc(hidden)]
#[must_use]
pub fn isolate_decorative_rules(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len() + 16);
    let mut line_start: usize = 0;
    let mut copy_from: usize = 0;
    let mut prev_nonblank = false;

    for nl_pos in memchr::memchr_iter(b'\n', bytes) {
        let line_no_eol = &input[line_start..nl_pos];
        // Single trim per line: feed the result to both the rule check
        // and the blank-line bookkeeping. Avoids the double `.trim()`
        // the prior implementation paid on every line.
        let trimmed = line_no_eol.trim();
        if is_rule_line_trimmed(trimmed) && prev_nonblank {
            // Flush the bulk-copy run up to (but not including) this
            // rule line, then inject the separating blank line. The
            // rule line itself stays in the next bulk-copy chunk.
            out.push_str(&input[copy_from..line_start]);
            out.push('\n');
            copy_from = line_start;
        }
        // A rule line (or any visible line) keeps `prev_nonblank` true;
        // an empty / whitespace-only line flips it false so the next
        // rule line does not trigger another spurious insertion.
        prev_nonblank = !trimmed.is_empty();
        line_start = nl_pos + 1;
    }
    // Final tail line (no trailing `\n`). Mirrors the per-line check.
    if line_start < bytes.len() {
        let tail = &input[line_start..];
        let tail_trimmed = tail.trim();
        if is_rule_line_trimmed(tail_trimmed) && prev_nonblank {
            out.push_str(&input[copy_from..line_start]);
            out.push('\n');
            copy_from = line_start;
        }
    }
    // Single closing flush emits the unmodified tail of the input
    // verbatim. Typical corpus documents take this path with
    // `copy_from == 0` and one big `push_str` of the whole buffer.
    if copy_from < bytes.len() {
        out.push_str(&input[copy_from..]);
    }
    out
}

/// Normalise line endings: every `\r\n` and every standalone `\r`
/// collapses to a single `\n`.
///
/// ## Algorithm
///
/// `memchr::memchr_iter(b'\r', ...)` walks every `\r` position in the
/// input via SIMD-accelerated byte scan, bulk-copying the inter-`\r`
/// runs through `push_str` (one `memcpy` per chunk). At each hit a
/// single-byte lookahead distinguishes `\r\n` (skip both, emit `\n`)
/// from a lone `\r` (skip the `\r`, emit `\n`).
///
/// One pass over the input, one buffer allocation. Replaces the prior
/// `.replace("\r\n", "\n").replace('\r', "\n")` pair which materialised
/// **two** intermediate `String`s and walked the input twice. On the
/// 17 k-document Aozora corpus — where every document arrives with
/// CRLF line endings (the archive's house format) — this sub-pass is
/// the dominant cost in phase 0; the single-pass form is ~2–3× faster
/// at memory-bandwidth ceiling.
///
/// `\r` (0x0D) is ASCII so `memchr` lands cleanly on UTF-8 boundaries;
/// no need for `is_char_boundary` checks.
#[doc(hidden)]
#[must_use]
pub fn normalize_line_endings(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    for cr_pos in memchr::memchr_iter(b'\r', bytes) {
        // Bulk-copy the run between the previous cursor and this `\r`.
        // `push_str` lowers to a single `memcpy` when the chunk is
        // contiguous and non-empty.
        if cr_pos > cursor {
            out.push_str(&input[cursor..cr_pos]);
        }
        // Always emit one `\n` for the line terminator. Skip the
        // following `\n` if this is `\r\n` (the CRLF path); otherwise
        // step past the lone `\r` only.
        out.push('\n');
        cursor = if bytes.get(cr_pos + 1) == Some(&b'\n') {
            cr_pos + 2
        } else {
            cr_pos + 1
        };
    }
    if cursor < bytes.len() {
        out.push_str(&input[cursor..]);
    }
    out
}

/// Scan `text` for source-side occurrences of any of the four PUA
/// sentinel codepoints (`U+E001..U+E004`), emitting one diagnostic
/// per hit.
///
/// ## Algorithm
///
/// All four sentinel codepoints encode to the same 2-byte UTF-8
/// prefix `EE 80`, with the third byte distinguishing them
/// (`81 .. 84`). The leading byte `0xEE` itself only appears at the
/// start of codepoints in `U+E000..U+EFFF` — Private Use Area + a
/// chunk of Hangul Jamo Extended-B. In real Japanese text these are
/// vanishingly rare, so a SIMD-friendly `memchr(0xEE)` scan zooms
/// through the source at memory-bandwidth speed and only pays per-
/// candidate validation cost on actual hits.
///
/// The byte-level scan runs at ~580 MB/s on the corpus profile, vs
/// ~75 MB/s for a character-by-character `text.chars()` walk that
/// ran the predicate on every codepoint.
#[doc(hidden)]
#[must_use]
pub fn scan_for_sentinel_collisions(text: &str) -> Vec<Diagnostic> {
    let bytes = text.as_bytes();
    let mut diagnostics = Vec::new();
    for cand in memchr::memchr_iter(0xEE, bytes) {
        // Must have 2 trailing bytes for a complete 3-byte codepoint.
        if cand + 3 > bytes.len() {
            continue;
        }
        // U+E001..U+E004 ↔ EE 80 81..84.
        if bytes[cand + 1] != 0x80 {
            continue;
        }
        let third = bytes[cand + 2];
        let codepoint = match third {
            0x81 => INLINE_SENTINEL,
            0x82 => BLOCK_LEAF_SENTINEL,
            0x83 => BLOCK_OPEN_SENTINEL,
            0x84 => BLOCK_CLOSE_SENTINEL,
            _ => continue,
        };
        // `memchr_iter` only walks in-bounds; cand and cand+3 fit u32
        // because sanitize asserts source.len() <= u32::MAX upstream.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "source.len() <= u32::MAX is asserted at sanitize entry"
        )]
        let abs_start = cand as u32;
        diagnostics.push(Diagnostic::source_contains_pua(
            Span::new(abs_start, abs_start + 3),
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
        // Only a *leading* BOM gets stripped; interior U+FEFF is left as
        // zero-width no-break space (the other meaning of the codepoint).
        assert_eq!(out.text.as_ref(), input);
    }

    #[test]
    fn stacked_leading_boms_are_all_stripped() {
        // I3 fixed-point regression: `serialize(serialize(x))` must
        // byte-equal `serialize(x)`. Stacked leading BOMs would
        // otherwise peel off one per round-trip pass, so the strip
        // loop has to consume every leading `U+FEFF`.
        let input = "\u{FEFF}\u{FEFF}\u{FEFF}hello";
        let out = sanitize(input);
        assert_eq!(out.text.as_ref(), "hello");
    }

    #[test]
    fn leading_boms_only_resolve_to_empty() {
        // Edge case: an input that is *nothing but* leading BOMs
        // resolves to the empty string. The previous single-strip
        // behaviour produced `""` for one BOM and `"\u{feff}"` for
        // two — the source of the I3 fuzz crash.
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
    // Accent-decomposition inside 〔...〕.
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
        // to `è` inside the span so the parser never sees the lone backtick.
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
        // unmodified so the H1/H2 promotion still fires.
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
        // separate them so the paragraph reaches the parser as a
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
