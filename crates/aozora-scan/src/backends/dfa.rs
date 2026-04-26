//! DFA backend — Hoehrmann-style multi-pattern byte automaton.
//!
//! ## Algorithm
//!
//! Constructs a deterministic finite automaton over UTF-8 bytes whose
//! accepting states correspond to the 11 trigger trigrams. Walks the
//! source byte-by-byte through the DFA in a tight loop; every time
//! the DFA reaches an accepting state, the current position is a
//! trigger end, so we record `pos - 2` as the trigger start.
//!
//! No SIMD. The win over the legacy char-walker is that the DFA
//! transition table is dense (one byte → next state, single L1
//! load), and accepting state checks are branch-predictor-friendly
//! since misses are the common case (`<2 %` of bytes).
//!
//! ## Citation
//!
//! - Bjoern Hoehrmann, "Flexible and Economical UTF-8 Decoder" (2010)
//!   — the canonical small-table-DFA-over-bytes reference. We use
//!   the multi-pattern generalisation packaged by `regex-automata`
//!   (`dfa::regex::Regex::new_many`).
//!
//! ## Role in the bake-off
//!
//! The SIMD-free baseline. If this gets within 2× of Teddy on the
//! `tokenize_compare` bench then the SIMD complexity isn't worth
//! it. Also serves as the universal fallback for
//! [`crate::best_scanner`] on hosts where Teddy's SSSE3 build
//! returns `None`.

use alloc::sync::Arc;
use alloc::vec::Vec;

use regex_automata::Input;
use regex_automata::dfa::regex::Regex;

use aozora_spec::trigger::ALL_TRIGGER_TRIGRAMS;

use crate::TriggerScanner;

/// Multi-pattern DFA scanner. Holds an `Arc<Regex>` so the scanner
/// is `Clone + Send + Sync` and the (~constant-cost) DFA build
/// runs once via the `OnceLock<DfaScanner>` in `lib.rs::best_scanner`.
#[derive(Debug, Clone)]
pub struct DfaScanner {
    regex: Arc<Regex>,
}

impl DfaScanner {
    /// Build a `DfaScanner` over the 11 trigger trigrams.
    ///
    /// Each trigger character is a valid UTF-8 string and contains no
    /// regex metacharacters in the default Unicode syntax, so we feed
    /// them in as-is.
    ///
    /// # Panics
    ///
    /// Panics if `regex-automata` rejects the pattern set. With our
    /// fixed 11 trigger trigrams this can't happen at runtime; the
    /// `OnceLock<DfaScanner>` upstream means we'd see the panic at
    /// process start the very first time `best_scanner()` is called.
    #[must_use]
    pub fn new() -> Self {
        // Convert the trigrams to &str — every entry is by construction
        // valid UTF-8 (it's the byte form of a BMP codepoint).
        // `regex_automata::dfa::regex::Regex::new_many` then builds
        // the dense DFA over these literal patterns.
        let triggers: alloc::vec::Vec<&str> = ALL_TRIGGER_TRIGRAMS
            .iter()
            .map(|trigram| {
                core::str::from_utf8(trigram)
                    .expect("trigger trigrams are valid UTF-8 by construction")
            })
            .collect();
        let regex =
            Regex::new_many(&triggers).expect("regex-automata accepts our 11 literal patterns");
        Self {
            regex: Arc::new(regex),
        }
    }
}

impl Default for DfaScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl TriggerScanner for DfaScanner {
    fn scan_offsets(&self, source: &str) -> Vec<u32> {
        let mut out = Vec::with_capacity(source.len() / 1000);
        for m in self.regex.find_iter(Input::new(source.as_bytes())) {
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
    use crate::NaiveScanner;
    use alloc::string::String;
    use alloc::vec;

    #[test]
    fn empty_input_yields_nothing() {
        assert!(DfaScanner::new().scan_offsets("").is_empty());
    }

    #[test]
    fn finds_each_singleton_trigger() {
        let dfa = DfaScanner::new();
        for trigger in [
            "｜", "《", "》", "［", "］", "＃", "※", "〔", "〕", "「", "」",
        ] {
            assert_eq!(dfa.scan_offsets(trigger), vec![0], "{trigger}");
        }
    }

    #[test]
    fn finds_triggers_amid_japanese_text() {
        let dfa = DfaScanner::new();
        assert_eq!(dfa.scan_offsets("漢《かん》字"), vec![3, 12]);
    }

    #[test]
    fn double_ruby_yields_two_adjacent_offsets() {
        let dfa = DfaScanner::new();
        let s = "《《X》》";
        let got = dfa.scan_offsets(s);
        let want = NaiveScanner.scan_offsets(s);
        assert_eq!(got, want);
        assert_eq!(got, vec![0, 3, 7, 10]);
    }

    #[test]
    fn matches_naive_on_long_japanese_input() {
        let dfa = DfaScanner::new();
        let s = "あこんにちは、世界！漢《かん》字、※［＃ここまで］".repeat(50);
        assert_eq!(dfa.scan_offsets(&s), NaiveScanner.scan_offsets(&s));
    }

    #[test]
    fn matches_naive_at_chunk_boundaries() {
        let dfa = DfaScanner::new();
        for n in [15usize, 16, 17, 31, 32, 33, 63, 64, 65, 95, 96, 97] {
            let mut s = String::with_capacity(n + 16);
            for _ in 0..n {
                s.push('x');
            }
            s.push_str("｜tail");
            assert_eq!(dfa.scan_offsets(&s), NaiveScanner.scan_offsets(&s), "n={n}");
        }
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
            let dfa = DfaScanner::new();
            let got = dfa.scan_offsets(&s);
            let want = NaiveScanner.scan_offsets(&s);
            prop_assert_eq!(got, want);
        }
    }
}
