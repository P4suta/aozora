//! Decodes source bytes and resolves gaiji before parsing.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::str::from_utf8;

use encoding_rs::{DecoderResult, SHIFT_JIS};
use miette::Diagnostic;
use thiserror::Error;

/// Errors from decoding source bytes.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum DecodeError {
    /// Malformed `Shift_JIS` input or insufficient output capacity.
    #[error("Shift_JIS からの変換に失敗しました (不正なバイト列)")]
    #[diagnostic(code(aozora::encoding::sjis_invalid))]
    ShiftJisInvalid,
}

/// Decode `Shift_JIS` bytes to UTF-8 without replacing malformed sequences.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] for malformed input or insufficient output capacity.
pub fn decode_sjis(input: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::new();
    decode_sjis_into(input, &mut out)?;
    Ok(out)
}

/// Append decoded `Shift_JIS` text to `dst`.
///
/// Preserves the original contents of `dst` if decoding fails.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] for malformed input or insufficient output capacity.
pub fn decode_sjis_into(input: &[u8], dst: &mut String) -> Result<(), DecodeError> {
    let original_len = dst.len();
    let mut decoder = SHIFT_JIS.new_decoder_without_bom_handling();
    let needed = decoder
        .max_utf8_buffer_length_without_replacement(input.len())
        .ok_or(DecodeError::ShiftJisInvalid)?;
    dst.reserve(needed);
    let (result, _read) = decoder.decode_to_string_without_replacement(input, dst, true);
    match result {
        DecoderResult::InputEmpty => Ok(()),
        DecoderResult::Malformed(_, _) | DecoderResult::OutputFull => {
            dst.truncate(original_len);
            Err(DecodeError::ShiftJisInvalid)
        }
    }
}

/// Decode source bytes, preferring UTF-8 over `Shift_JIS`.
///
/// Borrows valid UTF-8 input and returns owned text after `Shift_JIS` decoding.
/// Preserves byte order marks, line endings, and Unicode normalization.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] for invalid input or insufficient output capacity.
pub fn decode_auto(input: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
    if let Ok(text) = from_utf8(input) {
        return Ok(Cow::Borrowed(text));
    }
    decode_sjis(input).map(Cow::Owned)
}

/// Append decoded source bytes to `dst`, preferring UTF-8 over `Shift_JIS`.
///
/// Preserves the original contents of `dst` if decoding fails.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] for invalid input or insufficient output capacity.
pub fn decode_auto_into(input: &[u8], dst: &mut String) -> Result<(), DecodeError> {
    if let Ok(text) = from_utf8(input) {
        dst.push_str(text);
        return Ok(());
    }
    decode_sjis_into(input, dst)
}

/// Whether the input begins with the UTF-8 byte order mark.
#[must_use]
pub const fn has_utf8_bom(input: &[u8]) -> bool {
    const UTF8_BOM_LEAD: u8 = 0xEF;
    const UTF8_BOM_MIDDLE: u8 = 0xBB;
    const UTF8_BOM_TAIL: u8 = 0xBF;
    matches!(input, [UTF8_BOM_LEAD, UTF8_BOM_MIDDLE, UTF8_BOM_TAIL, ..])
}

pub(crate) mod gaiji;

