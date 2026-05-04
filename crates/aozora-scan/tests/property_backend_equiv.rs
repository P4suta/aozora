//! Cross-backend equivalence: every shipping kernel must agree
//! byte-for-byte with [`NaiveScanner`] (the brute-force PHF
//! reference) on every input drawn from the proptest distribution.
//!
//! Each in-source `#[cfg(test)]` proptest pins a single kernel's
//! chunk-level invariant; this integration test closes the
//! end-to-end gap by running every variant of
//! [`aozora_scan::BackendChoice`] over a shared input distribution.
//!
//! Variants whose intrinsics aren't available on the running host
//! (no AVX2 / no SSSE3) are skipped — the dispatcher's `detect()`
//! never picks them, so excluding them from the loop matches
//! production behaviour. The scalar reference is always included
//! because it has no host requirement.

use aozora_proptest::config::default_config;
use aozora_proptest::generators::{aozora_fragment, pathological_aozora, unicode_adversarial};
use aozora_scan::{BackendChoice, NaiveScanner};
use proptest::prelude::*;

fn dispatched_offsets(choice: BackendChoice, source: &str) -> Vec<u32> {
    let mut sink: Vec<u32> = Vec::new();
    choice.scan(source, &mut sink);
    sink
}

/// Every variant the host can actually run must agree with the
/// brute-force naive reference. Unreachable variants (per host CPU
/// feature) are skipped, mirroring what the runtime dispatcher
/// itself observes.
fn assert_every_backend_choice_matches_naive(source: &str) {
    let oracle = NaiveScanner.scan_offsets(source);
    for choice in [
        BackendChoice::TeddyAvx2,
        BackendChoice::TeddySsse3,
        BackendChoice::TeddyNeon,
        BackendChoice::TeddyWasm,
        BackendChoice::ScalarTeddy,
    ] {
        #[cfg(target_arch = "x86_64")]
        {
            if matches!(choice, BackendChoice::TeddyAvx2) && !std::is_x86_feature_detected!("avx2")
            {
                continue;
            }
            if matches!(choice, BackendChoice::TeddySsse3)
                && !std::is_x86_feature_detected!("ssse3")
            {
                continue;
            }
        }
        let actual = dispatched_offsets(choice, source);
        assert_eq!(
            actual,
            oracle,
            "{} diverged from NaiveScanner for input {source:?}",
            choice.name(),
        );
    }
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors — one per trigger kind plus the
// double variants. Cheap unit-style coverage that catches the most
// obvious regressions even when the proptests below are disabled.
// ----------------------------------------------------------------------

#[test]
fn empty_input_yields_no_offsets() {
    assert_every_backend_choice_matches_naive("");
}

#[test]
fn each_trigger_glyph() {
    for src in [
        "｜", "《", "》", "［", "］", "＃", "※", "〔", "〕", "「", "」",
    ] {
        assert_every_backend_choice_matches_naive(src);
    }
}

#[test]
fn double_glyph_sequences() {
    assert_every_backend_choice_matches_naive("《《");
    assert_every_backend_choice_matches_naive("》》");
    assert_every_backend_choice_matches_naive("《《重要》》");
}

#[test]
fn ascii_only_has_zero_triggers() {
    assert_every_backend_choice_matches_naive(&"a".repeat(4096));
}

proptest! {
    #![proptest_config(default_config())]

    /// Every BackendChoice variant agrees with NaiveScanner on every
    /// input drawn from the workhorse `aozora_fragment` distribution.
    #[test]
    fn every_backend_matches_naive_on_aozora_fragment(s in aozora_fragment(120)) {
        assert_every_backend_choice_matches_naive(&s);
    }

    /// Pathological / unbalanced Aozora — same agreement property over
    /// inputs the lex pipeline rejects (the scanner doesn't care about
    /// well-formedness; it only cares about trigger-byte positions).
    #[test]
    fn every_backend_matches_naive_on_pathological_aozora(s in pathological_aozora(120)) {
        assert_every_backend_choice_matches_naive(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA bytes.
    /// The scanner never decodes; it walks the byte buffer. Surfaces
    /// any backend that secretly normalises before scanning.
    #[test]
    fn every_backend_matches_naive_on_unicode_adversarial(s in unicode_adversarial()) {
        assert_every_backend_choice_matches_naive(&s);
    }
}
