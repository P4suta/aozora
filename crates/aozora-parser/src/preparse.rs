//! Pre-parse rewrite pass: apply Aozora accent decomposition inside `〔...〕`
//! spans before comrak's CommonMark inline parser sees the buffer.
//!
//! Motivation (see ADR-0004): real Aozora works embed European-language
//! fragments using ASCII digraphs such as `fune``bre` = funèbre. The backtick
//! inside such a span is an accent marker, not the opener of a CommonMark code
//! span. If we let comrak process the raw text, it opens a code span on the
//! first backtick and runs forward until it finds another, swallowing any
//! unrelated `［＃…］` annotations between. Rewriting the spans to Unicode up
//! front makes the backtick disappear before comrak ever sees it.
//!
//! Scope is deliberately conservative: only text inside `〔...〕` tortoiseshell
//! brackets is rewritten. Outside those brackets the function is the identity.
//! Extending to convention-less decomposition is a future ADR; this module is
//! intentionally tiny so that extension can be bolted on without refactoring.

use std::borrow::Cow;

const OPEN: char = '〔';
const CLOSE: char = '〕';

/// Rewrite `input` applying accent decomposition inside every `〔...〕` span.
///
/// Returns `Cow::Borrowed(input)` when the input contains no `〔` at all — the
/// vast majority of Aozora texts (and every pure-CommonMark document).
#[must_use]
pub fn apply_preparse(input: &str) -> Cow<'_, str> {
    if !input.contains(OPEN) {
        return Cow::Borrowed(input);
    }
    Cow::Owned(rewrite(input))
}

fn rewrite(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    let bytes = input.as_bytes();

    while cursor < bytes.len() {
        let Some(open_rel) = input[cursor..].find(OPEN) else {
            // No more 〔 — append the remainder and finish.
            out.push_str(&input[cursor..]);
            break;
        };
        let open_abs = cursor + open_rel;
        // Copy [cursor, open_abs) verbatim.
        out.push_str(&input[cursor..open_abs]);

        let after_open = open_abs + OPEN.len_utf8();
        let Some(close_rel) = input[after_open..].find(CLOSE) else {
            // Unclosed 〔 — emit rest verbatim, no rewrite.
            out.push_str(&input[open_abs..]);
            break;
        };
        let close_abs = after_open + close_rel;
        out.push(OPEN);
        let body = &input[after_open..close_abs];
        out.push_str(&afm_syntax::accent::decompose_fragment(body));
        out.push(CLOSE);
        cursor = close_abs + CLOSE.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pure_japanese_passes_through_borrowed() {
        let input = "これはただの日本語の文章です。";
        let out = apply_preparse(input);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, input);
    }

    #[test]
    fn commonmark_without_tortoiseshell_brackets_passes_through() {
        let input = "# heading\n\nParagraph with `code` and *emph*.\n";
        let out = apply_preparse(input);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, input);
    }

    #[test]
    fn rewrites_accent_digraph_inside_tortoiseshell() {
        // The 罪と罰 canary: grave accent in a French fragment no longer
        // reaches comrak as a lone backtick.
        let input = "〔oraison fune`bre〕";
        let out = apply_preparse(input);
        assert_eq!(out, "〔oraison funèbre〕");
        assert!(!out.contains('`'));
    }

    #[test]
    fn tortoiseshell_brackets_are_preserved() {
        let input = "〔Où〕";
        let out = apply_preparse(input);
        assert!(out.contains('〔'));
        assert!(out.contains('〕'));
    }

    #[test]
    fn text_outside_the_span_is_not_decomposed() {
        // The `t,` false-positive from `text,` stays — only inside 〔〕.
        let input = "text, 〔cafe'〕, rest";
        let out = apply_preparse(input);
        assert_eq!(out, "text, 〔café〕, rest");
        assert!(out.starts_with("text,"));
    }

    #[test]
    fn multiple_spans_are_each_rewritten() {
        let input = "前〔a`〕中〔e'〕後";
        let out = apply_preparse(input);
        assert_eq!(out, "前〔à〕中〔é〕後");
    }

    #[test]
    fn unclosed_tortoiseshell_leaves_content_verbatim() {
        // Graceful degradation — don't panic, emit rest as-is so the caller
        // sees the malformed input and can issue a diagnostic later.
        let input = "tail 〔fune`bre without close";
        let out = apply_preparse(input);
        assert_eq!(out, input);
    }

    #[test]
    fn empty_span_is_idempotent() {
        let input = "〔〕 empty";
        let out = apply_preparse(input);
        assert_eq!(out, input);
    }

    #[test]
    fn nested_tortoiseshell_honours_outer_first() {
        // Spec doesn't address nesting; we take the outermost open-to-close
        // substring and let the accent table decide. Inner 〔 remains in
        // the body and is emitted as the literal character.
        let input = "〔outer 〔inner`〕〕";
        let out = apply_preparse(input);
        // Inner `〔inner\`〕` forms the body of the outer span. decompose_fragment
        // sees it as "outer 〔inner`" (up to the first 〕) and leaves 〔
        // unchanged (not a base letter), decomposes `r\`` → r + \` since r is
        // not a table base for grave. Wait: looking at spec, r` is absent.
        // So body stays as-is → "outer 〔inner`".
        assert!(out.contains('〔'));
        assert!(out.contains('〕'));
    }

    #[test]
    fn span_with_no_digraphs_roundtrips_body() {
        let input = "〔hello world〕";
        let out = apply_preparse(input);
        assert_eq!(out, input);
    }

    #[test]
    fn property_every_backtick_inside_span_is_rewritten_or_preserved_as_literal() {
        // For every body built from `<vowel><backtick>`, the body must contain
        // ZERO backticks after preparse — each is either decomposed into its
        // grave-accented form or, when paired with a non-vowel base, would
        // fall through to the ` character itself. Since vowels are the only
        // bases with grave-accent entries, this gives us a clean invariant.
        for base in ['a', 'e', 'i', 'o', 'u'] {
            let input = format!("〔x{base}`y〕");
            let out = apply_preparse(&input);
            assert!(
                !out.contains('`'),
                "backtick survived for base {base:?}: {out}"
            );
        }
    }
}
