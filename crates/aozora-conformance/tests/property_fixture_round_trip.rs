//! Round-trip properties for conformance fixtures and their truncated variants.

use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_proptest::config::default_config;
use proptest::prelude::*;

fn all_fixture_sources() -> &'static Vec<String> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(|| {
        RenderFixture::load_group(&fixtures_root(), "render")
            .into_iter()
            .map(|fixture| fixture.source)
            .collect()
    })
}

#[expect(
    clippy::expect_used,
    reason = "bounded conformance fixtures must parse on both passes"
)]
fn parse_serialise_parse(source: &str) {
    let doc = aozora::parse(source.to_owned()).expect("source fits parser span limit");
    let tree = doc.snapshot();
    let serialised = tree.to_source();
    let doc2 = aozora::parse(serialised).expect("source fits parser span limit");
    let _tree2 = doc2.snapshot();
}

#[test]
fn fixture_corpus_is_non_empty() {
    let sources = all_fixture_sources();
    assert!(
        !sources.is_empty(),
        "expected at least one fixture under fixtures/render/"
    );
}

#[test]
fn every_fixture_round_trips_without_panic() {
    for source in all_fixture_sources() {
        parse_serialise_parse(source);
    }
}

proptest! {
    #![proptest_config(default_config())]

    #[test]
    fn fixture_truncations_are_total(
        idx in any::<prop::sample::Index>(),
        truncate_to in 0usize..=4096,
    ) {
        let sources = all_fixture_sources();
        let source = idx.get(sources);
        let cap = truncate_to.min(source.len());
        let truncated = take_to_char_boundary(source, cap);
        parse_serialise_parse(truncated);
    }
}

fn take_to_char_boundary(s: &str, mut cap: usize) -> &str {
    while cap > 0 && !s.is_char_boundary(cap) {
        cap -= 1;
    }
    &s[..cap]
}
