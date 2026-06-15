//! End-to-end "does the diagnostic fire?" tests for the authoring-error
//! diagnostics emitted by the fused lex pipeline.
//!
//! Each test parses a hand-crafted trigger string through
//! [`lex_into_arena`] and asserts that the expected
//! [`aozora_spec::Diagnostic`] code shows up with the right severity. The
//! exact span / wire shape is frozen separately by the conformance render
//! gate (`crates/aozora-conformance/fixtures/render/<case>/`); here we only
//! pin *that* the detection triggers (and, for the negatives, that it does
//! not).

use aozora_pipeline::lex_into_arena;
use aozora_spec::{Severity, codes};
use aozora_syntax::borrowed::Arena;

/// Count diagnostics with the given stable code in a fresh parse of `src`.
fn count_code(src: &str, code: &str) -> usize {
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);
    out.diagnostics.iter().filter(|d| d.code() == code).count()
}

/// Assert exactly one diagnostic of `code` fires, and return its severity.
fn one_diag_severity(src: &str, code: &str) -> Severity {
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);
    let hits: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.code() == code)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one `{code}` in {:?}",
        out.diagnostics
    );
    hits[0].severity()
}

// ---------------------------------------------------------------------------
// #8 accent_decomposition_applied (Note, Phase 0)
// ---------------------------------------------------------------------------

#[test]
fn accent_decomposition_fires_as_note() {
    // `〔cafe'〕` decomposes to `〔café〕`; the span is not at byte 0 so a
    // coordinate slip would surface in the conformance golden.
    let sev = one_diag_severity("前〔cafe'〕後", codes::ACCENT_DECOMPOSITION_APPLIED);
    assert_eq!(sev, Severity::Note);
}

#[test]
fn accent_decomposition_one_note_per_decomposed_span() {
    // Two distinct `〔…〕` digraphs → two notes.
    assert_eq!(
        count_code("前〔a`〕中〔e'〕後", codes::ACCENT_DECOMPOSITION_APPLIED),
        2
    );
}

#[test]
fn accent_span_without_digraph_is_silent() {
    // A `〔…〕` whose body holds no accent digraph must not fire the note.
    assert_eq!(
        count_code("〔plain〕text", codes::ACCENT_DECOMPOSITION_APPLIED),
        0
    );
    // And ordinary prose with no tortoiseshell brackets at all is silent.
    assert_eq!(
        count_code("ふつうの文章です", codes::ACCENT_DECOMPOSITION_APPLIED),
        0
    );
}

// ---------------------------------------------------------------------------
// #6 unresolved_gaiji (Warning, Phase 3)
// ---------------------------------------------------------------------------

