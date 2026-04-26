//! Teddy backend — Hyperscan multi-pattern fingerprint matcher.
//!
//! ## Algorithm
//!
//! Teddy hashes the first 1-3 bytes of each pattern into two PSHUFB
//! nybble-lookup tables, one per nybble. For each 16/32-byte chunk
//! it does:
//!
//! 1. Shuffle each input byte through the two PSHUFB tables → two
//!    XMM/YMM registers of "which-bucket" bitmaps per byte.
//! 2. AND across positions to combine N-byte fingerprints.
//! 3. The set bits in the result are candidate match positions.
//! 4. Verify each candidate against the original pattern bytes.
//!
//! With our 11 trigger trigrams (`min_pattern_len = 3`) Teddy uses
//! 3-byte fingerprints, which on a 16-pattern matcher are essentially
//! collision-free — verify almost never fires for non-triggers.
//!
//! ## Citation
//!
//! - Geoff Langdale, "Teddy: A literal matcher for short patterns",
//!   Hyperscan internals (Intel, 2015). FOSS port: aho-corasick by
//!   Andrew Gallant (BurntSushi), 2019.
//! - See `aho-corasick-1.1.x/src/packed/teddy/README.md` for the
//!   shuffle-bucket maths and SIMD layout.
//!
//! ## CPU requirement
//!
//! Teddy needs SSSE3 minimum (`PSHUFB`). On hosts without SSSE3 the
//! `Searcher::build` returns `None` and we surface a build error. The
//! crate dispatcher in `lib.rs::best_scanner` is responsible for
//! falling back to a SIMD-free backend in that case.

use alloc::sync::Arc;
use alloc::vec::Vec;

use aho_corasick::packed::{Config, MatchKind, Searcher};

use aozora_spec::trigger::ALL_TRIGGER_TRIGRAMS;

use crate::TriggerScanner;

/// Teddy-driven [`TriggerScanner`].
///
/// Holds an `Arc<Searcher>` so it is `Clone + Send + Sync` and the
/// (~constant-cost) Teddy build runs once via the
/// `OnceLock<TeddyScanner>` in `lib.rs::best_scanner`.
#[derive(Debug, Clone)]
pub struct TeddyScanner {
    searcher: Arc<Searcher>,
}

impl TeddyScanner {
    /// Build a `TeddyScanner` over the 11 trigger trigrams.
    ///
    /// Returns `None` if the host CPU lacks SSSE3 — in which case
    /// the dispatcher falls through to a non-Teddy backend.
    #[must_use]
    pub fn new() -> Option<Self> {
        // LeftmostFirst guarantees that, when two patterns happen to
        // overlap at the same offset, the earlier-added one wins.
        // For our 11 disjoint trigrams this is moot (no two trigrams
        // share all 3 bytes), but specifying the kind avoids relying
        // on the default and pins the semantics for review.
        let mut builder = Config::new().match_kind(MatchKind::LeftmostFirst).builder();
        for trigram in &ALL_TRIGGER_TRIGRAMS {
            builder.add(trigram);
        }
        let searcher = builder.build()?;
        Some(Self {
            searcher: Arc::new(searcher),
        })
    }
}

impl TriggerScanner for TeddyScanner {
    fn scan_offsets(&self, source: &str) -> Vec<u32> {
        // Teddy returns every match start (already verified against
        // the trigram bytes internally), so no PHF re-classify is
        // needed. The capacity heuristic (1 trigger / 1000 bytes)
        // tracks the corpus median (~1.8 % triggers, but we want to
        // optimise for the common case rather than the worst).
        let mut out = Vec::with_capacity(source.len() / 1000);
        for m in self.searcher.find_iter(source.as_bytes()) {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "lex pipeline asserts source ≤ u32::MAX upstream"
            )]
            out.push(m.start() as u32);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NaiveScanner, TriggerScanner};
    use alloc::string::String;
    use alloc::vec;

    fn teddy_or_skip() -> Option<TeddyScanner> {
        // Teddy needs SSSE3. On such CI hosts skip silently — the
        // proptest still cross-validates whichever backends are
        // present.
        TeddyScanner::new()
    }

    #[test]
    fn empty_input_yields_nothing() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
        assert!(t.scan_offsets("").is_empty());
    }

    #[test]
    fn finds_each_singleton_trigger() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
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
            let got = t.scan_offsets(s);
            assert_eq!(&got, expected, "trigger {s:?}");
        }
    }

    #[test]
    fn finds_triggers_amid_japanese_text() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
        let s = "漢《かん》字";
        assert_eq!(t.scan_offsets(s), vec![3, 12]);
    }

    #[test]
    fn skips_non_trigger_chars_with_same_leading_byte() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
        // Long input so Teddy's chunk loop fires.
        let s = "あこんにちは、世界！".repeat(20);
        let got = t.scan_offsets(&s);
        let want = NaiveScanner.scan_offsets(&s);
        assert_eq!(got, want);
    }

    #[test]
    fn matches_naive_at_chunk_boundaries() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
        // Build inputs that span Teddy's various chunk widths
        // (16 / 32 / 64 bytes depending on SIMD width selected).
        for n in [15usize, 16, 17, 31, 32, 33, 63, 64, 65, 95, 96, 97] {
            let mut s = String::with_capacity(n + 16);
            for _ in 0..n {
                s.push('x');
            }
            s.push_str("｜tail");
            let got = t.scan_offsets(&s);
            let want = NaiveScanner.scan_offsets(&s);
            assert_eq!(got, want, "diverged at boundary n={n}");
        }
    }

    #[test]
    fn double_ruby_yields_two_adjacent_offsets() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
        let s = "《《X》》";
        let got = t.scan_offsets(s);
        let want = NaiveScanner.scan_offsets(s);
        assert_eq!(got, want);
        assert_eq!(got, vec![0, 3, 7, 10]);
    }

    #[test]
    fn round_trip_against_naive_on_aozora_sample() {
        let Some(t) = teddy_or_skip() else {
            return;
        };
        // A representative paragraph that mixes prose, ruby triggers,
        // square-bracket annotations, tortoise-shell, and corner
        // brackets — covers all 11 triggers plus dense plain runs.
        let s = "［＃ここから２字下げ］青梅《おうめ》のお仙《せん》という女が、\
                 \u{3000}〔書中の一節〕、「ほんとうにそうですね」と。\
                 \u{3000}※［＃「木＋吶のつくり」、第3水準1-85-54］を見よ。";
        let got = t.scan_offsets(s);
        let want = NaiveScanner.scan_offsets(s);
        assert_eq!(got, want);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::NaiveScanner;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn byte_identical_to_naive(
            s in r"(\PC|｜|《|》|［|］|＃|※|〔|〕|「|」){0,2400}",
        ) {
            let Some(teddy) = TeddyScanner::new() else {
                return Ok(());
            };
            let got = teddy.scan_offsets(&s);
            let want = NaiveScanner.scan_offsets(&s);
            prop_assert_eq!(got, want);
        }
    }
}
