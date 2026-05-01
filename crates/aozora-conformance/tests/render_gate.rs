//! Phase K3 — byte-identical render gate.
//!
//! Loads every fixture under `fixtures/render/`, parses the source
//! through `aozora::Document`, and asserts the rendered HTML and
//! serialize output match the golden file byte-for-byte. A single
//! commit can break this gate by intentionally changing renderer
//! output; in that case run with `UPDATE_GOLDEN=1` to refresh the
//! golden, review the diff, and commit.
//!
//! Pre-Phase-K3 the byte-identical contract was enforced only inside
//! `aozora-render` (streaming-vs-node consistency). This gate adds
//! cross-commit regression detection: any unintentional change to
//! HTML / serialize output trips the gate before reaching review.

use aozora::Document;
use aozora_conformance::{RenderFixture, fixtures_root};
use pretty_assertions::assert_eq;

#[test]
fn render_gate_html_matches_golden() {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");

    for fixture in fixtures {
        let doc = Document::new(fixture.source.clone());
        let actual = doc.parse().to_html();
        let expected = fixture.html_golden(&actual);
        assert_eq!(actual, expected, "html drift for fixture {}", fixture.name,);
    }
}

#[test]
fn render_gate_serialize_matches_golden() {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "render");
    assert!(!fixtures.is_empty(), "no render fixtures found");

    for fixture in fixtures {
        let doc = Document::new(fixture.source.clone());
        let actual = doc.parse().serialize();
        let expected = fixture.serialize_golden(&actual);
        assert_eq!(
            actual, expected,
            "serialize drift for fixture {}",
            fixture.name,
        );
    }
}
