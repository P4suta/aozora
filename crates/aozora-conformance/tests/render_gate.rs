//! Byte-identical six-axis render gate.
//!
//! Loads every fixture under `fixtures/render/`, parses the source
//! through `aozora::Document`, and asserts the rendered output
//! matches the golden file byte-for-byte across all six axes:
//!
//! 1. `tree.to_html()` — human-readable HTML5 surface
//! 2. `tree.to_source()` — round-trip 青空文庫 source
//! 3. `aozora::json::diagnostics(...)` — diagnostic envelope
//! 4. `aozora::json::nodes(...)` — source-coordinate node table
//! 5. `aozora::json::pairs(...)` — matched delimiter pairs
//! 6. `aozora::json::container_pairs(...)` — container open/close offsets
//!
//! A single commit can break this gate by intentionally changing
//! any of the six surfaces; in that case run with `UPDATE_GOLDEN=1`
//! to refresh the relevant golden, review the diff, and commit.

use aozora::Document;
use aozora::json::{container_pairs, diagnostics, nodes, pairs};
use aozora_conformance::{RenderFixture, fixtures_root};
use pretty_assertions::assert_eq;

#[test]
fn render_gate_html_matches_golden() {
    for fixture in load_render_fixtures() {
        let doc = Document::new(fixture.source.clone());
        let actual = doc.parse().to_html();
        let expected = fixture.html_golden(&actual);
        assert_eq!(actual, expected, "html drift for fixture {}", fixture.name);
    }
}

#[test]
fn render_gate_serialize_matches_golden() {
    for fixture in load_render_fixtures() {
        let doc = Document::new(fixture.source.clone());
        let actual = doc.parse().to_source();
        let expected = fixture.serialize_golden(&actual);
        assert_eq!(
            actual, expected,
            "serialize drift for fixture {}",
            fixture.name,
        );
    }
}

#[test]
fn render_gate_diagnostics_matches_golden() {
    for fixture in load_render_fixtures() {
        let doc = Document::new(fixture.source.clone());
        let tree = doc.parse();
        let actual = diagnostics(tree.diagnostics());
        let expected = fixture.diagnostics_golden(&actual);
        assert_eq!(
            actual, expected,
            "diagnostics wire drift for fixture {}",
            fixture.name,
        );
    }
}

#[test]
fn render_gate_nodes_matches_golden() {
    for fixture in load_render_fixtures() {
        let doc = Document::new(fixture.source.clone());
        let tree = doc.parse();
        let actual = nodes(&tree);
        let expected = fixture.nodes_golden(&actual);
        assert_eq!(
            actual, expected,
            "nodes wire drift for fixture {}",
            fixture.name,
        );
    }
}

#[test]
fn render_gate_pairs_matches_golden() {
    for fixture in load_render_fixtures() {
        let doc = Document::new(fixture.source.clone());
        let tree = doc.parse();
        let actual = pairs(&tree);
        let expected = fixture.pairs_golden(&actual);
        assert_eq!(
            actual, expected,
            "pairs wire drift for fixture {}",
            fixture.name,
        );
    }
}

#[test]
fn render_gate_container_pairs_matches_golden() {
    for fixture in load_render_fixtures() {
        let doc = Document::new(fixture.source.clone());
        let tree = doc.parse();
        let actual = container_pairs(&tree);
        let expected = fixture.container_pairs_golden(&actual);
        assert_eq!(
            actual, expected,
            "container_pairs wire drift for fixture {}",
            fixture.name,
        );
    }
}

fn load_render_fixtures() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");
    fixtures
}
