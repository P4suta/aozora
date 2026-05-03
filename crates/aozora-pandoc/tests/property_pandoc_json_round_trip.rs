//! Pandoc projection invariants.
//!
//! Two complementary properties on the `AozoraTree → pandoc_ast::Pandoc`
//! projection:
//!
//! 1. **Totality**: [`to_pandoc`] is total over every input the lex
//!    pipeline accepts. The projection is a pure mapping over the
//!    borrowed AST; a panic here is a denial-of-service surface for
//!    every Pandoc-format consumer (pandoc → HTML / EPUB / DOCX / …).
//!
//! 2. **JSON round-trip is value-preserving**: the projection
//!    serialises through `pandoc_ast::Pandoc`'s `serde::Serialize`
//!
//!    impl into JSON, and `pandoc_ast` consumers (the `pandoc`
//!    binary) parse the JSON back into the same Pandoc value. The
//!    property here is that `serde_json::to_value` (the value-level
//!    serialise) is deterministic — running it twice on the same AST
//!    produces byte-identical JSON. A regression that introduced
//!    nondeterminism (`HashMap` iteration order, etc.) would silently
//!    break every Pandoc downstream that diff-compares output.

use aozora::Document;
use aozora_pandoc::to_pandoc;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use proptest::prelude::*;

fn project_to_json(source: &str) -> serde_json::Value {
    let doc = Document::new(source.to_owned());
    let tree = doc.parse();
    let pandoc = to_pandoc(&tree);
    serde_json::to_value(&pandoc).expect("pandoc_ast::Pandoc serialises into a serde_json::Value")
}

fn assert_json_round_trip_is_deterministic(source: &str) {
    let first = project_to_json(source);
    let second = project_to_json(source);
    assert_eq!(
        first, second,
        "pandoc JSON projection diverged across runs for source {source:?}"
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_projects_deterministically() {
    assert_json_round_trip_is_deterministic("");
}

#[test]
fn ruby_projects_deterministically() {
    assert_json_round_trip_is_deterministic("｜青梅《おうめ》");
    assert_json_round_trip_is_deterministic("青梅《おうめ》");
}

#[test]
fn brackets_project_deterministically() {
    assert_json_round_trip_is_deterministic("text［＃改ページ］more");
}

#[test]
fn paired_container_projects_deterministically() {
    assert_json_round_trip_is_deterministic(
        "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］",
    );
}

#[test]
fn projection_is_total_on_xss_payloads() {
    // Confirms the `to_pandoc` projection doesn't panic on inputs
    // crafted to expose unescaped HTML — the test is not asserting
    // anything about the *content* of the projection (Pandoc handles
    // escaping at write time), only that the projection stays total.
    let payloads = [
        "<script>alert(1)</script>",
        "｜<script>《</script>》",
        "［＃「<script>」は大見出し］",
    ];
    for p in payloads {
        drop(project_to_json(p));
    }
}

proptest! {
    #![proptest_config(default_config())]

    /// Projection is total + deterministic over the workhorse Aozora
    /// fragment generator.
    #[test]
    fn aozora_fragment_projection_is_deterministic(s in aozora_fragment(120)) {
        assert_json_round_trip_is_deterministic(&s);
    }

    /// Pathological / unbalanced inputs — projection stays total even
    /// when the lex pipeline emits diagnostics.
    #[test]
    fn pathological_input_projection_is_deterministic(s in pathological_aozora(120)) {
        assert_json_round_trip_is_deterministic(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// codepoints. The projection must stay total and the JSON must
    /// be deterministic.
    #[test]
    fn unicode_adversarial_projection_is_deterministic(s in unicode_adversarial()) {
        assert_json_round_trip_is_deterministic(&s);
    }
}
