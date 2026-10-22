//! Golden-HTML checks for vendored Aozora Bunko works.
//!
//! After an intentional renderer change, regenerate and review the diff:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate
//! ```

use std::collections::BTreeSet;
use std::{env, fs};

use aozora_conformance::{RenderFixture, fixtures_root};
use pretty_assertions::assert_eq;

#[test]
fn works_gate_html_matches_golden() {
    let fixtures = load_works_fixtures();
    for fixture in &fixtures {
        let doc = aozora::parse(fixture.source.clone()).expect("source fits parser span limit");
        let actual = doc.snapshot().to_html();
        let expected = fixture.html_golden(&actual);
        assert_eq!(
            actual, expected,
            "html drift for vendored work {}",
            fixture.name,
        );
    }
}

fn load_works_fixtures() -> Vec<RenderFixture> {
    let fixtures = RenderFixture::load_group(&fixtures_root(), "works");
    assert!(!fixtures.is_empty(), "no works fixtures found");
    fixtures
}

#[test]
fn works_family_coverage_holds() {
    let mut classes: BTreeSet<String> = BTreeSet::new();
    for fixture in &load_works_fixtures() {
        let html = aozora::parse(fixture.source.clone())
            .expect("source fits parser span limit")
            .snapshot()
            .to_html();
        collect_aozora_classes(&html, &mut classes);
    }
    let mut rendered = String::new();
    for class in &classes {
        rendered.push_str(class);
        rendered.push('\n');
    }

    let baseline = fixtures_root().join("works").join("COVERAGE.txt");
    if env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&baseline, &rendered).expect("write COVERAGE.txt");
        return;
    }
    let expected = fs::read_to_string(&baseline)
        .expect("works/COVERAGE.txt missing — run with UPDATE_GOLDEN=1 to seed");
    let missing: Vec<&str> = expected
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|c| !classes.contains(*c))
        .collect();
    assert!(
        missing.is_empty(),
        "works golden set no longer covers these render classes (a fixture that \
         exercised them was removed?): {missing:?}. If intended, re-seed with UPDATE_GOLDEN=1."
    );
}

fn collect_aozora_classes(html: &str, out: &mut BTreeSet<String>) {
    let bytes = html.as_bytes();
    let mut i = 0;
    while let Some(pos) = html[i..].find("aozora-") {
        let start = i + pos;
        let mut end = start + "aozora-".len();
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-') {
            end += 1;
        }
        out.insert(html[start..end].to_owned());
        i = end;
    }
}
