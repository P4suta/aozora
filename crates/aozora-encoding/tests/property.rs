//! Property-based + adversarial tests for `aozora-encoding`.
//!
//! The hand-written unit tests in `src/lib.rs` and `src/gaiji.rs` cover
//! known good and known bad inputs. This file complements them with
//! coverage that hits each invariant from multiple angles:
//!
//! * SJIS round-trip via `encoding_rs` for every byte sequence the
//!   encoder produces — the decoder should accept it back.
//! * `decode_sjis` and `decode_sjis_into` are output-equivalent on
//!   every input (the buffer-reuse contract).
//! * `decode_sjis_into` never shrinks the destination buffer's
//!   capacity (the L-3 reuse invariant).
//! * Gaiji `Resolved::write_to` outputs a string whose UTF-8 byte
//!   length matches `Resolved::utf8_len`.
//! * `parse_u_plus` accepts every valid Unicode scalar via the
//!   `U+XXXX` form and rejects every surrogate.
//! * `lookup` is a pure function: same input → same output.
//! * The smart "single-char description" fallback never resolves to
//!   a different scalar than the description itself contains.

use aozora_encoding::gaiji::{Resolved, lookup};
use aozora_encoding::{DecodeError, decode_sjis, decode_sjis_into, has_utf8_bom};
use proptest::collection::vec as prop_vec;
use proptest::option::of as prop_option_of;
use proptest::prelude::*;

// -----------------------------------------------------------------
// SJIS decode — round-trip and equivalence
// -----------------------------------------------------------------

/// Encode any UTF-8 string into SJIS via `encoding_rs` (lossy:
/// non-encodable codepoints become `&#NNN;` HTML entities). For the
/// tests below we constrain the proptest input to characters that
/// `encoding_rs` can faithfully encode, so the round-trip is byte-
/// identical.
fn encode_sjis(s: &str) -> Vec<u8> {
    let (bytes, _, _had_unmappable) = encoding_rs::SHIFT_JIS.encode(s);
    bytes.into_owned()
}

#[test]
fn round_trip_pure_ascii() {
    let s = "Hello, world!";
    let bytes = encode_sjis(s);
    assert_eq!(decode_sjis(&bytes).unwrap(), s);
}

#[test]
fn round_trip_japanese_prose() {
    let s = "青空文庫の本文。";
    let bytes = encode_sjis(s);
    assert_eq!(decode_sjis(&bytes).unwrap(), s);
}

#[test]
fn round_trip_mixed_kanji_kana_ascii() {
    let s = "Today: 今日は晴れ (sunny)。";
    let bytes = encode_sjis(s);
    assert_eq!(decode_sjis(&bytes).unwrap(), s);
}

#[test]
fn into_equivalent_to_owned_for_japanese() {
    let s = "夏目漱石「吾輩は猫である」";
    let bytes = encode_sjis(s);
    let owned = decode_sjis(&bytes).unwrap();
    let mut buf = String::new();
    decode_sjis_into(&bytes, &mut buf).unwrap();
    assert_eq!(owned, buf);
}

#[test]
fn into_keeps_capacity_after_clear_and_reuse() {
    let mut buf = String::with_capacity(8192);
    let cap0 = buf.capacity();
    for _ in 0..16 {
        buf.clear();
        decode_sjis_into(b"hello world", &mut buf).unwrap();
    }
    assert!(
        buf.capacity() >= cap0,
        "buffer-reuse path must not shrink capacity (cap0={cap0}, now={})",
        buf.capacity(),
    );
}

#[test]
fn into_with_zero_capacity_still_decodes() {
    let mut buf = String::new();
    decode_sjis_into(&encode_sjis("青"), &mut buf).unwrap();
    assert_eq!(buf, "青");
}

/// Trail-byte error path. The decoder must classify these as
/// `ShiftJisInvalid`, not silently truncate.
#[test]
fn malformed_trail_byte_is_rejected_strictly() {
    // 0x82 expects a trail in 0x40..=0xFC, != 0x7F. 0x3F is invalid.
    let result = decode_sjis(&[0x82, 0x3F]);
    assert!(matches!(result, Err(DecodeError::ShiftJisInvalid)));
}

