//! Verify Pandoc projection robustness and deterministic JSON output.
#![cfg(feature = "pandoc")]

use aozora::pandoc::to_pandoc;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use proptest::prelude::*;

#[expect(
    clippy::expect_used,
    reason = "projection properties require parsing and JSON serialization to succeed"
)]
fn project_to_json(source: &str) -> serde_json::Value {
    let doc = aozora::parse(source.to_owned()).expect("source fits parser span limit");
    let snapshot = doc.snapshot();
    let pandoc = to_pandoc(&snapshot);
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

    #[test]
    fn aozora_fragment_projection_is_deterministic(s in aozora_fragment(120)) {
        assert_json_round_trip_is_deterministic(&s);
    }

    #[test]
    fn pathological_input_projection_is_deterministic(s in pathological_aozora(120)) {
        assert_json_round_trip_is_deterministic(&s);
    }

    #[test]
    fn unicode_adversarial_projection_is_deterministic(s in unicode_adversarial()) {
        assert_json_round_trip_is_deterministic(&s);
    }
}
