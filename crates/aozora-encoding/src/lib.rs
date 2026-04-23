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
/// Used by the CLI to strip the BOM before handing input to the parser. A full
/// encoding sniffer (BOM + byte-frequency heuristic) is deferred to M2 when we
/// actually need to accept unknown-encoding input streams; today the CLI uses an
/// explicit `--encoding` flag, so BOM presence is the only runtime signal we care
/// about.
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

pub mod gaiji {
    //! Gaiji (外字) resolution.
    //!
    //! Two incoming forms:
    //!   - `※［＃「component」、第3水準1-85-54］`  — JIS X 0213 plane/row/cell
    //!   - `※［＃「component」、U+XXXX、page-line］` — explicit Unicode codepoint

    use afm_syntax::Gaiji;

    /// Outcome of resolving a gaiji reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Resolution {
        /// The canonical representation: `Some(ch)` if we resolved to a single char,
        /// `None` if the gaiji has no Unicode home and must render as its description.
        pub character: Option<char>,
        /// Echo of the input description, preserved for HTML `title` / accessibility.
        pub description: String,
    }

    /// Resolve a `Gaiji` node to a displayable form.
    ///
    /// For M0 we accept whatever `ucs` the parser already extracted; the JIS-to-Unicode
    /// lookup table lands in M2. This stub exists to pin the API surface early so
    /// callers don't need to change later.
    #[must_use]
    pub fn resolve(node: &Gaiji) -> Resolution {
        Resolution {
            character: node.ucs,
            description: node.description.clone(),
        }
    }
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
        // 「青空文庫」 in Shift_JIS
        let bytes = &[0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        assert_eq!(decode_sjis(bytes).unwrap(), "青空文庫");
    }

    #[test]
    fn rejects_malformed_sjis() {
        // Invalid lead byte
        let bytes = &[0xFF, 0xFF];
        assert!(matches!(
            decode_sjis(bytes),
            Err(DecodeError::ShiftJisInvalid)
        ));
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
}
