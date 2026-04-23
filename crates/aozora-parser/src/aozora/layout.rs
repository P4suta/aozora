//! Leaf-form layout annotations: 字下げ / 地付き / 地から N 字上げ.
//!
//! These are single-bracket markers that don't pair with a closing annotation
//! — each bracket fully specifies its effect and applies to the logical line
//! it appears on. The paired `［＃ここから字下げ］…［＃ここで字下げ終わり］`
//! family is a separate parse path that lands with the paired-block
//! container hook in Phase D.
//!
//! # Forms handled
//!
//! | Body                 | Variant                     |
//! |----------------------|-----------------------------|
//! | `{N}字下げ`          | [`Indent { amount: N }`]    |
//! | `地付き`             | [`AlignEnd { offset: 0 }`]  |
//! | `地から{N}字上げ`    | [`AlignEnd { offset: N }`]  |
//!
//! `N` is a single digit 1–9, either ASCII (`2`) or full-width (`２`).
//! Aozora Bunko uses full-width in practice; we accept both for convenience.
//! Multi-digit indents would need a wider parser and land when real corpora
//! demand it.

use afm_syntax::{AlignEnd, Indent};

/// Parse the body of a leaf indent annotation. Returns `None` if the body
/// isn't a `{N}字下げ` form.
#[must_use]
pub(crate) fn parse_indent(body: &str) -> Option<Indent> {
    let digits = body.strip_suffix("字下げ")?;
    let amount = parse_single_digit(digits)?;
    Some(Indent { amount })
}

/// Parse the body of a leaf align-end annotation. Accepts `地付き`
/// (offset = 0) and `地から{N}字上げ` (offset = N).
#[must_use]
pub(crate) fn parse_align_end(body: &str) -> Option<AlignEnd> {
    if body == "地付き" {
        return Some(AlignEnd { offset: 0 });
    }
    let inner = body.strip_prefix("地から")?.strip_suffix("字上げ")?;
    let offset = parse_single_digit(inner)?;
    Some(AlignEnd { offset })
}

/// Accept exactly one digit character (ASCII `0–9` or full-width `０–９`)
/// and return its numeric value. Returns `None` for zero, empty, multi-char,
/// or non-digit bodies — all of which are spec-ambiguous as indent amounts.
fn parse_single_digit(s: &str) -> Option<u8> {
    let mut chars = s.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let value = match ch {
        '0'..='9' => u32::from(ch) - u32::from('0'),
        '０'..='９' => u32::from(ch) - u32::from('０'),
        _ => return None,
    };
    if value == 0 {
        return None;
    }
    u8::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fullwidth_indent_digit() {
        assert_eq!(parse_indent("２字下げ"), Some(Indent { amount: 2 }));
        assert_eq!(parse_indent("７字下げ"), Some(Indent { amount: 7 }));
    }

    #[test]
    fn parses_ascii_indent_digit() {
        assert_eq!(parse_indent("2字下げ"), Some(Indent { amount: 2 }));
    }

    #[test]
    fn rejects_indent_with_zero_or_missing_digit() {
        assert!(parse_indent("字下げ").is_none());
        assert!(parse_indent("０字下げ").is_none());
        assert!(parse_indent("0字下げ").is_none());
    }

    #[test]
    fn rejects_indent_with_multidigit() {
        // Multi-digit (10+) indent requires a future parser extension; today
        // the leaf form is single-digit only. A passing test would hide the
        // narrow scope from future maintainers.
        assert!(parse_indent("12字下げ").is_none());
        assert!(parse_indent("１２字下げ").is_none());
    }

    #[test]
    fn rejects_indent_non_digit() {
        assert!(parse_indent("a字下げ").is_none());
        assert!(parse_indent("二字下げ").is_none()); // kanji numeral not accepted yet
    }

    #[test]
    fn parses_jitsuki_zero_offset() {
        assert_eq!(parse_align_end("地付き"), Some(AlignEnd { offset: 0 }));
    }

    #[test]
    fn parses_chi_kara_n_ji_age() {
        assert_eq!(
            parse_align_end("地から２字上げ"),
            Some(AlignEnd { offset: 2 })
        );
        assert_eq!(
            parse_align_end("地から９字上げ"),
            Some(AlignEnd { offset: 9 })
        );
        assert_eq!(
            parse_align_end("地から2字上げ"),
            Some(AlignEnd { offset: 2 })
        );
    }

    #[test]
    fn rejects_align_end_bad_shapes() {
        assert!(parse_align_end("地から字上げ").is_none());
        assert!(parse_align_end("地から０字上げ").is_none());
        assert!(parse_align_end("地から12字上げ").is_none());
        assert!(parse_align_end("地に付き").is_none());
        assert!(parse_align_end("字下げ").is_none());
    }
}
