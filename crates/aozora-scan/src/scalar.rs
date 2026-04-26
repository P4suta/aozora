//! Scalar `memchr3`-based trigger scanner.
//!
//! Walks the source bytes, jumping over runs of non-candidate bytes
//! via `memchr::memchr3`, then validating each candidate via the
//! const-PHF lookup in [`aozora_spec::classify_trigger_bytes`].
//!
//! `memchr` itself already vectorises (AVX2 on `x86_64` / NEON on
//! aarch64) so this scalar implementation is closer to "vectorised
//! candidate finder + scalar precise classify" than to a hand-rolled
//! byte-by-byte loop. It still benefits from the future `Avx2Scanner`
//! / `NeonScanner` / `WasmSimdScanner` because those will combine the
//! candidate find AND the precise classify into a single pass via
//! structural bitmaps + `pext`.

use alloc::vec::Vec;

use aozora_spec::trigger::TRIGGER_LEADING_BYTES;
use aozora_spec::{TriggerKind, classify_trigger_bytes};

use crate::TriggerScanner;

/// Always-available scalar scanner. Internally `memchr3`-driven;
/// stateless and `Copy`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScalarScanner;

impl TriggerScanner for ScalarScanner {
    fn scan_offsets(&self, source: &str) -> Vec<u32> {
        scan_offsets_scalar(source.as_bytes())
    }
}

impl ScalarScanner {
    /// Scan and also classify each candidate, returning `(offset,
    /// kind)` pairs. Useful for callers that want to fuse the two
    /// steps; the lex driver typically prefers `scan_offsets` so it
    /// can interleave plain-text emission.
    #[must_use]
    pub fn scan_offsets_and_kinds(source: &str) -> Vec<(u32, TriggerKind)> {
        let bytes = source.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 256);
        for offset in scan_offsets_scalar(bytes) {
            let i = offset as usize;
            // `scan_offsets_scalar` only yields offsets that have
            // i + 3 ≤ bytes.len() and that classify_trigger_bytes
            // recognised, so this slice + classify pair is always Some.
            let window: [u8; 3] = [bytes[i], bytes[i + 1], bytes[i + 2]];
            if let Some(kind) = classify_trigger_bytes(window) {
                out.push((offset, kind));
            }
        }
        out
    }
}

/// Inner scan loop shared by both entry points. Operates on raw bytes
/// (no UTF-8 decoding required: every trigger is exactly 3 bytes).
fn scan_offsets_scalar(bytes: &[u8]) -> Vec<u32> {
    // Heuristic capacity: median Aozora doc has ~1 trigger per 200
    // bytes (corpus 2026-04-25 measurement). One extra alloc on a
    // pathological dense input is cheap.
    let mut out = Vec::with_capacity(bytes.len() / 200);
    scan_offsets_scalar_with_offset(bytes, 0, &mut out);
    out
}

