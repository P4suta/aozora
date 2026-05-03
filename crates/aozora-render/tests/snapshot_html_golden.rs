//! Snapshot HTML output for a curated sample of inputs.
//!
//! Complementary to `aozora-conformance::render_gate.rs` — that test
//! does byte-identical golden comparison against `expected.html`
//! files committed alongside fixtures. This one snapshots the
//! rendered HTML for a smaller hand-curated set of inputs that are
//! easier to read in `cargo insta review`, with `insta` filters that
//! mask incidental whitespace runs so reviewers can focus on the
//! structural part of the diff.
//!
//! Coverage rationale: each test pins one *kind* of construct in
//! isolation. A renderer regression that subtly alters the wrapper
//! tag for one variant surfaces as a one-test diff instead of a
//! mass conformance failure.

use aozora_pipeline::lex_into_arena;
use aozora_render::html::render_to_string;
use aozora_syntax::borrowed::Arena;

fn render(source: &str) -> String {
    let arena = Arena::new();
    let out = lex_into_arena(source, &arena);
    render_to_string(&out)
}

#[test]
fn snapshot_plain_text() {
    insta::assert_snapshot!(render("Hello, world."));
}

#[test]
fn snapshot_explicit_ruby() {
    insta::assert_snapshot!(render("｜青梅《おうめ》"));
}

#[test]
fn snapshot_implicit_ruby() {
    insta::assert_snapshot!(render("青梅《おうめ》"));
}

#[test]
fn snapshot_double_ruby() {
    insta::assert_snapshot!(render("《《重要》》"));
}

#[test]
fn snapshot_bracket_annotation() {
    insta::assert_snapshot!(render("text［＃改ページ］more text"));
}

#[test]
fn snapshot_gaiji_marker() {
    insta::assert_snapshot!(render("※［＃「木＋吶のつくり」、第3水準1-85-54］"));
}

#[test]
fn snapshot_paired_indent_container() {
    insta::assert_snapshot!(render(
        "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］"
    ));
}

#[test]
fn snapshot_xss_payload_is_escaped() {
    // Any rendered `<script>` substring is a security regression.
    // Snapshot pins the escaped form so reviewers see exactly what
    // the renderer emits for hostile input.
    insta::assert_snapshot!(render("<script>alert(1)</script>"));
}
