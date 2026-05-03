//! Cross-backend equivalence: every shipping [`TriggerScanner`] backend
//! must agree byte-for-byte with [`NaiveScanner`] on every input.
//!
//! Each backend module already cross-validates itself against
//! `NaiveScanner` via in-source `#[cfg(test)]` proptests; the gap this
//! integration test closes is **dispatcher-side equivalence**:
//!
//! 1. The dispatcher [`best_scanner`] is allowed to pick any backend.
//!    A backend that disagrees with the others would silently change
//!    classification depending on host SIMD support — gated here by
//!    asserting `best_scanner` agrees with `NaiveScanner`.
//! 2. The Teddy / DFA / structural-bitmap / Naive trio is exercised
//!    side-by-side in one file with a single shared input
//!    distribution. A new backend landing in `backends/` would only
//!    pass the per-backend self-check; this integration test is the
//!    one that catches the `naive ≠ teddy` ≠ `dfa` divergence as soon
//!    as a constants-shared mutation lands (e.g. fingerprint byte
//!    indices in Teddy swapped).

use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_scan::{NaiveScanner, TriggerScanner, best_scanner};
use proptest::prelude::*;

#[cfg(feature = "std")]
fn assert_dispatcher_matches_naive(source: &str) {
    let oracle = NaiveScanner.scan_offsets(source);
    let actual = best_scanner().scan_offsets(source);
    assert_eq!(
        actual, oracle,
        "best_scanner output diverged from NaiveScanner for input {source:?}"
    );
}

#[cfg(feature = "std")]
fn assert_all_backends_agree(source: &str) {
    let naive = NaiveScanner.scan_offsets(source);

    if let Some(teddy) = aozora_scan::TeddyScanner::new() {
        assert_eq!(
            teddy.scan_offsets(source),
            naive,
            "TeddyScanner diverged from NaiveScanner for input {source:?}"
        );
    }

    let dfa = aozora_scan::DfaScanner::new();
    assert_eq!(
        dfa.scan_offsets(source),
        naive,
        "DfaScanner diverged from NaiveScanner for input {source:?}"
    );

    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        let sb = aozora_scan::StructuralBitmapScanner;
        assert_eq!(
            sb.scan_offsets(source),
            naive,
            "StructuralBitmapScanner diverged from NaiveScanner for input {source:?}"
        );
    }
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors — one per trigger kind plus the
// double variants. Cheap unit-style coverage that catches regressions
// even when the proptests below are disabled (e.g. `cargo test
// -p aozora-scan --no-default-features`).
// ----------------------------------------------------------------------

#[cfg(feature = "std")]
#[test]
fn empty_input_yields_no_offsets() {
    assert_dispatcher_matches_naive("");
}

#[cfg(feature = "std")]
#[test]
fn each_trigger_glyph() {
    for src in [
        "｜", "《", "》", "［", "］", "＃", "※", "〔", "〕", "「", "」",
    ] {
        assert_dispatcher_matches_naive(src);
        assert_all_backends_agree(src);
    }
}

#[cfg(feature = "std")]
#[test]
fn double_glyph_sequences() {
    assert_dispatcher_matches_naive("《《");
    assert_dispatcher_matches_naive("》》");
    assert_all_backends_agree("《《重要》》");
}

#[cfg(feature = "std")]
#[test]
fn ascii_only_has_zero_triggers() {
    assert_dispatcher_matches_naive(&"a".repeat(4096));
}

#[cfg(feature = "std")]
proptest! {
    #![proptest_config(default_config())]

    /// `best_scanner` agrees with `NaiveScanner` on every input drawn
    /// from the workhorse `aozora_fragment` distribution. The
    /// dispatcher MUST be transparent to backend choice.
    #[test]
    fn best_scanner_matches_naive_on_aozora_fragment(s in aozora_fragment(120)) {
        assert_dispatcher_matches_naive(&s);
    }

    /// All shipping backends agree on the same input. The decisive
    /// killer for fingerprint-byte / lead-byte index swaps in any one
    /// backend.
    #[test]
    fn all_backends_agree_on_aozora_fragment(s in aozora_fragment(120)) {
        assert_all_backends_agree(&s);
    }

    /// Pathological / unbalanced Aozora — same agreement property over
    /// inputs the lex pipeline rejects (the scanner doesn't care about
    /// well-formedness; it only cares about trigger-byte positions).
    #[test]
    fn all_backends_agree_on_pathological_aozora(s in pathological_aozora(120)) {
        assert_all_backends_agree(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// bytes. The scanner never decodes; it walks the byte buffer.
    /// Surfaces any backend that secretly normalises before scanning.
    #[test]
    fn all_backends_agree_on_unicode_adversarial(s in unicode_adversarial()) {
        assert_all_backends_agree(&s);
    }
}
