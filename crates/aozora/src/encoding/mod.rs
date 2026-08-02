//! Encoding utilities for Aozora Bunko source material.
//!
//! The `aozora` parser itself is strictly UTF-8. Anything that decodes `Shift_JIS` or
//! resolves gaiji (外字) mappings lives here, so the parser stays free of encoding
//! concerns and the same logic is available to CLI, editor integrations, or
//! downstream tools.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::str::from_utf8;

use encoding_rs::{DecoderResult, SHIFT_JIS};
use miette::Diagnostic;
use thiserror::Error;

/// Errors surfaced by the decode pipeline.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input was not valid `Shift_JIS`: `encoding_rs` reported a malformed
    /// byte sequence (or, in [`decode_sjis_into`], an output-buffer overflow).
    ///
    /// Carries no position payload — the decode path is deliberately strict and
    /// all-or-nothing rather than offering lossy replacement, so callers learn
    /// they're looking at corrupted source rather than silently absorbing it.
    #[error("Shift_JIS からの変換に失敗しました (不正なバイト列)")]
    #[diagnostic(code(aozora::encoding::sjis_invalid))]
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
///
/// Allocates a fresh `String` per call. For workloads that decode many
/// documents in succession, prefer [`decode_sjis_into`] with a reusable
/// buffer to avoid the per-call allocation.
pub fn decode_sjis(input: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::new();
    decode_sjis_into(input, &mut out)?;
    Ok(out)
}

/// Decode a `Shift_JIS` byte slice into the caller-owned `dst` buffer.
///
/// Pre-sizes `dst` exactly via
/// `encoding_rs::Decoder::max_utf8_buffer_length_without_replacement`
/// so the decode inner loop does no growth-realloc. The buffer is
/// **not** cleared first — callers that want a fresh decode should
/// `dst.clear()` before calling. This is intentional so the same
/// buffer can be reused across many decodes in a thread-local /
/// per-worker pool without paying the allocator per iteration.
///
/// Strict — same error contract as [`decode_sjis`]. Bypasses
/// `encoding_rs`'s public `decode` shape, which always allocates a
/// worst-case-sized `String` internally and `Cow::into_owned`s the
/// result; this entry point goes straight through the
/// `Decoder::decode_to_string_without_replacement` API the bench
/// pipeline needs.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] on malformed input or if
/// the encoder reports overflow (which `max_utf8_buffer_length_…`
/// should make unreachable, but is still surfaced rather than
/// silently truncated).
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

/// Decode Aozora source bytes to UTF-8, detecting the encoding.
///
/// Aozora material reaches this crate in two shapes: the canonical
/// `Shift_JIS` archive files, and already-decoded UTF-8 mirrors (e.g. a
/// corpus that has been pre-converted). Forcing every caller to commit
/// to one encoding up front is the wrong default — it is why corpus
/// tooling historically hard-coded [`decode_sjis`] and silently broke on
/// UTF-8 input. This entry point removes that concern: hand it bytes,
/// get back UTF-8.
///
/// - Valid UTF-8 is returned **borrowed**, zero-copy.
/// - Otherwise the bytes are decoded as `Shift_JIS` (owned).
///
/// UTF-8 is tried first on purpose. Valid UTF-8 is a near-unambiguous
/// signal — `Shift_JIS` Japanese text essentially never forms a wholly
/// valid UTF-8 sequence — whereas the converse does not hold: a UTF-8
/// document can contain byte runs that decode as *some* `Shift_JIS`
/// without erroring, so sniffing `Shift_JIS` first risks mojibake on
/// UTF-8 input.
///
/// BOM stripping, CRLF folding and NFC normalisation are the parser's
/// Phase-0 responsibility and are deliberately not applied here.
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] when the bytes are neither
/// valid UTF-8 nor valid `Shift_JIS`.
pub fn decode_auto(input: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
    if let Ok(text) = from_utf8(input) {
        return Ok(Cow::Borrowed(text));
    }
    decode_sjis(input).map(Cow::Owned)
}

/// Encoding-agnostic counterpart to [`decode_sjis_into`]: append the
/// decoded UTF-8 to `dst`, detecting the source encoding.
///
/// Same sniffing rule as [`decode_auto`] (valid UTF-8 wins, else
/// `Shift_JIS`), but writes into a caller-owned buffer so corpus
/// loaders can reuse one allocation across many documents. The buffer
/// is **not** cleared first — see [`decode_sjis_into`].
///
/// # Errors
///
/// Returns [`DecodeError::ShiftJisInvalid`] when the bytes are neither
/// valid UTF-8 nor valid `Shift_JIS`.
pub fn decode_auto_into(input: &[u8], dst: &mut String) -> Result<(), DecodeError> {
    if let Ok(text) = from_utf8(input) {
        dst.push_str(text);
        return Ok(());
    }
    decode_sjis_into(input, dst)
}

