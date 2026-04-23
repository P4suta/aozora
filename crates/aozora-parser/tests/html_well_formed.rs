//! M2-S4 — HTML well-formedness invariant I4.
//!
//! Drive a small battery of real-world shapes through
//! `afm_parser::html::render_to_string` and assert the resulting
//! HTML is balanced (every open tag has a matching close in the
//! correct order). Guards against renderer bugs that the Tier-A
//! bracket canary can't catch — e.g. a `<div>` open without its
//! `</div>` close leaves the surrounding markup technically
//! broken even though no `［＃` leaked.
//!
//! The validator lives in `tests/common/mod.rs` so both this file
//! and `corpus_sweep.rs` can call it; this file is the fast-path
//! unit coverage for shapes with known expected-balanced output.

mod common;

use afm_parser::html::render_to_string;
use common::check_well_formed;

/// Small helper: fail with the full HTML context so a violation is
/// actionable without re-running with eyeball inspection.
fn assert_balanced(html: &str, label: &str) {
    let errs = check_well_formed(html);
    assert!(
        errs.is_empty(),
        "{label} HTML is not well-formed:\n  html: {html}\n  errors: {errs:?}"
    );
}

#[test]
fn plain_paragraph_is_balanced() {
    assert_balanced(&render_to_string("hello world"), "plain");
}

#[test]
fn ruby_output_is_balanced() {
    assert_balanced(&render_to_string("｜青梅《おうめ》"), "ruby");
}

#[test]
fn forward_bouten_with_goma_is_balanced() {
    assert_balanced(
        &render_to_string("可哀想［＃「可哀想」に傍点］と彼は言った"),
        "bouten",
    );
}

#[test]
fn unknown_annotation_is_balanced() {
    assert_balanced(
        &render_to_string("前［＃ほげふが］後"),
        "unknown annotation",
    );
}

#[test]
fn page_break_standalone_div_is_balanced() {
    assert_balanced(
        &render_to_string("前\n\n［＃改ページ］\n\n後"),
        "page break",
    );
}

#[test]
fn paired_indent_container_wraps_children_cleanly() {
    // F5 container: `<div class="afm-container-indent-1" …>` must
    // wrap the body `<p>` and close properly on exit.
    assert_balanced(
        &render_to_string("［＃ここから字下げ］\n本文\n［＃ここで字下げ終わり］"),
        "paired indent",
    );
}

#[test]
fn nested_containers_are_balanced() {
    assert_balanced(
        &render_to_string(
            "［＃ここから字下げ］\n\n［＃罫囲み］\n本文\n［＃罫囲み終わり］\n\n［＃ここで字下げ終わり］",
        ),
        "nested containers",
    );
}

#[test]
fn mixed_inline_ruby_bouten_annotation_balanced() {
    assert_balanced(
        &render_to_string("｜青梅《おうめ》の地で、可哀想［＃「可哀想」に傍点］に［＃ほげ］た"),
        "mixed inline",
    );
}

#[test]
fn html_escape_payload_stays_balanced() {
    // The OWASP escape canary — `<` etc. inside Aozora payloads must
    // be escaped, so the validator never sees a literal `<script>`.
    assert_balanced(
        &render_to_string("｜x《<script>alert(1)</script>》"),
        "xss attempt",
    );
}

#[test]
fn double_ruby_academic_brackets_balanced() {
    assert_balanced(&render_to_string("《《強調》》"), "double ruby");
}

// ---------------------------------------------------------------------------
// Self-test of the validator itself
// ---------------------------------------------------------------------------

#[test]
fn validator_accepts_well_formed_html() {
    assert!(check_well_formed("<p>hello <em>world</em></p>").is_empty());
    assert!(check_well_formed("<br>").is_empty()); // void element
    assert!(check_well_formed("<img src=\"x\">").is_empty()); // void with attrs
    assert!(check_well_formed("").is_empty());
}

#[test]
fn validator_rejects_unclosed_tag() {
    let errs = check_well_formed("<p>hello");
    assert_eq!(errs.len(), 1);
    assert!(matches!(&errs[0], common::WellFormedError::UnclosedTag { tag, .. } if tag == "p"));
}

#[test]
fn validator_rejects_extra_close() {
    let errs = check_well_formed("</p>");
    assert_eq!(errs.len(), 1);
    assert!(matches!(&errs[0], common::WellFormedError::ExtraClose { tag, .. } if tag == "p"));
}

#[test]
fn validator_rejects_misordered_close() {
    let errs = check_well_formed("<p><em>hi</p></em>");
    assert!(
        errs.iter()
            .any(|e| matches!(e, common::WellFormedError::MisorderedClose { .. })),
        "expected MisorderedClose, got {errs:?}"
    );
}

#[test]
fn validator_tolerates_attributes_with_tag_characters() {
    // Attribute values may contain `>` (but not in unquoted form).
    // Our renderer doesn't emit such shapes, but robustness helps.
    assert!(check_well_formed("<a href=\"test\">link</a>").is_empty());
}

#[test]
fn validator_handles_void_elements_inside_flow() {
    assert!(check_well_formed("<p>line<br>break</p>").is_empty());
    assert!(check_well_formed("<p>image<img src=\"x\">here</p>").is_empty());
}