#[test]
fn unresolved_gaiji_fires_as_warning() {
    // Same `※［＃「…」、<mencode>］` shape as a real gaiji, but the men-ku-ten
    // is out of range so it resolves to nothing and the multi-char
    // description is not a single fallback character → `ucs == None`.
    let sev = one_diag_severity(
        "未知の字※［＃「架空の外字」、第3水準99-99-99］です",
        codes::UNRESOLVED_GAIJI,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn resolvable_gaiji_is_silent() {
    // A real, resolvable gaiji (JIS X 0213 第3水準1-85-54 → 𠀋) must not fire.
    assert_eq!(
        count_code(
            "珍しき木※［＃「木＋吶のつくり」、第3水準1-85-54］が立つ。",
            codes::UNRESOLVED_GAIJI
        ),
        0
    );
}

// ---------------------------------------------------------------------------
// #3 mismatched_container_close (Error, normalizer fold)
// ---------------------------------------------------------------------------

#[test]
fn mismatched_container_close_fires_as_error() {
    // Indent opener (`ここから2字下げ`) closed by an align-end closer
    // (`ここで地付き終わり`) → families differ → Error.
    let sev = one_diag_severity(
        "［＃ここから2字下げ］本文［＃ここで地付き終わり］",
        codes::MISMATCHED_CONTAINER_CLOSE,
    );
    assert_eq!(sev, Severity::Error);
}

#[test]
fn matched_container_close_is_silent() {
    // Same family open/close — the amount payload differs internally
    // (`Indent{2}` open vs `Indent{0}` close) but the discriminant is
    // equal, so this must NOT fire.
    assert_eq!(
        count_code(
            "［＃ここから2字下げ］本文［＃ここで字下げ終わり］",
            codes::MISMATCHED_CONTAINER_CLOSE
        ),
        0
    );
    // Align-end pair, likewise silent.
    assert_eq!(
        count_code(
            "［＃ここから地から2字上げ］本文［＃ここで地付き終わり］",
            codes::MISMATCHED_CONTAINER_CLOSE
        ),
        0
    );
}

// ---------------------------------------------------------------------------
// #1 empty_ruby_reading (Error, Phase 3)
// ---------------------------------------------------------------------------

#[test]
fn empty_ruby_reading_fires_as_error() {
    // Explicit `｜` base with an empty `《》` reading.
    let sev = one_diag_severity("｜青梅《》", codes::EMPTY_RUBY_READING);
    assert_eq!(sev, Severity::Error);
}

#[test]
fn valid_and_baseless_empty_ruby_are_silent() {
    // A complete explicit ruby must not fire.
    assert_eq!(count_code("｜青梅《おうめ》", codes::EMPTY_RUBY_READING), 0);
    // An empty `《》` with NO explicit base is just literal text (the
    // parser can't be sure a base was intended) → silent by design.
    assert_eq!(count_code("あ《》", codes::EMPTY_RUBY_READING), 0);
}

// ---------------------------------------------------------------------------
// #2 nested_ruby (Error, Phase 3)
// ---------------------------------------------------------------------------

#[test]
fn nested_ruby_fires_as_error() {
    // The reading body of the outer `《…》` itself opens an inner `《ん》`.
    // The two closes are non-adjacent (text `じ` sits between them) so the
    // tokenizer does NOT merge `》》` into a DoubleRuby close — this is the
    // genuine nested-ruby shape the catalogue describes (`｜…《…《…》…》`).
    let sev = one_diag_severity("｜漢《か《ん》じ》", codes::NESTED_RUBY);
    assert_eq!(sev, Severity::Error);
}

#[test]
fn flat_ruby_is_not_nested() {
    // A normal ruby and two adjacent rubies must not trip the nested check.
    assert_eq!(count_code("｜青梅《おうめ》", codes::NESTED_RUBY), 0);
    assert_eq!(
        count_code("｜青《あお》｜梅《うめ》", codes::NESTED_RUBY),
        0
    );
}

// ---------------------------------------------------------------------------
// #7 unrecognised_container_directive (Warning, Phase 3)
// ---------------------------------------------------------------------------

#[test]
fn unrecognised_container_directive_fires_as_warning() {
    // `ここから…` opener whose remainder is not a known container kind.
    let sev = one_diag_severity(
        "［＃ここからナントカ］本文",
        codes::UNRECOGNISED_CONTAINER_DIRECTIVE,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn known_and_non_container_directives_are_silent() {
    // A valid container opener resolves → no warning.
    assert_eq!(
        count_code(
            "［＃ここから2字下げ］本文［＃ここで字下げ終わり］",
            codes::UNRECOGNISED_CONTAINER_DIRECTIVE
        ),
        0
    );
    // An unknown annotation that is NOT a `ここから` directive is just a
    // plain `Annotation{Unknown}` and must not be mislabelled.
    assert_eq!(
        count_code(
            "［＃ふつうの注記］",
            codes::UNRECOGNISED_CONTAINER_DIRECTIVE
        ),
        0
    );
}

// ---------------------------------------------------------------------------
// #4 tcy_target_not_found (Warning, Phase 3)
// ---------------------------------------------------------------------------

#[test]
fn tcy_target_not_found_fires_as_warning() {
    // `は縦中横` shape, but the target `い` does not appear before the bracket.
    let sev = one_diag_severity("あ［＃「い」は縦中横］", codes::TCY_TARGET_NOT_FOUND);
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn tcy_with_present_target_is_silent() {
    // Target `64` precedes the directive → recognised → no warning.
    assert_eq!(
        count_code("昭和64［＃「64」は縦中横］年", codes::TCY_TARGET_NOT_FOUND),
        0
    );
}

// ---------------------------------------------------------------------------
// #5 bouten_target_ambiguous (Warning, Phase 3)
// ---------------------------------------------------------------------------

#[test]
fn bouten_target_ambiguous_fires_as_warning() {
    // `青空` occurs twice before the bracket → which run to emphasise is
    // ambiguous.
    let sev = one_diag_severity(
        "青空青空［＃「青空」に傍点］",
        codes::BOUTEN_TARGET_AMBIGUOUS,
    );
    assert_eq!(sev, Severity::Warning);
}

#[test]
fn bouten_unique_target_is_silent() {
    // `青空` occurs exactly once before the bracket → unambiguous.
    assert_eq!(
        count_code("青空［＃「青空」に傍点］", codes::BOUTEN_TARGET_AMBIGUOUS),
        0
    );
}