#[expect(
    clippy::unreadable_literal,
    reason = "phf_codegen emits 64-bit perfect-hash keys without separators; \
              we cannot reformat them without forking the codegen crate"
)]
mod jisx0213_table {
    include!(concat!(env!("OUT_DIR"), "/aozora_gaiji_table.rs"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_ascii_sjis() {
        assert_eq!(decode_sjis(b"hello").unwrap(), "hello");
    }

    #[test]
    fn decodes_japanese_sjis() {
        let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        assert_eq!(decode_sjis(bytes).unwrap(), "青空文庫");
    }

    #[test]
    fn decodes_empty_input_to_empty_string() {
        assert_eq!(decode_sjis(b"").unwrap(), "");
    }

    #[test]
    fn decodes_ascii_control_characters_verbatim() {
        assert_eq!(decode_sjis(b"a\nb\rc\td").unwrap(), "a\nb\rc\td");
    }

    #[test]
    fn decodes_halfwidth_katakana() {
        let bytes = &[0xB1, 0xB2, 0xB3, 0xB4, 0xB5];
        assert_eq!(decode_sjis(bytes).unwrap(), "ｱｲｳｴｵ");
    }

    #[test]
    fn decodes_mixed_ascii_and_kanji() {
        let mut bytes = Vec::from(*b"about ");
        bytes.extend_from_slice(&[0x93, 0xFA, 0x96, 0x7B]);
        bytes.extend_from_slice(b" !");
        assert_eq!(decode_sjis(&bytes).unwrap(), "about 日本 !");
    }

    #[test]
    fn decodes_hiragana_sjis() {
        let bytes = &[0x82, 0xB1, 0x82, 0xF1, 0x82, 0xC9, 0x82, 0xBF, 0x82, 0xCD];
        assert_eq!(decode_sjis(bytes).unwrap(), "こんにちは");
    }

    #[test]
    fn decodes_fullwidth_digits() {
        let bytes = &[0x82, 0x4F, 0x82, 0x50, 0x82, 0x51];
        assert_eq!(decode_sjis(bytes).unwrap(), "０１２");
    }

    #[test]
    fn decode_auto_passes_utf8_through_borrowed() {
        let bytes = "青空文庫".as_bytes();
        let out = decode_auto(bytes).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)), "UTF-8 must be zero-copy");
        assert_eq!(out, "青空文庫");
    }

    #[test]
    fn decode_auto_falls_back_to_sjis_owned() {
        let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        let out = decode_auto(bytes).unwrap();
        assert!(
            matches!(out, Cow::Owned(_)),
            "SJIS must be decoded to owned"
        );
        assert_eq!(out, "青空文庫");
    }

    #[test]
    fn decode_auto_borrows_ascii() {
        let out = decode_auto(b"hello").unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "hello");
    }

    #[test]
    fn decode_auto_prefers_utf8_over_ambiguous_sjis() {
        let bytes = "日本語".as_bytes();
        let out = decode_auto(bytes).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "日本語");
    }

    #[test]
    fn decode_auto_errors_when_neither_encoding_fits() {
        assert!(matches!(
            decode_auto(&[0xFF, 0xFF]),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn decode_auto_empty_is_borrowed_empty() {
        let out = decode_auto(b"").unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "");
    }

    #[test]
    fn decode_auto_into_appends_both_encodings() {
        let mut buf = String::new();
        decode_auto_into("青空".as_bytes(), &mut buf).unwrap();
        decode_auto_into(&[0x95, 0xB6, 0x8C, 0xC9], &mut buf).unwrap();
        assert_eq!(buf, "青空文庫");
    }

    #[test]
    fn decode_sjis_into_appends_only_on_success() {
        let mut dst = "prefix:".to_owned();
        decode_sjis_into(&[0x95, 0xB6, 0x8C, 0xC9], &mut dst).expect("valid Shift_JIS");
        assert_eq!(dst, "prefix:文庫");

        let before = dst.clone();
        let result = decode_sjis_into(b"valid prefix\x82", &mut dst);
        assert!(matches!(result, Err(DecodeError::ShiftJisInvalid)));
        assert_eq!(dst, before);
    }

    fn check_equivalent(input: &[u8]) {
        let owned = decode_sjis(input);
        let mut buf = String::new();
        let into_result = decode_sjis_into(input, &mut buf);
        match (owned, into_result) {
            (Ok(s), Ok(())) => assert_eq!(s, buf, "decode_sjis output != decode_sjis_into output"),
            (Err(_), Err(_)) => {}
            (Ok(s), Err(e)) => panic!("owned succeeded ({s:?}) but _into failed ({e:?})"),
            (Err(e), Ok(())) => panic!("owned failed ({e:?}) but _into succeeded ({buf:?})"),
        }
    }

    #[test]
    fn into_equivalent_on_ascii() {
        check_equivalent(b"hello world");
    }

    #[test]
    fn into_equivalent_on_japanese() {
        check_equivalent(&[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9]);
    }

    #[test]
    fn into_equivalent_on_empty() {
        check_equivalent(b"");
    }

    #[test]
    fn into_equivalent_on_halfwidth_katakana() {
        check_equivalent(&[0xB1, 0xB2, 0xB3, 0xB4, 0xB5]);
    }

    #[test]
    fn into_equivalent_on_invalid_lead_byte() {
        check_equivalent(&[0xFF, 0xFF]);
    }

    #[test]
    fn into_equivalent_on_lone_lead_byte() {
        check_equivalent(&[b'o', b'k', 0x82]);
    }

    #[test]
    fn into_reuses_buffer_capacity_across_calls() {
        let mut buf = String::with_capacity(4096);
        let cap_before = buf.capacity();
        decode_sjis_into(b"hello", &mut buf).unwrap();
        let cap_after_first = buf.capacity();
        assert!(
            cap_after_first >= cap_before,
            "capacity must not shrink on small decode"
        );
        buf.clear();
        decode_sjis_into(b"world", &mut buf).unwrap();
        assert!(
            buf.capacity() >= cap_after_first,
            "capacity must not shrink on a buffer-reuse cycle"
        );
    }

    #[test]
    fn into_appends_when_dst_not_cleared() {
        let mut buf = String::from("PRE:");
        decode_sjis_into(b"hi", &mut buf).unwrap();
        assert_eq!(buf, "PRE:hi");
    }

    #[test]
    fn rejects_invalid_lead_byte() {
        let bytes = &[0xFF, 0xFF];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn rejects_lone_lead_byte_at_end_of_input() {
        let bytes = &[b'o', b'k', 0x82];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn rejects_invalid_trail_byte() {
        let bytes = &[0x82, 0x00];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn error_message_is_japanese_and_carries_miette_code() {
        let err = decode_sjis(&[0xFF, 0xFF]).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("Shift_JIS"),
            "error message must contain Shift_JIS for locatability, got {message:?}",
        );
    }

    #[test]
    fn detects_utf8_bom() {
        assert!(has_utf8_bom(b"\xEF\xBB\xBFtext"));
    }

    #[test]
    fn no_utf8_bom_on_plain_input() {
        assert!(!has_utf8_bom(b"text"));
    }

    #[test]
    fn no_utf8_bom_on_shorter_than_bom() {
        assert!(!has_utf8_bom(b"\xEF\xBB"));
    }

    #[test]
    fn no_utf8_bom_on_empty_input() {
        assert!(!has_utf8_bom(b""));
    }

    #[test]
    fn detects_utf8_bom_on_exactly_three_bytes() {
        assert!(has_utf8_bom(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn bom_detection_rejects_near_misses() {
        assert!(!has_utf8_bom(&[0xEF, 0xBB, 0xBE]));
        assert!(!has_utf8_bom(&[0xEE, 0xBB, 0xBF]));
        assert!(!has_utf8_bom(&[0xEF, 0xBC, 0xBF]));
        assert!(!has_utf8_bom(&[0xFE, 0xFF]));
        assert!(!has_utf8_bom(&[0xFF, 0xFE]));
    }

    #[test]
    fn gaiji_lookup_echoes_existing_ucs_when_set() {
        assert_eq!(
            gaiji::lookup(Some('吶'), Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(gaiji::Resolved::Char('吶'))
        );
    }

    #[test]
    fn gaiji_lookup_returns_none_when_unresolvable() {
        assert_eq!(gaiji::lookup(None, None, "第3水準1-85-54"), None);
    }
}
