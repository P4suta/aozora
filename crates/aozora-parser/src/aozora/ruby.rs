//! Ruby (furigana) inline parser.
//!
//! Recognises two forms from the Aozora Bunko spec
//! (<https://www.aozora.gr.jp/annotation/ruby.html>):
//!
//! - Explicit delimiter:  `｜<base>《<reading>》` — `｜` (U+FF5C) marks the start of a
//!   mixed-script base run so the reader doesn't have to guess.
//! - Implicit delimiter:  `<kanji-run>《<reading>》` — the base is the kanji run
//!   directly preceding `《`.

use afm_syntax::Ruby;
use unicode_segmentation::UnicodeSegmentation;

/// Attempt to parse a ruby span starting at `input`. Returns `(Ruby, consumed_byte_len)`
/// on success.
///
/// `starts_with_bar` selects between the explicit-delimiter and implicit-delimiter
/// forms so the caller (dispatch in `inline.rs`) can skip the `｜` byte before calling.
///
/// The implicit-delimiter case (`starts_with_bar == false`) expects `input` to begin at
/// the `《` character; the base is recovered from `preceding_run` which the inline
/// dispatcher supplies from the last text token it emitted.
///
/// This is the M0 Spike skeleton. The integration into the vendored comrak parser
/// lands once the upstream tree is in place.
#[must_use]
pub fn parse(input: &str, starts_with_bar: bool, preceding_run: &str) -> Option<(Ruby, usize)> {
    if starts_with_bar {
        parse_explicit(input)
    } else {
        parse_implicit(input, preceding_run)
    }
}

fn parse_explicit(input: &str) -> Option<(Ruby, usize)> {
    // Expect: <base-run>《<reading>》
    let open = input.find('《')?;
    let base = &input[..open];
    let after_open = &input[open + '《'.len_utf8()..];
    let close = after_open.find('》')?;
    let reading = &after_open[..close];

    let consumed = open + '《'.len_utf8() + close + '》'.len_utf8();
    if base.is_empty() || reading.is_empty() {
        return None;
    }
    Some((
        Ruby {
            base: base.into(),
            reading: reading.into(),
            delim_explicit: true,
        },
        consumed,
    ))
}

fn parse_implicit(input: &str, preceding_run: &str) -> Option<(Ruby, usize)> {
    // Expect: 《<reading>》 with base = trailing kanji run of preceding_run.
    let bytes_open = '《'.len_utf8();
    if !input.starts_with('《') {
        return None;
    }
    let after_open = &input[bytes_open..];
    let close = after_open.find('》')?;
    let reading = &after_open[..close];
    if reading.is_empty() {
        return None;
    }

    let base = trailing_kanji_run(preceding_run);
    if base.is_empty() {
        return None;
    }

    Some((
        Ruby {
            base: base.into(),
            reading: reading.into(),
            delim_explicit: false,
        },
        bytes_open + close + '》'.len_utf8(),
    ))
}

/// Return the trailing run of CJK ideographs in `text`.
///
/// Used by the implicit-delimiter form to split off the base from whatever text was
/// emitted by the parser just before the `《`. A grapheme-cluster walk handles IVS
/// selectors and surrogate-era code points without panicking on malformed input.
#[must_use]
fn trailing_kanji_run(text: &str) -> &str {
    let graphemes: Vec<(usize, &str)> = text.grapheme_indices(true).collect();
    let mut start = text.len();
    for (idx, g) in graphemes.iter().rev() {
        if g.chars().all(is_ruby_base_char) {
            start = *idx;
        } else {
            break;
        }
    }
    &text[start..]
}

const fn is_ruby_base_char(c: char) -> bool {
    // CJK Unified Ideographs (incl. Extensions A, B when they arrive via the text).
    // Also allow middle-dot and ideographic iteration mark which routinely appear
    // inside ruby bases in practice.
    matches!(c,
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFF}'
        | '々'
        | '〆'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_form_captures_base_and_reading() {
        let (ruby, consumed) = parse("青梅《おうめ》あとの文章", true, "").expect("parse");
        assert_eq!(ruby.base.as_plain().expect("plain"), "青梅");
        assert_eq!(ruby.reading.as_plain().expect("plain"), "おうめ");
        assert!(ruby.delim_explicit);
        assert_eq!(consumed, "青梅《おうめ》".len());
    }

    #[test]
    fn explicit_form_rejects_empty_reading() {
        assert!(parse("青梅《》", true, "").is_none());
    }

    #[test]
    fn explicit_form_rejects_unclosed_ruby() {
        assert!(parse("青梅《おうめ", true, "").is_none());
    }

    #[test]
    fn implicit_form_uses_trailing_kanji_run() {
        let (ruby, _) = parse("《にほん》です", false, "彼は日本").expect("parse");
        assert_eq!(ruby.base.as_plain().expect("plain"), "日本");
        assert_eq!(ruby.reading.as_plain().expect("plain"), "にほん");
        assert!(!ruby.delim_explicit);
    }

    #[test]
    fn implicit_form_without_leading_kanji_fails() {
        assert!(parse("《おうめ》", false, "ひらがなのみ").is_none());
    }

    #[test]
    fn trailing_kanji_run_handles_mixed_script() {
        assert_eq!(trailing_kanji_run("彼は日本"), "日本");
        assert_eq!(trailing_kanji_run("漢々"), "漢々");
        assert_eq!(trailing_kanji_run("hello"), "");
    }
}
