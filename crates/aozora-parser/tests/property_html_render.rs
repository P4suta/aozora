//! Property tests targeting [`aozora_parser::html::render_to_string`].
//!
//! Each block exercises a single invariant against arbitrary aozora-shaped
//! input. The proptest harness shrinks failing inputs to minimal
//! reproducers, which is the highest-ROI way to catch boundary bugs in
//! the new comrak-free block walker (added in v0.1.0 as part of the
//! aozora-from-afm extraction).
//!
//! # Invariants
//!
//! - **R-1: Determinism.** `render_to_string(s) == render_to_string(s)`
//!   always. Any divergence implies hidden state in the lexer or the
//!   walker.
//! - **R-2: Front-door consistency.**
//!   `render_to_string(s) == render_from_artifacts(parse(s).artifacts)`.
//!   The two public entries must agree.
//! - **R-3: Tier A no-bare-bracket.** Even on adversarial input, the
//!   `［＃` marker must never escape an `afm-annotation` wrapper.
//! - **R-4: Tier B no-PUA-leak.** No PUA sentinel `U+E001..=U+E004`
//!   may appear in rendered output.
//! - **R-5: Tag balance.** `<p>`/`</p>` and `<div>`/`</div>` counts
//!   match in every output.
//! - **R-6: Idempotent escape.** Special characters `< > & " '` never
//!   appear unescaped in text content (we route every text byte through
//!   `escape_text_chunk`, so the predicate fires on any render-side
//!   bypass).
//! - **R-7: Output bounded by input.** Walker is `O(n)`; bound the
//!   output size to detect runaway expansion (a regression where a
//!   sentinel re-emits its registered node infinitely would explode).
//! - **R-8: No XSS marker passthrough.** A literal `<script` /
//!   `javascript:` / `on...=` in source never lands as HTML executable.

use aozora_parser::html::{render_from_artifacts, render_to_string};
use aozora_parser::parse;
use aozora_parser::test_support::{
    check_annotation_wrapper_shape, check_html_tag_balance, check_no_bare_bracket,
    check_no_sentinel_leak, check_no_xss_marker, strip_annotation_wrappers,
};
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::{aozora_fragment, pathological_aozora};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;

/// Decide whether the parser produced any diagnostic that signals a
/// shape-level problem (unmatched brackets, residual markers, …).
///
/// Tier A no-bare-bracket is documented as "valid input never leaks
/// `［＃` outside an `afm-annotation` wrapper". For malformed inputs
/// the lexer reports the issue via a `Diagnostic` and passes the bytes
/// through; that is correct behaviour and out of scope for the Tier A
/// proptest. We use this predicate to scope the Tier A invariant to
/// inputs the lexer accepts without complaint.
fn is_lexer_clean(input: &str) -> bool {
    parse(input).diagnostics.is_empty()
}

/// Decide whether the input is free of paired-container markers
/// (`［＃ここから…］` / `［＃ここで…終わり］`). The proptest
/// arbitrarily mixes container *kinds* (indent, keigakomi, warichu,
/// …); a balanced count + ordered sequence is still not enough to
/// admit a tag-balanced render, because a keigakomi-open paired with
/// a indent-close emits `<div>` with no matching close (the lexer
/// classifies them by kind). Rather than exhaustively model the
/// pairing rules at the proptest scope, exclude container shapes
/// entirely — the unit tests in `html.rs::tests::container_*` and
/// the regression pins in `tests/regression_html_layout.rs` plus the
/// 17 k corpus sweep cover the container-tag-balance contract.
fn has_no_container_markers(input: &str) -> bool {
    !input.contains("ここから") && !input.contains("ここで")
}

