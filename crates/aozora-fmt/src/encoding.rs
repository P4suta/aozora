//! Source-encoding selection, shared by both formatter frontends.
//!
//! `aozora-fmt` reads real Aozora Bunko files, which ship as Shift_JIS; the
//! [`Encoding`] value-enum and [`decode`] let the standalone `aozora-fmt`
//! binary and the `aozora fmt` subcommand resolve bytes identically. clap
//! lives here rather than in the lower-level `aozora-encoding` crate, which
//! stays dependency-light.

use std::borrow::Cow;

use anyhow::{Context, Result};
use clap::ValueEnum;

/// How to decode source bytes into text.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
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

/// Decode `raw` under `encoding` into an owned `String`.
///
/// # Errors
///
/// Returns an error when the bytes do not decode under the chosen encoding
/// (invalid UTF-8 for [`Encoding::Utf8`], a Shift_JIS decode failure for
/// [`Encoding::Sjis`], or neither for [`Encoding::Auto`]).
pub fn decode(raw: &[u8], encoding: Encoding) -> Result<String> {
    match encoding {
        Encoding::Auto => aozora_encoding::decode_auto(raw)
            .map(Cow::into_owned)
            .map_err(|e| anyhow::anyhow!("input is neither valid UTF-8 nor Shift_JIS: {e}")),
        Encoding::Utf8 => String::from_utf8(raw.to_vec())
            .map_err(|e| e.utf8_error())
            .context("input is not valid UTF-8 (use --encoding sjis for Aozora Bunko files)"),
        Encoding::Sjis => aozora_encoding::decode_sjis(raw)
            .map_err(|e| anyhow::anyhow!("Shift_JIS decode failed: {e}")),
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
}
