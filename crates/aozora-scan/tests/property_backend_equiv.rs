//! Differential oracle: the production scanner ([`scan_offsets`], now
//! `aho-corasick`-backed) must agree byte-for-byte with [`NaiveScanner`]
//! (the brute-force `classify`-per-byte reference) on every input drawn
//! from the proptest distribution.
//!
//! This is the anti-drift guard for the unsafe→safe scanner swap: the
//! naive walker and the packed matcher derive their trigger set from the
//! same `aozora_spec` constant, and this test proves they never diverge
//! on real or adversarial input.

use aozora_proptest::config::default_config;
use aozora_proptest::generators::{aozora_fragment, pathological_aozora, unicode_adversarial};
use aozora_scan::{NaiveScanner, scan_offsets};
use proptest::prelude::*;

/// The production scanner and the brute-force naive reference must
/// produce identical offset vectors for any input.
fn assert_scan_matches_naive(source: &str) {
    let production = scan_offsets(source);
    let oracle = NaiveScanner.scan_offsets(source);
    assert_eq!(
        production, oracle,
        "scan_offsets diverged from NaiveScanner for input {source:?}",
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors — one per trigger kind plus the
// double variants. Cheap unit-style coverage that catches the most
// obvious regressions even when the proptests below are disabled.
// ----------------------------------------------------------------------

#[test]
fn empty_input_yields_no_offsets() {
    assert_scan_matches_naive("");
}

#[test]
fn each_trigger_glyph() {
    for src in [
        "｜", "《", "》", "［", "］", "＃", "※", "〔", "〕", "「", "」",
    ] {
        assert_scan_matches_naive(src);
    }
}

#[test]
fn double_glyph_sequences() {
    assert_scan_matches_naive("《《");
    assert_scan_matches_naive("》》");
    assert_scan_matches_naive("《《重要》》");
}

#[test]
fn ascii_only_has_zero_triggers() {
    assert_scan_matches_naive(&"a".repeat(4096));
}

#[test]
fn kana_sharing_e3_lead_byte_yields_zero_triggers() {
    // The motivation for the rewrite: every hiragana/katakana codepoint
    // leads with 0xE3 (like the 《》「」〔〕 triggers), but is not a
    // trigger. Both scanners must skip it.
    assert_scan_matches_naive("あいうえおカキクケコこんにちは漢字");
}

proptest! {
    #![proptest_config(default_config())]

    /// Agreement on the workhorse `aozora_fragment` distribution.
    #[test]
    fn scan_matches_naive_on_aozora_fragment(s in aozora_fragment(120)) {
        assert_scan_matches_naive(&s);
    }

    /// Pathological / unbalanced Aozora — same agreement property over
    /// inputs the lex pipeline rejects (the scanner doesn't care about
    /// well-formedness; it only cares about trigger-byte positions).
    #[test]
    fn scan_matches_naive_on_pathological_aozora(s in pathological_aozora(120)) {
        assert_scan_matches_naive(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA bytes.
    /// The scanner never decodes; it walks the byte buffer.
    #[test]
    fn scan_matches_naive_on_unicode_adversarial(s in unicode_adversarial()) {
        assert_scan_matches_naive(&s);
    }
}
