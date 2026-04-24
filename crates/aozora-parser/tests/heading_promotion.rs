//! Integration tests for the forward-reference heading-hint path.
//!
//! Covers the end-to-end rendering contract for
//! `［＃「X」は(大|中|小)見出し］`:
//!
//! * Lexer classifies the bracket as an `AozoraNode::HeadingHint`.
//! * `post_process::splice_heading_hint` promotes the host paragraph
//!   to `comrak::NodeValue::Heading { level, setext: false }` and
//!   replaces its content with the extracted target.
//! * Renderer emits `<h1>/<h2>/<h3>` native HTML tags.
//!
//! Also covers the adjacent sanitize rule (phase0):
//!
//! * Lines consisting of ≥ 10 repeats of `-`/`=`/`_` are isolated
//!   from the preceding paragraph so CommonMark does not promote
//!   that paragraph to a setext heading (`<h2>`).
//!
//! These tests sit above the unit tests in
//! `afm-lexer::phase3_classify`, `afm-lexer::phase0_sanitize`, and
//! `afm-parser::post_process` — they verify the three layers
//! compose as documented, not any single layer in isolation.

use afm_parser::html;

#[test]
fn big_heading_is_rendered_as_h1() {
    // 大見出し → Markdown H1. Forward-reference target "第一篇" is
    // preceded by its own plain copy so the lexer's target-exists
    // gate passes.
    let out = html::render_to_string("第一篇［＃「第一篇」は大見出し］");
    assert!(
        out.contains("<h1>第一篇</h1>"),
        "expected <h1>第一篇</h1> in output; got: {out}"
    );
}

#[test]
fn medium_heading_is_rendered_as_h2() {
    let out = html::render_to_string("一［＃「一」は中見出し］");
    assert!(
        out.contains("<h2>一</h2>"),
        "expected <h2>一</h2> in output; got: {out}"
    );
}

#[test]
fn small_heading_is_rendered_as_h3() {
    let out = html::render_to_string("小題［＃「小題」は小見出し］");
    assert!(
        out.contains("<h3>小題</h3>"),
        "expected <h3>小題</h3> in output; got: {out}"
    );
}

#[test]
fn heading_with_preceding_indent_marker_still_becomes_heading() {
    // The 罪と罰 fixture shape: `［＃２字下げ］第一篇［＃「第一篇」は大見出し］`.
    // The post-process must strip the leading indent AozoraNode from
    // the paragraph so it doesn't leak into the promoted heading.
    let out = html::render_to_string("［＃２字下げ］第一篇［＃「第一篇」は大見出し］");
    assert!(
        out.contains("<h1>第一篇</h1>"),
        "expected <h1>第一篇</h1>; got: {out}"
    );
    // The heading body must be the target only — no indent marker
    // class, no annotation wrapper.
    assert!(
        !out.contains("<h1><span class=\"afm-indent"),
        "indent marker must not leak into the heading: {out}"
    );
    assert!(
        !out.contains("<h1><span class=\"afm-annotation"),
        "annotation wrapper must not leak into the heading: {out}"
    );
}

#[test]
fn heading_hint_without_preceding_target_stays_as_annotation() {
    // No preceding "第一篇" run — the classifier rejects and the
    // catch-all emits `Annotation{Unknown}`. Tier-A canary still
    // holds: `［＃` never appears outside the `afm-annotation`
    // hidden wrapper.
    let input = "本文［＃「第一篇」は大見出し］";
    let out = html::render_to_string(input);
    assert!(
        !out.contains("<h1>"),
        "no heading should be promoted without a preceding target; got: {out}"
    );
    // Tier-A: the raw bracket characters must be inside a hidden
    // annotation wrapper, not bare in the output.
    assert!(
        !out.contains("］は大見出し］"),
        "bracket content should not leak bare; got: {out}"
    );
}

#[test]
fn long_hyphen_rule_does_not_turn_paragraph_into_setext_heading() {
    // Direct analogue of the `spec/aozora/fixtures/56656/input.utf8.txt`
    // front-matter shape: a prose line followed by a long `-` run.
    // Without phase0's rule-isolation pass, CommonMark would promote
    // the prose to a setext H2.
    let input = "凡例です。\n-----------------------------------\n本文";
    let out = html::render_to_string(input);
    assert!(
        out.contains("<p>凡例です。</p>"),
        "preceding prose must remain a paragraph; got: {out}"
    );
    assert!(
        !out.contains("<h2>凡例です。</h2>"),
        "preceding prose must not become a setext heading; got: {out}"
    );
    // The rule itself should render as a thematic break.
    assert!(
        out.contains("<hr"),
        "decorative rule should render as <hr>; got: {out}"
    );
}

#[test]
fn long_equals_rule_does_not_turn_paragraph_into_setext_heading() {
    let input = "凡例です。\n=====================================\n本文";
    let out = html::render_to_string(input);
    assert!(
        out.contains("<p>凡例です。</p>"),
        "preceding prose must remain a paragraph; got: {out}"
    );
    assert!(
        !out.contains("<h1>凡例です。</h1>"),
        "preceding prose must not become a setext H1; got: {out}"
    );
}

#[test]
fn short_setext_heading_still_works() {
    // Regression canary for the rule-isolation threshold. A standard
    // 3-character setext underline is shorter than
    // `DECORATIVE_RULE_MIN_LEN` (10) and therefore untouched — the
    // CommonMark idiom of `Heading\n---\n` still promotes to H2.
    let input = "Heading\n---\nbody";
    let out = html::render_to_string(input);
    assert!(
        out.contains("<h2>Heading</h2>"),
        "short `---` must still act as a setext underline; got: {out}"
    );
}

#[test]
fn heading_hint_round_trips_through_serialize() {
    // I3 (serialize ∘ parse fixed point) demands that a heading hint
    // reconstructs its original `［＃「…」は…見出し］` form through
    // the serializer even though post_process promoted the host
    // paragraph. The serializer works from the lexer's placeholder
    // registry, so the heading's AST-side promotion does not lose
    // round-trip information.
    use comrak::Arena;
    let arena = Arena::new();
    let opts = afm_parser::Options::afm_default();

    let input = "第一篇［＃「第一篇」は大見出し］";
    let parsed = afm_parser::parse(&arena, input, &opts);
    let serialised = afm_parser::serialize(&parsed);

    assert!(
        serialised.contains("［＃「第一篇」は大見出し］"),
        "heading-hint markup must survive round-trip; got: {serialised}"
    );

    // Second round-trip to lock fixed-point: parse again and confirm
    // the serializer emits the same bytes.
    let reparsed = afm_parser::parse(&arena, &serialised, &opts);
    let second = afm_parser::serialize(&reparsed);
    assert_eq!(
        serialised, second,
        "serialize ∘ parse must be a fixed point after one iteration"
    );
}
