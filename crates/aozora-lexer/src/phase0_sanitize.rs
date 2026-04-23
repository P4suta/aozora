//! Phase 0 — source sanitation.
//!
//! Prepares the raw source text for the downstream lexer phases:
//!
//! 1. **BOM strip** — a leading `U+FEFF` (UTF-8 BOM, 3 bytes) is consumed.
//! 2. **CR/LF normalization** — `\r\n` → `\n`, lone `\r` → `\n`. Aozora
//!    source comes from a variety of encoders; downstream phases assume
//!    `\n` as the one line terminator so they don't have to handle three
//!    variants each.
//! 3. **PUA sentinel collision scan** — the lexer will shortly inject
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
//! The output borrows the input when no CR is present (the common case)
//! and owns a normalized copy otherwise.

use std::borrow::Cow;

use afm_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::{BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL};

/// Output of Phase 0. `text` is what downstream phases consume; `diagnostics`
/// carries any non-fatal observations gathered during sanitation.
#[derive(Debug, Clone)]
pub struct SanitizeOutput<'s> {
    pub text: Cow<'s, str>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Apply the three sanitation steps and return the result. See module
/// documentation for the step order and rationale.
#[must_use]
pub fn sanitize(source: &str) -> SanitizeOutput<'_> {
    let after_bom = source.strip_prefix('\u{FEFF}').unwrap_or(source);

    let text: Cow<'_, str> = if after_bom.contains('\r') {
        Cow::Owned(normalize_line_endings(after_bom))
    } else {
        Cow::Borrowed(after_bom)
    };

    let diagnostics = scan_for_sentinel_collisions(&text);

    SanitizeOutput { text, diagnostics }
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
        match &out.diagnostics[0] {
            Diagnostic::SourceContainsPua { codepoint, .. } => {
                assert_eq!(*codepoint, '\u{E001}');
            }
        }
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
        let Diagnostic::SourceContainsPua { span, .. } = &out.diagnostics[0];
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
}
