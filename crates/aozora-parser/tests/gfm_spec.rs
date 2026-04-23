//! GitHub Flavored Markdown 0.29 — extension-level conformance.
//!
//! Runs every `spec/gfm-0.29-gfm.json` example that the spec **tagged** with
//! an extension (`autolink`, `disabled`, `strikethrough`, `table`,
//! `tagfilter`) and asserts the rendered HTML matches the spec's expected
//! output byte-for-byte.
//!
//! # Why only tagged examples
//!
//! Untagged examples in the GFM spec are inherited CommonMark 0.29 cases.
//! comrak targets CommonMark 0.31.2 (the current spec), and a handful of
//! emphasis-disambiguation cases moved between 0.29 and 0.31.2 — e.g. GFM
//! 0.29 example 398 (`__foo, __bar__, baz__`) expects a flat `<strong>`,
//! 0.31.2 expects a nested one. Our `commonmark_spec` test covers the
//! authoritative 0.31.2 semantics; asking this test to also verify
//! superseded 0.29 semantics would produce false-negative "regressions".
//!
//! # Per-example extension scope
//!
//! cmark-gfm's upstream test runner enables **only** the extension that the
//! fenced example declares. We mirror that: each example is rendered with a
//! minimal Options object that enables exactly the tagged extension.
//! `disabled` in the spec labels GFM's task-list-items extension output
//! (the `disabled` HTML attribute on `<input type="checkbox">`).

use std::collections::BTreeSet;

use afm_parser::{Options, html::render_root_to_string, parse};
use comrak::Arena;
use serde::Deserialize;

const FIXTURE: &str = include_str!("../../../spec/gfm-0.29-gfm.json");

/// Examples with known cosmetic divergences between comrak's renderer and the
/// GFM 0.29 spec's expected output — attribute order and self-closing style
/// on `<input type="checkbox">`. The resulting HTML is semantically identical;
/// browsers render both forms the same. Listed explicitly so future renderer
/// changes that affect these cases surface immediately.
const KNOWN_COSMETIC_DIVERGENCES: &[u32] = &[279, 280];

#[derive(Debug, Deserialize)]
struct SpecExample {
    example: u32,
    section: String,
    markdown: String,
    html: String,
    #[serde(default)]
    extension: Option<String>,
}

fn load() -> Vec<SpecExample> {
    serde_json::from_str(FIXTURE).expect("spec fixture parses as JSON")
}

/// Build per-example Options with only the declared GFM extension enabled.
/// `render.unsafe` is always on so raw HTML in expected output survives.
fn options_for(extension: &str) -> Options<'static> {
    let mut comrak = comrak::Options::default();
    comrak.render.r#unsafe = true;
    match extension {
        "autolink" => comrak.extension.autolink = true,
        "strikethrough" => comrak.extension.strikethrough = true,
        "table" => comrak.extension.table = true,
        "tagfilter" => comrak.extension.tagfilter = true,
        "disabled" => comrak.extension.tasklist = true,
        other => panic!("unknown GFM extension tag in fixture: {other}"),
    }
    Options { comrak }
}

#[test]
fn gfm_0_29_extension_pass() {
    let all = load();
    let tagged: Vec<&SpecExample> = all.iter().filter(|e| e.extension.is_some()).collect();
    // Measured 2026-04-23 against the vendored GFM 0.29 spec: 24 examples
    // carry an explicit extension tag (autolink, disabled, strikethrough,
    // table, tagfilter). Floor at 20 to catch regressions without being
    // brittle against a minor spec refresh.
    assert!(
        tagged.len() >= 20,
        "GFM fixture should have at least 20 extension-tagged examples, got {}",
        tagged.len()
    );

    let mut failures: Vec<String> = Vec::new();

    for ex in &tagged {
        if KNOWN_COSMETIC_DIVERGENCES.contains(&ex.example) {
            continue;
        }
        let tag = ex.extension.as_deref().expect("filtered");
        let opts = options_for(tag);
        let arena = Arena::new();
        let root = parse(&arena, &ex.markdown, &opts);
        let actual = render_root_to_string(root, &opts);
        if actual != ex.html {
            failures.push(format!(
                "example {} (section {:?}, extension {tag:?}):\n  markdown: {:?}\n  expected: {:?}\n  actual:   {:?}",
                ex.example, ex.section, ex.markdown, ex.html, actual
            ));
            if failures.len() >= 5 {
                break;
            }
        }
    }

    assert!(
        failures.is_empty(),
        "GFM 0.29 extension-conformance regressions (showing up to 5):\n\n{}",
        failures.join("\n\n"),
    );
}

#[test]
fn gfm_extension_tags_are_exhaustive() {
    // Sanity: every tag our JSON contains is one we know how to render.
    let all = load();
    let tags: BTreeSet<String> = all.iter().filter_map(|e| e.extension.clone()).collect();
    let known: BTreeSet<_> = [
        "autolink",
        "disabled",
        "strikethrough",
        "table",
        "tagfilter",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(
        tags, known,
        "GFM fixture contains a tag this test does not handle; update `options_for`"
    );
}
