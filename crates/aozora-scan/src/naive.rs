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

use crate::trait_def::OffsetSink;

/// Test-only brute-force reference scanner.
///
/// `pub` so the in-crate proptests and the cross-validation tests
/// in `tests/` can reach it without a `bench-baselines` feature
/// flag, but `#[doc(hidden)]` because external callers should
/// always pick a real backend (`BackendChoice`).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NaiveScanner;

impl NaiveScanner {
    /// Scan `source` and return every trigger byte offset.
    ///
    /// Convenience wrapper around [`Self::scan`] that allocates a
    /// fresh `Vec<u32>`. Test paths use this; production paths go
    /// through [`crate::scan_offsets_in`] / [`crate::BackendChoice`].
    #[doc(hidden)]
    #[must_use]
    pub fn scan_offsets(self, source: &str) -> Vec<u32> {
        let mut sink = Vec::new();
        self.scan(source, &mut sink);
        sink
    }

    /// Sink-based variant: write every trigger byte offset into
    /// `sink` in source order.
    #[doc(hidden)]
    pub fn scan<S: OffsetSink>(self, source: &str, sink: &mut S) {
        let bytes = source.as_bytes();
        if bytes.len() < 3 {
            return;
        }
        for i in 0..=bytes.len() - 3 {
            let window: [u8; 3] = [bytes[i], bytes[i + 1], bytes[i + 2]];
            if classify_trigger_bytes(window).is_some() {
                let offset = u32::try_from(i).expect("source longer than u32::MAX is unsupported");
                sink.push(offset);
            }
        }
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
