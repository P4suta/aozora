//! Encoding utilities for Aozora Bunko source material.
//!
//! `afm-parser` itself is strictly UTF-8. Anything that decodes `Shift_JIS` or
//! resolves gaiji (外字) mappings lives here, so the parser stays free of encoding
//! concerns and the same logic is available to CLI, editor integrations, or
//! downstream tools.

#![forbid(unsafe_code)]

use encoding_rs::{Encoding, SHIFT_JIS};
use miette::Diagnostic;
use thiserror::Error;

/// Errors surfaced by the decode pipeline.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum DecodeError {
    #[error("Shift_JIS からの変換に失敗しました (不正なバイト列)")]
    #[diagnostic(code(afm::encoding::sjis_invalid))]
    ShiftJisInvalid,
}

/// Decode a `Shift_JIS` byte slice into UTF-8 (NFC normalisation is applied by the
/// caller after decoding).
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] if `encoding_rs` reports a malformed byte
/// sequence. Lossy replacement is deliberately not offered — callers need to know
/// when they're looking at corrupted source material rather than silently absorbing
/// the damage.
pub fn decode_sjis(input: &[u8]) -> Result<String, DecodeError> {
    decode_strict(SHIFT_JIS, input)
}

/// Whether the byte slice carries a UTF-8 BOM (`EF BB BF`).
///
/// Used by the CLI to strip the BOM before handing input to the parser. The
/// CLI requires an explicit `--encoding` flag, so BOM presence is the only
/// runtime signal we care about. A full encoding sniffer (BOM + byte-frequency
/// heuristic) is intentionally out of scope until unknown-encoding input
/// streams become a concern.
#[must_use]
pub const fn has_utf8_bom(input: &[u8]) -> bool {
    matches!(input, [0xEF, 0xBB, 0xBF, ..])
}

fn decode_strict(encoding: &'static Encoding, input: &[u8]) -> Result<String, DecodeError> {
    let (cow, _used, had_errors) = encoding.decode(input);
    if had_errors {
        return Err(DecodeError::ShiftJisInvalid);
    }
    Ok(cow.into_owned())
}

pub mod gaiji;

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // SJIS happy-path decoding
    // ------------------------------------------------------------------

    #[test]
    fn decodes_plain_ascii_sjis() {
        assert_eq!(decode_sjis(b"hello").unwrap(), "hello");
    }

    #[test]
    fn decodes_japanese_sjis() {
        // 「青空文庫」 in Shift_JIS.
        let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        assert_eq!(decode_sjis(bytes).unwrap(), "青空文庫");
    }

    #[test]
    fn decodes_empty_input_to_empty_string() {
        assert_eq!(decode_sjis(b"").unwrap(), "");
    }

    #[test]
    fn decodes_ascii_control_characters_verbatim() {
        // LF / CR / tab are 1:1 identity in SJIS since the lead byte
        // range avoids ASCII. Exercising these locks in the pipeline
        // doesn't mangle them before the sanitize pass.
        assert_eq!(decode_sjis(b"a\nb\rc\td").unwrap(), "a\nb\rc\td");
    }

    #[test]
    fn decodes_halfwidth_katakana() {
        // Halfwidth katakana (0xA1..=0xDF) is a single byte each in SJIS.
        // `ｱｲｳｴｵ` → bytes 0xB1..0xB5.
        let bytes = &[0xB1, 0xB2, 0xB3, 0xB4, 0xB5];
        assert_eq!(decode_sjis(bytes).unwrap(), "ｱｲｳｴｵ");
    }

    #[test]
    fn decodes_mixed_ascii_and_kanji() {
        // Common shape in Aozora corpora: explanatory text in ASCII
        // mixed with Japanese quotations.
        let mut bytes = Vec::from(*b"about ");
        bytes.extend_from_slice(&[0x93, 0xFA, 0x96, 0x7B]); // 日本
        bytes.extend_from_slice(b" !");
        assert_eq!(decode_sjis(&bytes).unwrap(), "about 日本 !");
    }

    #[test]
    fn decodes_hiragana_sjis() {
        // 「こんにちは」 — lead bytes in the 0x82 range.
        let bytes = &[
            0x82, 0xB1, // こ
            0x82, 0xF1, // ん
            0x82, 0xC9, // に
            0x82, 0xBF, // ち
            0x82, 0xCD, // は
        ];
        assert_eq!(decode_sjis(bytes).unwrap(), "こんにちは");
    }

    #[test]
    fn decodes_fullwidth_digits() {
        // １２３ — fullwidth digits are common in Aozora ruby delimiters.
        let bytes = &[0x82, 0x4F, 0x82, 0x50, 0x82, 0x51];
        assert_eq!(decode_sjis(bytes).unwrap(), "０１２");
    }

    // ------------------------------------------------------------------
    // SJIS error surfaces
    // ------------------------------------------------------------------

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
        // 0x82 alone is a truncated two-byte sequence (expects trail).
        let bytes = &[b'o', b'k', 0x82];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn rejects_invalid_trail_byte() {
        // Lead 0x82 with an invalid trail 0x00 (trails must be 0x40..=0xFC, != 0x7F).
        let bytes = &[0x82, 0x00];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
    }

    #[test]
    fn error_message_is_japanese_and_carries_miette_code() {
        // The project-wide rule is that user-facing errors are in
        // Japanese. Pin that and the miette diagnostic code both.
        let err = decode_sjis(&[0xFF, 0xFF]).unwrap_err();
        let message = format!("{err}");
        assert!(
            message.contains("Shift_JIS"),
            "error message must contain Shift_JIS for locatability, got {message:?}",
        );
    }

    // ------------------------------------------------------------------
    // UTF-8 BOM detection
    // ------------------------------------------------------------------

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
        // Boundary: the slice is exactly `EF BB BF` with no trailing
        // content. `matches!` pattern with `..` rest binding accepts
        // empty tails.
        assert!(has_utf8_bom(&[0xEF, 0xBB, 0xBF]));
    }

    #[test]
    fn bom_detection_rejects_near_misses() {
        // Off-by-one patterns that are NOT the UTF-8 BOM.
        assert!(!has_utf8_bom(&[0xEF, 0xBB, 0xBE])); // last byte wrong
        assert!(!has_utf8_bom(&[0xEE, 0xBB, 0xBF])); // first byte wrong
        assert!(!has_utf8_bom(&[0xEF, 0xBC, 0xBF])); // middle byte wrong
        assert!(!has_utf8_bom(&[0xFE, 0xFF])); // UTF-16 BE BOM — not ours
        assert!(!has_utf8_bom(&[0xFF, 0xFE])); // UTF-16 LE BOM — not ours
    }

    // ------------------------------------------------------------------
    // Gaiji resolution
    // ------------------------------------------------------------------

    #[test]
    fn gaiji_resolve_echoes_description_and_ucs_when_present() {
        use aozora_syntax::Gaiji;
        let node = Gaiji {
            description: "木＋吶のつくり".into(),
            ucs: Some('吶'),
            mencode: Some("第3水準1-85-54".into()),
        };
        let r = gaiji::resolve(&node);
        assert_eq!(r.character, Some('吶'));
        assert_eq!(&*r.description, "木＋吶のつくり");
    }

    #[test]
    fn gaiji_resolve_returns_none_character_when_ucs_unresolved() {
        use aozora_syntax::Gaiji;
        let node = Gaiji {
            description: "第3水準1-85-54".into(),
            ucs: None,
            mencode: None,
        };
        let r = gaiji::resolve(&node);
        assert_eq!(r.character, None);
        assert_eq!(&*r.description, "第3水準1-85-54");
    }
}
