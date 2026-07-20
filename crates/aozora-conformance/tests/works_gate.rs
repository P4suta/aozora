//! Byte-identical golden-HTML gate over a small set of REAL 青空文庫
//! works.
//!
//! The crafted per-family fixtures under `fixtures/render/` isolate one
//! notation construct each. This gate is complementary: it renders a
//! lean, hand-picked set of *whole* public-domain works — the CRLF is
//! normalised to LF and the source is vendored verbatim under
//! `fixtures/works/<slug>/source.txt` — and byte-compares
//! `aozora::parse(src).expect("source fits parser span limit").snapshot().to_html()` to the committed
//! `expected.html`. Its job is to catch rendering drift on the
//! notation *combinations* real works exhibit (ruby beside 傍点 beside
//! 縦中横 beside 字下げ …) that the single-construct fixtures cannot see.
//!
//! It is corpus-free: the works are vendored, so the gate reads no
//! `AOZORA_CORPUS_ROOT` and always runs — it lives in the `conformance`
//! CI job next to the spec-vector and reference-grammar checks.
//!
//! The committed golden is the parser's *own* `to_html()` output — a
//! drift-detection baseline, not an independent oracle. After an
//! intentional renderer change, regenerate and review the diff:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate
//! ```
//!
//! The original 14 works were hand-picked; the rest were selected reproducibly
//! by `xtask corpus select-works` (a deterministic greedy family set-cover under
//! a source-byte budget) — see `fixtures/works-selection.toml` for each work's
//! corpus path and family profile. Rather than hand-maintain a "families
//! exercised" table, `works_family_coverage_holds` pins the union of `aozora-*`
//! render classes the whole set covers (`fixtures/works/COVERAGE.txt`).

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

/// The set of `aozora-*` render classes the whole works golden set exercises —
/// an executable replacement for a hand-maintained "families exercised" table.
/// Deleting a fixture that was the sole source of a class fails this test; new
/// classes are fine (the check is coverage ⊇ baseline). Re-seed the committed
/// `works/COVERAGE.txt` with `UPDATE_GOLDEN=1`.
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

/// Collect every `aozora-<token>` run (render class) appearing in `html`.
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
