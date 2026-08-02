//! Source-encoding selection for `aozora fmt`.
//!
//! `aozora fmt` reads real Aozora Bunko files, which ship as Shift_JIS; the
//! [`Encoding`] value-enum and [`decode`] resolve their bytes to text. clap
//! lives here rather than in the lower-level `aozora-encoding` crate, which
//! stays dependency-light.

use std::borrow::Cow;

use anyhow::{Context, Result};
use clap::ValueEnum;
use encoding_rs::SHIFT_JIS;

use aozora::{decode_auto, decode_sjis};

use crate::fmt::source;

/// How to decode source bytes into text.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Encoding {
    /// Detect the source encoding: valid UTF-8 is used as-is, otherwise the
    /// bytes are decoded as Shift_JIS. The right default — Aozora files ship
    /// as Shift_JIS, but UTF-8 mirrors are common, and the caller should not
    /// have to know which they have.
    #[default]
    Auto,
    /// Force UTF-8; error if the input is not valid UTF-8.
    Utf8,
    /// Force Shift_JIS decoding.
    Sjis,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum FileEncoding {
    Utf8,
    ShiftJis,
}

pub(super) struct Decoded {
    pub text: String,
    pub encoding: FileEncoding,
}

/// Decode `raw` under `encoding` into an owned `String`.
///
/// The decoded text is checked against the parser core's `u32` span limit:
/// Shift_JIS → UTF-8 can expand past it even when the raw bytes fit, so this is
/// the choke point every read path shares.
///
/// # Errors
///
/// Returns an error when the bytes do not decode under the chosen encoding
/// (invalid UTF-8 for [`Encoding::Utf8`], a Shift_JIS decode failure for
/// [`Encoding::Sjis`], or neither for [`Encoding::Auto`]), or when the decoded
/// text exceeds [`MAX_SOURCE_BYTES`](crate::fmt::source::MAX_SOURCE_BYTES).
pub(crate) fn decode(raw: &[u8], encoding: Encoding) -> Result<String> {
    decode_with_encoding(raw, encoding).map(|decoded| decoded.text)
}

pub(super) fn decode_with_encoding(raw: &[u8], encoding: Encoding) -> Result<Decoded> {
    let decoded = match encoding {
        Encoding::Auto => {
            let text = decode_auto(raw)
                .map_err(|e| anyhow::anyhow!("input is neither valid UTF-8 nor Shift_JIS: {e}"))?;
            let encoding = if matches!(text, Cow::Borrowed(_)) {
                FileEncoding::Utf8
            } else {
                FileEncoding::ShiftJis
            };
            Decoded {
                text: text.into_owned(),
                encoding,
            }
        }
        Encoding::Utf8 => String::from_utf8(raw.to_vec())
            .map_err(|e| e.utf8_error())
            .context("input is not valid UTF-8 (use --encoding sjis for Aozora Bunko files)")
            .map(|text| Decoded {
                text,
                encoding: FileEncoding::Utf8,
            })?,
        Encoding::Sjis => decode_sjis(raw)
            .map_err(|e| anyhow::anyhow!("Shift_JIS decode failed: {e}"))
            .map(|text| Decoded {
                text,
                encoding: FileEncoding::ShiftJis,
            })?,
    };
    source::ensure_within_span_limit(decoded.text.len() as u64)?;
    Ok(decoded)
}

pub(super) fn encode(text: &str, encoding: FileEncoding) -> Result<Cow<'_, [u8]>> {
    match encoding {
        FileEncoding::Utf8 => Ok(Cow::Borrowed(text.as_bytes())),
        FileEncoding::ShiftJis => {
            let (encoded, _, had_errors) = SHIFT_JIS.encode(text);
            if had_errors {
                anyhow::bail!(
                    "formatted source contains characters not representable in Shift_JIS"
                );
            }
            Ok(encoded)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_passes_through() {
        assert_eq!(decode("日本".as_bytes(), Encoding::Utf8).unwrap(), "日本");
    }

    #[test]
    fn auto_detects_utf8() {
        assert_eq!(decode("日本".as_bytes(), Encoding::Auto).unwrap(), "日本");
    }

    #[test]
    fn sjis_decodes_shift_jis_bytes() {
        // 「日本」in Shift_JIS.
        let sjis = [0x93, 0xfa, 0x96, 0x7b];
        assert_eq!(decode(&sjis, Encoding::Sjis).unwrap(), "日本");
        // Auto falls back to Shift_JIS for non-UTF-8 bytes.
        assert_eq!(decode(&sjis, Encoding::Auto).unwrap(), "日本");
    }

    #[test]
    fn utf8_mode_rejects_shift_jis() {
        let sjis = [0x93, 0xfa, 0x96, 0x7b];
        decode(&sjis, Encoding::Utf8).unwrap_err();
    }

    #[test]
    fn auto_reports_the_detected_encoding() {
        assert_eq!(
            decode_with_encoding("日本".as_bytes(), Encoding::Auto)
                .unwrap()
                .encoding,
            FileEncoding::Utf8,
        );
        assert_eq!(
            decode_with_encoding(&[0x93, 0xfa, 0x96, 0x7b], Encoding::Auto)
                .unwrap()
                .encoding,
            FileEncoding::ShiftJis,
        );
    }

    #[test]
    fn explicit_sjis_preserves_ascii_encoding_intent() {
        assert_eq!(
            decode_with_encoding(b"plain", Encoding::Sjis)
                .unwrap()
                .encoding,
            FileEncoding::ShiftJis,
        );
    }

    #[test]
    fn shift_jis_encoding_is_strict() {
        encode("日本", FileEncoding::ShiftJis).unwrap();
        encode("\u{1f980}", FileEncoding::ShiftJis).unwrap_err();
    }
}
