//! Brute-force ground-truth scanner.
//!
//! Walks every byte position and asks the PHF whether the 3-byte
//! window starting there is a recognised trigger. No SIMD, no
//! candidate filter, no chunking — just the obvious O(n × PHF)
//! reference against which every clever backend is cross-checked
//! via proptest.
//!
//! Useful only as a test fixture: the cleverer backends share
//! constants (the trigger leading-byte set, the middle-byte set,
//! the PHF) so two of them can silently agree on a wrong answer.
//! `NaiveScanner` is the independent reference that closes that
//! loophole.

use alloc::vec::Vec;

use aozora_spec::classify_trigger_bytes;

use crate::TriggerScanner;

/// Test-only brute-force reference scanner.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NaiveScanner;

impl TriggerScanner for NaiveScanner {
    fn scan_offsets(&self, source: &str) -> Vec<u32> {
        let bytes = source.as_bytes();
        let mut out = Vec::new();
        if bytes.len() < 3 {
            return out;
        }
        for i in 0..=bytes.len() - 3 {
            let window: [u8; 3] = [bytes[i], bytes[i + 1], bytes[i + 2]];
            if classify_trigger_bytes(window).is_some() {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "lex pipeline asserts source ≤ u32::MAX upstream"
                )]
                out.push(i as u32);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn empty_input_yields_nothing() {
        assert!(NaiveScanner.scan_offsets("").is_empty());
    }

    #[test]
    fn input_too_short_yields_nothing() {
        assert!(NaiveScanner.scan_offsets("ab").is_empty());
    }

    #[test]
    fn finds_singleton_trigger() {
        assert_eq!(NaiveScanner.scan_offsets("｜"), vec![0]);
    }

    #[test]
    fn finds_triggers_amid_japanese_text() {
        // 漢《かん》字 — 《 at byte 3, 》 at byte 12.
        let s = "漢《かん》字";
        assert_eq!(NaiveScanner.scan_offsets(s), vec![3, 12]);
    }

    #[test]
    fn skips_non_trigger_chars_with_same_leading_byte() {
        // hiragana 'あこ…' all start with 0xE3 but classify as None.
        let s = "あこんにちは、世界！";
        assert!(NaiveScanner.scan_offsets(s).is_empty());
    }
}