/// Whether the byte slice carries a UTF-8 BOM (`EF BB BF`).
///
/// Used by the CLI to strip the BOM before handing input to the parser.
/// BOM presence is the one signal even [`decode_auto`] leaves to the
/// caller: it is valid UTF-8, so it round-trips through `decode_auto`
/// untouched and is stripped by the parser's Phase-0 sanitiser.
#[must_use]
pub const fn has_utf8_bom(input: &[u8]) -> bool {
    matches!(input, [0xEF, 0xBB, 0xBF, ..])
}

pub(crate) mod gaiji;

/// PHF tables (single, combo, description) emitted by `build.rs`
/// at compile time via `phf_codegen`. Lives in `OUT_DIR` so it's
/// regenerated automatically when any input TSV changes; the
/// committed source tree carries only the data, not the perfect-
/// hash output. See `build.rs` for the generator.
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
    // decode_auto — encoding-agnostic entry point
    // ------------------------------------------------------------------

    #[test]
    fn decode_auto_passes_utf8_through_borrowed() {
        let bytes = "青空文庫".as_bytes();
        let out = decode_auto(bytes).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)), "UTF-8 must be zero-copy");
        assert_eq!(out, "青空文庫");
    }

    #[test]
    fn decode_auto_falls_back_to_sjis_owned() {
        // 「青空文庫」 in Shift_JIS — not valid UTF-8, so it decodes.
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
        // ASCII is valid in both encodings; UTF-8-first means borrowed.
        let out = decode_auto(b"hello").unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "hello");
    }

    #[test]
    fn decode_auto_prefers_utf8_over_ambiguous_sjis() {
        // UTF-8 「日本語」 = E3 81 A5 ... whose leading bytes are *also*
        // valid Shift_JIS lead bytes. Sniffing SJIS first would mojibake
        // this; UTF-8-first returns the correct text, borrowed.
        let bytes = "日本語".as_bytes();
        let out = decode_auto(bytes).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "日本語");
    }

    #[test]
    fn decode_auto_errors_when_neither_encoding_fits() {
        // 0xFF is neither a valid UTF-8 byte nor an assigned Shift_JIS byte.
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
        decode_auto_into("青空".as_bytes(), &mut buf).unwrap(); // UTF-8
        decode_auto_into(&[0x95, 0xB6, 0x8C, 0xC9], &mut buf).unwrap(); // 文庫 in SJIS
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

    // ------------------------------------------------------------------
    // decode_sjis_into — buffer-reuse path equivalence
    // ------------------------------------------------------------------
    //
    // Every test below the section header verifies the contract that
    // `decode_sjis(b) == decode_sjis_into(b, &mut buf)` byte-for-byte
    // (and for the strict-error case, returns the same `Err`).
    // `decode_sjis_into` is the buffer-reuse entry point used by the
    // bench `parallel_size_bands` thread-local pool; the production
    // `decode_sjis` is a thin wrapper that calls `decode_sjis_into`
    // with a fresh `String`.

    fn check_equivalent(input: &[u8]) {
        let owned = decode_sjis(input);
        let mut buf = String::new();
        let into_result = decode_sjis_into(input, &mut buf);
        match (owned, into_result) {
            (Ok(s), Ok(())) => assert_eq!(s, buf, "decode_sjis output != decode_sjis_into output"),
            (Err(_), Err(_)) => {} // both fail — identical strict error contract
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
        // The buffer-reuse contract: a `dst` String that already has
        // enough capacity should not allocate again on the second
        // decode. We verify this by asserting capacity is preserved
        // across `clear() + decode_sjis_into` cycles. (Pinning the
        // exact byte count would couple the test to bumpalo /
        // encoding_rs internals; the load-bearing invariant is "no
        // shrink".)
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
        // Documented contract: callers must `clear()` before each
        // decode if they want a fresh result. This test pins that
        // shape so future "convenience clear inside the function"
        // changes break loudly.
        let mut buf = String::from("PRE:");
        decode_sjis_into(b"hi", &mut buf).unwrap();
        assert_eq!(buf, "PRE:hi");
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
    // Gaiji resolution (via primitive `gaiji::lookup`)
    // ------------------------------------------------------------------

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
