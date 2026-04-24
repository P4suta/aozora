//! Property test — "must-never-be" invariants for rendered HTML shape.
//!
//! Runs every tier-A/B/D/E/G/H/I/J/K/L predicate from
//! [`afm_parser::test_support`] against adversarial random input drawn
//! from three stratified generators:
//!
//! * [`aozora_fragment`] — balanced and unbalanced mixes of Aozora
//!   triggers, plus long `-`/`=`/`_` decorative rule rows for Tier H.
//! * [`pathological_aozora`] — deliberately malformed shapes that
//!   stress-test the lexer's error path.
//! * A combined strategy — `prop_oneof![aozora_fragment, commonmark_adversarial]`
//!   for "Aozora × CommonMark interaction" coverage.
//!
//! Tier F (XSS prevention) lives in its own file because its payload
//! strategy is disjoint from shape-level generators. Tier C (heading
//! integrity) likewise has a dedicated file with heading-biased input.
//!
//! # What this property does *not* promise
//!
//! * Tier B (PUA sentinel leak) fires meaningfully only on sources
//!   that produced no lexer diagnostics — the lexer allows a user to
//!   type a literal U+E001, but emits a diagnostic and does not strip
//!   it, so the sentinel may survive to the output legitimately. The
//!   property gates on `afm_lexer::lex(src).diagnostics.is_empty()`
//!   before calling `check_no_sentinel_leak`.
//!
//! * Tier A likewise only applies when the bracket pairing is
//!   well-formed (per [`afm_parser::test_support::check_no_bare_bracket`]'s
//!   own documented contract). Malformed inputs may legitimately leave
//!   bare `［＃` because the lexer's fallback classifier does not wrap
//!   them. For those inputs we only assert the predicate does not
//!   panic.

use afm_parser::html::render_to_string;
use afm_parser::test_support::{
    check_annotation_wrapper_shape, check_content_model, check_css_class_contract,
    check_escape_invariants, check_heading_integrity, check_html_tag_balance,
    check_markup_completeness, check_no_bare_bracket, check_no_sentinel_leak,
};
use afm_test_utils::config::default_config;
use afm_test_utils::generators::{aozora_fragment, commonmark_adversarial, pathological_aozora};
use proptest::prelude::*;

/// Whether the lexer raised any diagnostic for `src`. Gates Tier A /
/// Tier B assertions so malformed-input boundary behaviour does not
/// sabotage otherwise-valid properties.
fn lexer_is_well_formed(src: &str) -> bool {
    afm_lexer::lex(src).diagnostics.is_empty()
}

/// Assert every always-on shape predicate. Tier A / B are conditionally
/// asserted by the caller because they have input preconditions
/// documented above.
fn assert_always_on(html: &str, src: &str) {
    check_html_tag_balance(html)
        .unwrap_or_else(|e| panic!("Tier D (tag balance) violated for src={src:?}: {e}"));
    check_annotation_wrapper_shape(html)
        .unwrap_or_else(|e| panic!("Tier E (annotation wrapper) violated for src={src:?}: {e}"));
    check_css_class_contract(html)
        .unwrap_or_else(|e| panic!("Tier G (CSS class contract) violated for src={src:?}: {e}"));
    check_escape_invariants(html)
        .unwrap_or_else(|e| panic!("Tier I (escape invariants) violated for src={src:?}: {e}"));
    check_content_model(html)
        .unwrap_or_else(|e| panic!("Tier J (content model) violated for src={src:?}: {e}"));
    check_markup_completeness(html)
        .unwrap_or_else(|e| panic!("Tier K (markup completeness) violated for src={src:?}: {e}"));
    check_heading_integrity(html)
        .unwrap_or_else(|e| panic!("Tier C (heading integrity) violated for src={src:?}: {e}"));
}

/// Assert input-gated predicates (Tier A, Tier B) when the lexer
/// reports a clean parse.
fn assert_gated(html: &str, src: &str) {
    if !lexer_is_well_formed(src) {
        return;
    }
    check_no_bare_bracket(html)
        .unwrap_or_else(|e| panic!("Tier A (bare ［＃ leak) violated for src={src:?}: {e}"));
    check_no_sentinel_leak(html)
        .unwrap_or_else(|e| panic!("Tier B (PUA sentinel leak) violated for src={src:?}: {e}"));
}

proptest! {
    #![proptest_config(default_config())]

    /// Mixed Aozora fragments: the workhorse shape. Covers long
    /// decorative rules (Tier H bait), unbalanced brackets, and a
    /// broad mix of trigger glyphs and plain text.
    #[test]
    fn html_shape_invariants_hold_for_aozora_fragments(src in aozora_fragment(16)) {
        let html = render_to_string(&src);
        assert_always_on(&html, &src);
        assert_gated(&html, &src);
    }

    /// Pathological shapes: deep bracket stacking, paired-container
    /// opens without closes, ruby permutations the classifier must
    /// reject gracefully. These routinely emit lexer diagnostics, so
    /// Tier A / B are skipped here — the interesting property is that
    /// the always-on shape invariants hold regardless of how malformed
    /// the input is.
    #[test]
    fn html_shape_invariants_hold_for_pathological_aozora(src in pathological_aozora(6)) {
        let html = render_to_string(&src);
        assert_always_on(&html, &src);
    }

    /// Aozora × CommonMark interaction: the two grammars collide (a
    /// heading's body carrying annotation markers, a blockquote
    /// containing ruby, a list containing page breaks). Shape
    /// invariants hold across all such mixes.
    #[test]
    fn html_shape_invariants_hold_for_mixed_cm_aozora(
        src in prop_oneof![aozora_fragment(12), commonmark_adversarial()]
    ) {
        let html = render_to_string(&src);
        assert_always_on(&html, &src);
        assert_gated(&html, &src);
    }
}
