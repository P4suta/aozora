//! Provides the reference scanner for differential tests.

use std::vec::Vec;

use crate::spec::classify_trigger_bytes;

use crate::scan::trait_def::OffsetSink;

const TRIGGER_BYTE_LEN: usize = 3;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NaiveScanner;

impl NaiveScanner {
    #[must_use]
    pub(crate) fn scan_offsets(self, source: &str) -> Vec<u32> {
        let mut sink = Vec::new();
        self.scan(source, &mut sink);
        sink
    }

    #[expect(
        clippy::unused_self,
        reason = "value-method shape keeps this symmetric with scan_offsets so both are \
                  called as NaiveScanner.method(), the zero-sized-handle convention callers use"
    )]
    pub(crate) fn scan<S: OffsetSink>(self, source: &str, sink: &mut S) {
        let bytes = source.as_bytes();
        if bytes.len() < TRIGGER_BYTE_LEN {
            return;
        }
        for i in 0..=bytes.len() - TRIGGER_BYTE_LEN {
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
    use std::vec;

    #[test]
    fn empty_input_yields_nothing() {
        assert_eq!(NaiveScanner.scan_offsets(""), Vec::<u32>::new());
    }

    #[test]
    fn input_too_short_yields_nothing() {
        assert_eq!(NaiveScanner.scan_offsets("ab"), Vec::<u32>::new());
    }

    #[test]
    fn finds_singleton_trigger() {
        assert_eq!(NaiveScanner.scan_offsets("｜"), vec![0]);
    }

    #[test]
    fn finds_triggers_amid_japanese_text() {
        let s = "漢《かん》字";
        assert_eq!(NaiveScanner.scan_offsets(s), vec![3, 12]);
    }

    #[test]
    fn skips_non_trigger_chars_with_same_leading_byte() {
        let s = "あこんにちは、世界！";
        assert_eq!(NaiveScanner.scan_offsets(s), Vec::<u32>::new());
    }
}