#[test]
fn truncation_at_lead_byte_is_rejected() {
    // 0x82 alone — lead without trail.
    assert!(matches!(
        decode_sjis(&[b'a', 0x82]),
        Err(DecodeError::ShiftJisInvalid)
    ));
}

// -----------------------------------------------------------------
// UTF-8 BOM detection — exhaustive boundary
// -----------------------------------------------------------------

#[test]
fn bom_detection_is_exact() {
    assert!(has_utf8_bom(b"\xEF\xBB\xBF"));
    assert!(has_utf8_bom(b"\xEF\xBB\xBFx"));
    assert!(!has_utf8_bom(b"\xEF\xBB"));
    assert!(!has_utf8_bom(b"\xEF"));
    assert!(!has_utf8_bom(b""));
    // Off-by-one negatives:
    for delta in [
        [0xEE, 0xBB, 0xBF],
        [0xEF, 0xBA, 0xBF],
        [0xEF, 0xBB, 0xBE],
        [0xFE, 0xFF, 0x00], // UTF-16 BE, not UTF-8
        [0xFF, 0xFE, 0x00], // UTF-16 LE
    ] {
        assert!(!has_utf8_bom(&delta), "false positive on {delta:?}");
    }
}

// -----------------------------------------------------------------
// Gaiji — purity, write_to / utf8_len consistency, edge cases
// -----------------------------------------------------------------

#[test]
fn lookup_is_pure_repeated_calls_return_identical_results() {
    for inputs in [
        (None, Some("第3水準1-85-54"), "木＋吶のつくり"),
        (None, Some("U+0041"), ""),
        (Some('あ'), Some("anything"), "anything"),
        (None, None, "〓"),
        (None, None, "丂"),
    ] {
        let a = lookup(inputs.0, inputs.1, inputs.2);
        let b = lookup(inputs.0, inputs.1, inputs.2);
        assert_eq!(a, b, "lookup is not pure for {inputs:?}");
    }
}

#[test]
fn write_to_yields_utf8_len_bytes() {
    for r in [
        Resolved::Char('A'),
        Resolved::Char('あ'),
        Resolved::Char('𠂉'),
        Resolved::Multi("\u{304B}\u{309A}"),
        Resolved::Multi("\u{30AB}\u{309A}"),
    ] {
        let mut s = String::new();
        r.write_to(&mut s).unwrap();
        assert_eq!(
            s.len(),
            r.utf8_len(),
            "write_to byte count != utf8_len() for {r:?}",
        );
    }
}

#[test]
fn smart_fallback_resolves_only_to_the_description_itself() {
    // Whatever the description's single character is, the smart
    // fallback must echo IT, not transform it.
    for ch in ['A', 'あ', '丂', '畺', '龔', '𠂉'] {
        let s = ch.to_string();
        // The two arguments below are sentinels guaranteed not to be
        // in any of the gaiji tables; the smart fallback is the only
        // path that can fire.
        let r = lookup(None, Some("__not-a-real-mencode__"), &s);
        match r {
            Some(Resolved::Char(c)) if c == ch => {}
            // 〓 is in the description table; it would resolve via
            // the dictionary path before the fallback kicks in.
            other => panic!("char {ch} fallback gave {other:?}"),
        }
    }
}

#[test]
fn smart_fallback_does_not_fire_on_two_char_descriptions() {
    // Even if the description contains nothing from the dictionary,
    // multi-char descriptions must not trigger the single-char path.
    for desc in ["AB", "あい", "ab", "丂畺", "𠂉𠁫"] {
        // Sentinel mencode not in any table.
        let r = lookup(None, Some("__not-a-real-mencode__"), desc);
        assert_eq!(r, None, "fallback fired on multi-char desc {desc:?}");
    }
}

