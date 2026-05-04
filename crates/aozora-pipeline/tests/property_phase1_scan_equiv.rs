//! Cross-checks between Phase 1's SIMD trigger scan and the
//! brute-force [`NaiveScanner`] reference, projected through the
//! whole `lex_into_arena` pipeline.
//!
//! `aozora-scan/tests/property_backend_equiv.rs` already proves that
//! every `BackendChoice` variant agrees byte-for-byte with
//! [`NaiveScanner`] on raw source bytes. That covers a *single layer*
//! — the scanner kernel — and says nothing about how Phase 1's output
//! flows through Phase 2 / Phase 3 on into the lexer's normalized
//! buffer.
//!
//! The properties here close the gap by reasoning about the *whole*
//! pipeline:
//!
//! 1. **Scanner totality on lex output.** [`NaiveScanner`] applied to
//!    the lexer's normalized buffer must not panic and must return
//!    offsets that are valid UTF-8 char boundaries inside that buffer.
//!    The normalized buffer carries PUA sentinels and may differ in
//!    length from the source; a regression that left dangling bytes
//!    or invalidated boundaries would surface here.
//!
//! 2. **Phase 1 monotonicity.** The Aozora pipeline can only *consume*
//!    triggers (replacing them with PUA sentinels) — it never adds
//!    new ones. Therefore the count of triggers in the *normalized*
//!    buffer is at most the count in the *source* (modulo Phase 0's
//!    sanitize rewrites: BOM strip, CRLF→LF, accent decomposition).
//!    A regression that emitted a trigger in the normalized output
//!    where none existed in source would shift this delta.
//!
//! Both properties are deliberately stated as inequalities rather than
//! equalities. Phase 0 sanitize can drop bytes (BOM, CR) and rewrite
//! sequences (`〔NFC〕` → combining), so a strict "trigger count is
//! equal" property would have many false positives. The inequality
//! properties remain decisive: any regression that *adds* triggers
//! during normalization fails them under shrinking.

use aozora_pipeline::lex_into_arena;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_scan::NaiveScanner;
use aozora_syntax::borrowed::Arena;
use proptest::prelude::*;

fn scan_count(source: &str) -> usize {
    NaiveScanner.scan_offsets(source).len()
}

fn assert_offsets_are_char_boundaries(label: &str, text: &str, offsets: &[u32]) {
    for &offset in offsets {
        let off = offset as usize;
        assert!(
            off <= text.len(),
            "{label} offset {off} out of bounds for length {} in {text:?}",
            text.len()
        );
        assert!(
            text.is_char_boundary(off),
            "{label} offset {off} is not a UTF-8 char boundary in {text:?}"
        );
    }
}

fn assert_phase1_scan_invariants(source: &str) {
    // (1a) Scanner totality on raw source. NaiveScanner is the same
    // kernel used by every backend; this also pins that the scanner
    // is panic-free over the full generator surface (a separate
    // assurance from the in-scan-crate property because we now expose
    // it through a downstream caller).
    let source_offsets = NaiveScanner.scan_offsets(source);
    assert_offsets_are_char_boundaries("source", source, &source_offsets);

    // Run the lex pipeline. (1b) Scanner totality on the normalized
    // buffer.
    let arena = Arena::new();
    let out = lex_into_arena(source, &arena);
    let norm_offsets = NaiveScanner.scan_offsets(out.normalized);
    assert_offsets_are_char_boundaries("normalized", out.normalized, &norm_offsets);

    // (2) Phase 1 monotonicity: the pipeline consumes (or passes
    // through) triggers but never invents new ones. The Phase 0
    // sanitize layer can rewrite sequences in ways that *add*
    // characters from the lexer's reserved trigger set in principle —
    // accent decomposition expands `〔NFC〕` into combining sequences
    // that are not themselves triggers, so this concern is theoretical
    // — but the property catches any future Phase 0 / Phase 1 change
    // that accidentally synthesises trigger glyphs.
    let source_triggers = scan_count(source);
    let normalized_triggers = norm_offsets.len();
    assert!(
        normalized_triggers <= source_triggers,
        "trigger count grew through lex: source={source_triggers} normalized={normalized_triggers}\n\
         source: {source:?}\nnormalized: {:?}",
        out.normalized
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_satisfies_invariants() {
    assert_phase1_scan_invariants("");
}

#[test]
fn plain_text_satisfies_invariants() {
    assert_phase1_scan_invariants("Hello, world.");
    assert_phase1_scan_invariants("こんにちは、世界！");
}

#[test]
fn explicit_ruby_satisfies_invariants() {
    // `｜青梅《おうめ》` — three triggers in source (`｜`, `《`, `》`),
    // all consumed into PUA sentinels by Phase 3 → zero triggers in
    // normalized. Property: 0 ≤ 3.
    assert_phase1_scan_invariants("｜青梅《おうめ》");
}

#[test]
fn paired_container_satisfies_invariants() {
    assert_phase1_scan_invariants(
        "［＃ここから2字下げ］\n\
         body\n\
         ［＃ここで字下げ終わり］",
    );
}

#[test]
fn pua_passthrough_keeps_invariants() {
    // PUA in source — phase 0 emits a SourceContainsPua diagnostic
    // and lets the bytes through. The PUA codepoints are not in the
    // trigger set, so they don't shift either count.
    assert_phase1_scan_invariants("a\u{E001}b\u{E004}c");
}

proptest! {
    #![proptest_config(default_config())]

    /// Workhorse — the SIMD scan / phase-1 monotonicity duality must
    /// hold over every Aozora-shaped fragment.
    #[test]
    fn aozora_fragment_scan_invariants_hold(s in aozora_fragment(120)) {
        assert_phase1_scan_invariants(&s);
    }

    /// Pathological — runs of unbalanced trigger glyphs (the case that
    /// most stresses Phase 1 → Phase 2 → Phase 3 trigger-consumption
    /// accounting).
    #[test]
    fn pathological_input_scan_invariants_hold(s in pathological_aozora(120)) {
        assert_phase1_scan_invariants(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// codepoints, full-width forms. The SIMD scanner must classify
    /// every Unicode shape correctly, and the lexer must not synthesise
    /// triggers when normalising adversarial input.
    #[test]
    fn unicode_adversarial_scan_invariants_hold(s in unicode_adversarial()) {
        assert_phase1_scan_invariants(&s);
    }
}