/// Variant that scans only `bytes[start..]` and appends the
/// (absolute, in `bytes`) offsets to `out`. Exposed as
/// `pub(crate)` so the SIMD backends can hand off the < 32-byte
/// tail to the scalar path without duplicating the classify logic.
pub(crate) fn scan_offsets_scalar_with_offset(bytes: &[u8], start: usize, out: &mut Vec<u32>) {
    if start >= bytes.len() {
        return;
    }
    let [needle0, needle1, needle2] = TRIGGER_LEADING_BYTES;
    let finder = memchr::memchr3_iter(needle0, needle1, needle2, &bytes[start..]);

    for cand_rel in finder {
        let cand = start + cand_rel;
        // Need 3 bytes for the trigger window. If we hit a leading
        // byte too close to EOF, that byte is mid-character and
        // cannot be a trigger.
        if cand + 3 > bytes.len() {
            continue;
        }
        let window: [u8; 3] = [bytes[cand], bytes[cand + 1], bytes[cand + 2]];
        if classify_trigger_bytes(window).is_some() {
            // memchr3_iter is monotone-ascending and the cast is
            // bounded by source.len() ≤ u32::MAX (asserted at the
            // lex-layer entry).
            #[allow(
                clippy::cast_possible_truncation,
                reason = "lexer pipeline asserts source ≤ u32::MAX upstream"
            )]
            out.push(cand as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn scan_empty_input_yields_no_offsets() {
        assert!(ScalarScanner.scan_offsets("").is_empty());
    }

    #[test]
    fn scan_plain_japanese_with_no_triggers_is_empty() {
        // "こんにちは" — all hiragana, no triggers, but they DO start
        // with 0xE3 (the ruby-open prefix), so the scanner must
        // exercise the precise-classify reject path.
        let s = "こんにちは";
        let offsets = ScalarScanner.scan_offsets(s);
        assert!(offsets.is_empty(), "got {offsets:?}");
    }

    #[test]
    fn scan_finds_each_singleton_trigger() {
        let cases: &[(&str, &[u32])] = &[
            ("｜", &[0]),
            ("《", &[0]),
            ("》", &[0]),
            ("［", &[0]),
            ("］", &[0]),
            ("＃", &[0]),
            ("※", &[0]),
            ("〔", &[0]),
            ("〕", &[0]),
            ("「", &[0]),
            ("」", &[0]),
        ];
        for (s, expected) in cases {
            let got = ScalarScanner.scan_offsets(s);
            assert_eq!(
                &got, expected,
                "trigger {s:?}: expected {expected:?}, got {got:?}"
            );
        }
    }

    #[test]
    fn scan_finds_triggers_amid_plain_text() {
        // "abc｜def" → ｜ at byte 3
        let s = "abc｜def";
        let offsets = ScalarScanner.scan_offsets(s);
        assert_eq!(offsets, vec![3]);
    }

    #[test]
    fn scan_finds_multiple_triggers_in_order() {
        // Each Japanese char is 3 bytes UTF-8.
        // 漢《かん》字  →  漢=0..3, 《=3..6, かん=6..12, 》=12..15, 字=15..18
        let s = "漢《かん》字";
        let offsets = ScalarScanner.scan_offsets(s);
        assert_eq!(offsets, vec![3, 12]);
    }

    #[test]
    fn scan_yields_two_adjacent_offsets_for_double_ruby() {
        // 《《X》》 byte map (each Japanese char is 3 bytes):
        //   《 = 0..3
        //   《 = 3..6
        //   X = 6..7   (ASCII)
        //   》 = 7..10
        //   》 = 10..13
        let s = "《《X》》";
        let offsets = ScalarScanner.scan_offsets(s);
        assert_eq!(offsets, vec![0, 3, 7, 10]);
    }

    #[test]
    fn scan_handles_truncated_trigger_byte_at_end() {
        // A single 0xE3 byte at end-of-input cannot be a 3-byte
        // trigger. We pad with garbage bytes before/after to make
        // sure the loop's tail logic doesn't false-positive.
        // Construct as bytes: "abc\xE3" — invalid UTF-8 so we go
        // through the bytes route directly.
        let bytes: &[u8] = b"abc\xE3";
        let offsets = scan_offsets_scalar(bytes);
        assert!(offsets.is_empty(), "got {offsets:?}");
    }

    #[test]
    fn scan_offsets_and_kinds_returns_classified_pairs() {
        let s = "漢《かん》字";
        let pairs = ScalarScanner::scan_offsets_and_kinds(s);
        assert_eq!(
            pairs,
            vec![(3, TriggerKind::RubyOpen), (12, TriggerKind::RubyClose)]
        );
    }

    #[test]
    fn scan_offsets_match_classify_for_every_singleton() {
        // For each trigger, build a buffer "X{trigger}Y" and confirm
        // the scanner returns exactly the trigger's offset and the
        // classify table agrees.
        let triggers: &[(&str, TriggerKind)] = &[
            ("｜", TriggerKind::Bar),
            ("《", TriggerKind::RubyOpen),
            ("》", TriggerKind::RubyClose),
            ("［", TriggerKind::BracketOpen),
            ("］", TriggerKind::BracketClose),
            ("＃", TriggerKind::Hash),
            ("※", TriggerKind::RefMark),
            ("〔", TriggerKind::TortoiseOpen),
            ("〕", TriggerKind::TortoiseClose),
            ("「", TriggerKind::QuoteOpen),
            ("」", TriggerKind::QuoteClose),
        ];
        for (trigger, expected_kind) in triggers {
            let buf = alloc::format!("X{trigger}Y");
            let offsets = ScalarScanner.scan_offsets(&buf);
            assert_eq!(offsets.len(), 1, "{trigger}: {offsets:?}");
            let off = offsets[0] as usize;
            let bytes = buf.as_bytes();
            let window = [bytes[off], bytes[off + 1], bytes[off + 2]];
            assert_eq!(classify_trigger_bytes(window), Some(*expected_kind));
        }
    }

    #[test]
    fn scan_skips_non_trigger_chars_with_same_leading_byte() {
        // 'あ' = E3 81 82, 'こ' = E3 81 93, 'ん' = E3 82 93 — all
        // share leading byte 0xE3 with several triggers but none
        // classify as triggers.
        let s = "あこんにちは、世界！";
        let offsets = ScalarScanner.scan_offsets(s);
        assert!(offsets.is_empty(), "got {offsets:?}");
    }

    #[test]
    fn scan_dense_triggers_yields_every_offset_in_order() {
        let s = "《》《》《》";
        let offsets = ScalarScanner.scan_offsets(s);
        assert_eq!(offsets, vec![0, 3, 6, 9, 12, 15]);
    }

    #[test]
    fn best_scanner_returns_a_working_scanner() {
        let s = "漢《かん》字";
        let offsets = crate::best_scanner().scan_offsets(s);
        assert_eq!(offsets, vec![3, 12]);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    // Property: every offset returned by the scanner classifies as
    // a real trigger; every byte position NOT in the output set
    // must NOT classify as a trigger (no false negatives).
    proptest! {
        #[test]
        fn scan_is_consistent_with_classify_table(
            s in r"(\PC|｜|《|》|［|］|＃|※|〔|〕|「|」){0,200}",
        ) {
            let scanner = ScalarScanner;
            let offsets = scanner.scan_offsets(&s);
            let bytes = s.as_bytes();

            // 1. Every offset is a trigger window.
            for &off in &offsets {
                let i = off as usize;
                prop_assert!(i + 3 <= bytes.len());
                let window = [bytes[i], bytes[i + 1], bytes[i + 2]];
                prop_assert!(classify_trigger_bytes(window).is_some(),
                    "offset {} is in output but doesn't classify", off);
            }

            // 2. Offsets are strictly ascending.
            for w in offsets.windows(2) {
                prop_assert!(w[0] < w[1]);
            }

            // 3. No false negatives: any byte position that classifies
            //    must appear in the output.
            let want: Vec<u32> = (0..bytes.len().saturating_sub(2))
                .filter_map(|i| {
                    let window = [bytes[i], bytes[i + 1], bytes[i + 2]];
                    if classify_trigger_bytes(window).is_some() {
                        u32::try_from(i).ok()
                    } else {
                        None
                    }
                })
                .collect();
            prop_assert_eq!(offsets, want);
        }
    }
}