#[test]
fn u_plus_path_accepts_every_scalar_including_emoji() {
    for (mencode, want) in [
        ("U+0041", Some('A')),
        ("U+1F600", Some('😀')),
        ("U+10FFFF", Some('\u{10FFFF}')),
        ("U+0", Some('\u{0}')),
        // Surrogates rejected:
        ("U+D800", None),
        ("U+DFFF", None),
        // Past Unicode max:
        ("U+110000", None),
        // Too many digits:
        ("U+1234567", None),
        // Empty:
        ("U+", None),
        // Bad hex:
        ("U+ZZZZ", None),
        // Missing prefix:
        ("0041", None),
    ] {
        let r = lookup(None, Some(mencode), "");
        let expected = want.map(Resolved::Char);
        assert_eq!(r, expected, "U+ path mismatch for {mencode:?}");
    }
}

#[test]
fn existing_short_circuit_takes_precedence_over_every_other_path() {
    // Even if the mencode and description would resolve, the
    // caller-provided existing wins.
    let r = lookup(Some('Z'), Some("第3水準1-85-54"), "木＋吶のつくり");
    assert_eq!(r, Some(Resolved::Char('Z')));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// Round-trip: encoding_rs encode + our decode_sjis must yield
    /// the same string for any character SJIS supports. We restrict
    /// the input to ASCII printable + hiragana so unmappable
    /// characters don't change the encoded length and we can assert
    /// byte-identical equivalence.
    #[test]
    fn sjis_round_trip_ascii_and_hiragana(
        s in "[A-Za-z0-9 \u{3041}-\u{3093}]{0,40}",
    ) {
        let bytes = encode_sjis(&s);
        let back = decode_sjis(&bytes).unwrap();
        prop_assert_eq!(back, s);
    }

    /// `decode_sjis_into` and `decode_sjis` produce the same string
    /// (or both fail) for every byte input — the L-3 buffer-reuse
    /// equivalence contract.
    #[test]
    fn into_and_owned_are_output_equivalent_on_arbitrary_bytes(
        bytes in prop_vec(any::<u8>(), 0..200),
    ) {
        let owned = decode_sjis(&bytes);
        let mut buf = String::new();
        let into = decode_sjis_into(&bytes, &mut buf);
        match (owned, into) {
            (Ok(s), Ok(())) => prop_assert_eq!(s, buf),
            (Err(_), Err(_)) => {} // both reject — fine
            (Ok(s), Err(e)) => prop_assert!(false, "owned ok({s:?}) but into err({e:?})"),
            (Err(e), Ok(())) => prop_assert!(false, "owned err({e:?}) but into ok({buf:?})"),
        }
    }

    /// Gaiji `lookup` is total — never panics, for any combination
    /// of inputs the lexer might construct. The result is either
    /// `Some(Resolved::*)` or `None`, both of which are valid.
    #[test]
    fn lookup_is_total_and_total_only(
        existing in prop_option_of(any::<char>()),
        mencode in prop_option_of("[\u{0020}-\u{007E}]{0,30}"),
        description in "[\u{0020}-\u{007E}\u{3041}-\u{3093}]{0,20}",
    ) {
        // The result is either Some/None — both are valid; we just
        // pin that the function does not panic. Force-use the
        // `#[must_use]` return value so clippy doesn't complain.
        let r = lookup(existing, mencode.as_deref(), &description);
        prop_assert!(matches!(r, Some(_) | None));
    }

    /// `Resolved::write_to` always writes exactly `utf8_len()` bytes.
    /// Pinned via the lookup result to exercise BOTH variants (Char
    /// and Multi).
    #[test]
    fn write_to_byte_count_matches_utf8_len(
        // 第3水準1-4-* mostly covers combo cells; 1-85-* mostly char.
        kind in 0..3u8,
        existing in prop_option_of(any::<char>()),
    ) {
        let r = match kind {
            0 => lookup(existing, Some("第3水準1-4-87"), ""), // combo
            1 => lookup(existing, Some("第3水準1-85-54"), ""), // char
            _ => lookup(existing, Some("U+1F600"), ""), // emoji char
        };
        if let Some(r) = r {
            let mut s = String::new();
            r.write_to(&mut s).unwrap();
            prop_assert_eq!(s.len(), r.utf8_len());
        }
    }
}
