//! Class-contract test between renderer and themes.
//!
//! The afm-book ships two CSS themes (`afm-horizontal.css` and
//! `afm-vertical.css`) whose class selectors must cover every
//! class token the renderer can emit. Without this contract a
//! renderer change (e.g. a new `afm-bouten-foo` kind) silently
//! ships unstyled markup — invisible in unit tests because the
//! existing HTML assertions don't care about CSS.
//!
//! The test reads both CSS files at runtime, extracts every
//! `.afm-*` selector, and asserts each pinned class token appears
//! in both. If a renderer adds a class, the pinned list in this
//! file flags the missing style at `cargo test` time.
//!
//! The pinned list is intentionally hand-curated (not derived by
//! scraping `html.rs`) so that adding a new class forces an
//! *explicit* update here — drive-by CSS gaps can't slip in.

use core::str;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use afm_parser::aozora::AFM_CLASSES;

/// Absolute path to one of the theme CSS files. Resolving via
/// `CARGO_MANIFEST_DIR` keeps the test stable regardless of the
/// runner's working directory.
fn theme_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/afm-parser → crates/
    p.push("afm-book");
    p.push("theme");
    p.push(name);
    p
}

/// Return every `.afm-…` class selector name appearing in `css`.
///
/// A tokeniser that extracts the identifier immediately after each
/// `.afm-` prefix. Accepts lowercase ASCII letters, digits, and
/// hyphens; stops at any other character. Intentionally trivial —
/// the project's CSS doesn't use namespace prefixes or escaped
/// selectors.
fn collect_afm_selectors(css: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let bytes = css.as_bytes();
    let mut i = 0usize;
    while i + 5 <= bytes.len() {
        if &bytes[i..i + 5] == b".afm-" {
            let start = i + 1; // after the '.'
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-') {
                end += 1;
            }
            // Trim trailing hyphens from a ".afm-" prefix with no
            // body — shouldn't occur but be tolerant.
            let token = str::from_utf8(&bytes[start..end]).expect("ASCII");
            if token.len() > "afm-".len() && !token.ends_with('-') {
                out.insert(token.to_owned());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

#[test]
fn every_emitted_class_has_a_horizontal_theme_rule() {
    let css = fs::read_to_string(theme_path("afm-horizontal.css"))
        .expect("afm-horizontal.css must exist alongside afm-book/theme/");
    let selectors = collect_afm_selectors(&css);
    let missing: Vec<&&str> = AFM_CLASSES
        .iter()
        .filter(|c| !selectors.contains(**c))
        .collect();
    assert!(
        missing.is_empty(),
        "afm-horizontal.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn every_emitted_class_has_a_vertical_theme_rule() {
    let css = fs::read_to_string(theme_path("afm-vertical.css"))
        .expect("afm-vertical.css must exist alongside afm-book/theme/");
    let selectors = collect_afm_selectors(&css);
    let missing: Vec<&&str> = AFM_CLASSES
        .iter()
        .filter(|c| !selectors.contains(**c))
        .collect();
    assert!(
        missing.is_empty(),
        "afm-vertical.css is missing rules for emitted classes: {missing:?}"
    );
}

#[test]
fn pinned_classes_are_sorted_and_unique() {
    // Hygiene: the pinned list is kept in sorted order + no dupes
    // for review friendliness. A sorted list also makes PR diffs
    // trivial to review.
    let mut copy: Vec<&str> = AFM_CLASSES.to_vec();
    copy.sort_unstable();
    assert_eq!(AFM_CLASSES.to_vec(), copy, "AFM_CLASSES must stay sorted");
    let mut seen: HashSet<&str> = HashSet::new();
    for &c in AFM_CLASSES {
        assert!(seen.insert(c), "duplicate entry in AFM_CLASSES: {c}");
    }
}

#[test]
fn collect_afm_selectors_extracts_basic_rules() {
    // Self-test for the tokeniser — a regression here would
    // silently weaken every other test in this file.
    let css = ".afm-foo { color: red; }\n.afm-bar-baz, .afm-qux { }\n.foo { }";
    let selectors = collect_afm_selectors(css);
    assert!(selectors.contains("afm-foo"));
    assert!(selectors.contains("afm-bar-baz"));
    assert!(selectors.contains("afm-qux"));
    assert!(!selectors.contains("foo"));
}

#[test]
fn collect_afm_selectors_tolerates_trailing_hyphen() {
    // `.afm-` alone (no body) must not emit a token.
    let css = ".afm- { }";
    let selectors = collect_afm_selectors(css);
    assert!(selectors.is_empty());
}