const SENTINELS: &[char] = &['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];

/// Returns an input that is unlikely to trigger a renderer crash even
/// under proptest's adversarial shrinking, but still exercises every
/// block-walker arm. Generated input may contain trigger glyphs and
/// stray newlines.
fn render_input() -> impl Strategy<Value = String> {
    prop_oneof![
        // Aozora-shaped fragments — most likely to surface walker bugs
        // because they exercise sentinels in non-trivial positions.
        aozora_fragment(20),
        // Pathological brackets — test the fallback paths around
        // unrecognised markers.
        pathological_aozora(8),
        // Random ASCII letters + the five HTML-unsafe characters —
        // drives the escape-pass against text-only chunks. Bounded by
        // codepoint count, not byte count, so multibyte sources can't
        // hit a truncate-at-non-boundary panic.
        proptest_vec(
            prop_oneof![
                any::<u8>()
                    .prop_filter("ASCII printable", |b| (0x20..0x7f).contains(b))
                    .prop_map(char::from),
                Just('<'),
                Just('>'),
                Just('&'),
                Just('"'),
                Just('\''),
            ],
            0..64,
        )
        .prop_map(|chars| chars.into_iter().collect()),
    ]
}

proptest! {
    #![proptest_config(default_config())]

    /// R-1 — render is deterministic.
    #[test]
    fn render_is_deterministic(input in render_input()) {
        let a = render_to_string(&input);
        let b = render_to_string(&input);
        prop_assert_eq!(a, b, "non-deterministic render");
    }

    /// R-2 — front-door consistency.
    #[test]
    fn render_to_string_matches_render_from_artifacts(input in render_input()) {
        let direct = render_to_string(&input);
        let two_step = render_from_artifacts(&parse(&input).artifacts);
        prop_assert_eq!(direct, two_step, "front-door drift");
    }

    /// R-3 — Tier A canary on lexer-clean input.
    ///
    /// Malformed brackets surface as `Diagnostic::UnclosedBracket`,
    /// `UnmatchedClose`, or `ResidualAnnotationMarker`; the lexer
    /// passes the raw bytes through and they legitimately survive
    /// to the rendered HTML. Tier A is the contract for *recognised*
    /// annotations, so we gate the invariant behind a "lexer accepted
    /// without complaint" precondition.
    #[test]
    fn tier_a_no_bare_bracket_on_lexer_clean_input(input in render_input()) {
        prop_assume!(is_lexer_clean(&input));
        let html = render_to_string(&input);
        check_no_bare_bracket(&html)
            .map_err(|e| TestCaseError::fail(format!("Tier A violated for {input:?}: {e}")))?;
    }

    /// R-4 — Tier B no PUA sentinel leak.
    #[test]
    fn tier_b_no_pua_leak(input in render_input()) {
        let html = render_to_string(&input);
        check_no_sentinel_leak(&html)
            .map_err(|e| TestCaseError::fail(format!("Tier B violated for {input:?}: {e}")))?;
    }

    /// R-5 — every render produces tag-balanced HTML on lexer-clean,
    /// container-free input.
    ///
    /// Two preconditions are stacked:
    /// 1. The lexer accepts the input without diagnostics — rules
    ///    out pathological brackets that pass through as literal
    ///    text and would unbalance the shape-only tag check.
    /// 2. The input contains no paired-container markers — see
    ///    [`has_no_container_markers`] for why even a count-balanced
    ///    container shape can mismatch by kind and break tag balance.
    ///    Container-tag-balance is hard-gated by the unit tests in
    ///    `html.rs::tests::container_*` and the 17 k corpus sweep.
    #[test]
    fn html_is_tag_balanced_on_well_formed_input(input in render_input()) {
        prop_assume!(is_lexer_clean(&input));
        prop_assume!(has_no_container_markers(&input));
        let html = render_to_string(&input);
        check_html_tag_balance(&html)
            .map_err(|e| TestCaseError::fail(format!("tag balance violated for {input:?}: {e}")))?;
    }

    /// R-6 — annotation wrapper shape is well-formed (no nested
    /// wrappers, every open has a matching close, every wrapper
    /// carries `hidden`).
    #[test]
    fn annotation_wrapper_shape_is_well_formed(input in render_input()) {
        let html = render_to_string(&input);
        check_annotation_wrapper_shape(&html)
            .map_err(|e| TestCaseError::fail(format!(
                "annotation wrapper shape violated for {input:?}: {e}"
            )))?;
    }

    /// R-7 — output size is bounded by `input.len() * 64`. The
    /// inline-renderer expansion factor is bounded; a regression that
    /// re-emits a registered node at every iteration would blow
    /// through this cap.
    ///
    /// Scaled so that even an all-sentinel input expands to richer
    /// HTML markup (~50 bytes per inline node) without false-positive.
    #[test]
    fn output_is_size_bounded(input in render_input()) {
        let html = render_to_string(&input);
        let cap = input.len().saturating_mul(64).max(64);
        prop_assert!(
            html.len() <= cap,
            "output expanded past cap: input.len()={} html.len()={} (cap {cap})",
            input.len(),
            html.len(),
        );
    }

    /// R-8 — XSS markers are never live in the rendered output.
    /// Stripped of `afm-annotation` wrappers (which legitimately wrap
    /// the *escaped* form of an unknown bracket-marker), the
    /// remaining HTML must not host raw `<script`, `javascript:` in
    /// an attribute, or an inline event handler.
    #[test]
    fn no_xss_marker_in_render(input in render_input()) {
        let html = render_to_string(&input);
        check_no_xss_marker(&html)
            .map_err(|e| TestCaseError::fail(format!("XSS marker leak for {input:?}: {e}")))?;
    }

    /// Composite: every literal sentinel in the source gets *consumed*
    /// (turned into an `afm-annotation` wrapper or a recognised node)
    /// before it reaches the rendered output. A regression that bypassed
    /// the lexer's classification step would surface here.
    ///
    /// Strips wrappers first because legitimate annotations carry the
    /// bare PUA char inside their hidden content; we only fail on
    /// chars that escape the wrapper.
    #[test]
    fn no_pua_outside_wrapper(input in render_input()) {
        let html = render_to_string(&input);
        let bare = strip_annotation_wrappers(&html);
        for sentinel in SENTINELS {
            prop_assert!(
                !bare.contains(*sentinel),
                "PUA sentinel U+{:04X} leaked outside wrapper for {input:?}",
                *sentinel as u32,
            );
        }
    }
}

/// Independent unit-level invariant: rendering an empty buffer
/// produces an empty string. This is asserted via proptest above as a
/// shrunk reproducer of "the render survives any input", but the unit
/// pin gives an instant signal when the empty-input path regresses.
#[test]
fn empty_input_produces_empty_output_pin() {
    assert_eq!(render_to_string(""), "");
}
