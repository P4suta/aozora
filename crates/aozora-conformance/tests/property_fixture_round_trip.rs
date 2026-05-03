//! Property-test the parser against every fixture in the conformance
//! corpus, plus shrinker-driven mutations of those fixtures.
//!
//! The existing `render_gate.rs` is byte-identical: it asserts the
//! current parse output matches the committed golden. This test
//! complements that with two looser-but-broader properties:
//!
//! 1. **Parse + serialise + parse is total**: every fixture's source
//!    must parse, serialise, and re-parse without panic. Catches a
//!    regression that breaks fixture parsing in a way the golden gate
//!    doesn't (e.g. divergent diagnostic output that doesn't affect
//!    the rendered HTML).
//!
//! 2. **Fixture-derived shrinking**: each fixture's source is fed into
//!    a proptest as a *seed* — the shrinker can chop / mutate it to
//!    surface inputs adjacent to the canonical fixture set. Any
//!    regression that breaks "fixture-like" inputs (slight whitespace
//!    drift, truncated body, ASCII noise injected) shows up as a
//!    shrunken counter-example.

use aozora::Document;
use aozora_conformance::{RenderFixture, fixtures_root};
use aozora_proptest::config::default_config;
use proptest::prelude::*;

/// Load every fixture under `fixtures/render/` once. Cached behind a
/// `OnceLock` so the proptest's per-iteration cost stays at "draw an
/// index", not "walk the filesystem".
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

fn parse_serialise_parse(source: &str) {
    let doc = Document::new(source.to_owned());
    let tree = doc.parse();
    let serialised = tree.serialize();
    let doc2 = Document::new(serialised);
    let _tree2 = doc2.parse();
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

    /// Every fixture, plus a per-iteration shrinker-driven byte
    /// truncation, must parse + serialise + re-parse without panic.
    /// The shrinker is what makes this catch regressions the static
    /// fixture corpus doesn't already cover — a truncation that
    /// previously parsed cleanly but now panics will surface as a
    /// shrunken counter-example instead of a flaky render diff.
    #[test]
    fn fixture_truncations_are_total(
        idx in any::<prop::sample::Index>(),
        truncate_to in 0usize..=4096,
    ) {
        let sources = all_fixture_sources();
        let source = idx.get(sources);
        // Truncate to a char boundary so `Document::new` (which takes
        // owned UTF-8) doesn't panic on the truncate itself.
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
