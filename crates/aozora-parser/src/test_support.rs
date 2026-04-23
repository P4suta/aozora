//! Test utilities for afm-parser.
//!
//! Deliberately tiny: everything richer (pretty diffs, snapshots, property tests,
//! error reporting) is outsourced to industry-standard testing crates —
//! `pretty_assertions`, `insta`, `proptest`, `miette` — rather than
//! re-implemented here. This module exists for traversal and output-shape glue
//! that isn't worth a dependency.
//!
//! # Visibility
//!
//! `#[doc(hidden)] pub` rather than `#[cfg(test)] mod` so integration tests in
//! `tests/` (which are separate crate roots) can reach these helpers without
//! duplicating them. Marked `doc(hidden)` because the module is not part of the
//! public afm-parser API.

#![doc(hidden)]

use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};

use crate::{Options, parse};

// ---------------------------------------------------------------------------
// AST traversal
// ---------------------------------------------------------------------------

/// Parse `input` with afm defaults and return every Aozora node in order.
///
/// Drives behavioural tests that care about "which recognisers fired" rather
/// than the shape of the arena tree. See also [`collect_aozora_recursive`] for
/// tests that already hold an [`AstNode`] and only need the traversal glue.
#[must_use]
pub fn collect_aozora(input: &str) -> Vec<afm_syntax::AozoraNode> {
    let arena = Arena::new();
    let opts = Options::afm_default();
    let result = parse(&arena, input, &opts);
    let mut out = Vec::new();
    collect_aozora_recursive(result.root, &mut out);
    out
}

/// Recursive traversal helper usable by tests that already hold an [`AstNode`]
/// (e.g. when testing parse modes that bypass the default arena).
pub fn collect_aozora_recursive<'a>(node: &'a AstNode<'a>, out: &mut Vec<afm_syntax::AozoraNode>) {
    if let NodeValue::Aozora(ref boxed) = node.data.borrow().value {
        out.push((**boxed).clone());
    }
    for child in node.children() {
        collect_aozora_recursive(child, out);
    }
}

// ---------------------------------------------------------------------------
// Rendered-HTML post-processing
// ---------------------------------------------------------------------------

const AFM_ANNOTATION_OPEN: &str = r#"<span class="afm-annotation" hidden>"#;
const AFM_ANNOTATION_CLOSE: &str = "</span>";

/// Remove `<span class="afm-annotation" hidden>…</span>` wrappers from `html`.
///
/// Leaves the caller with "bare" output — useful for asserting that no `［＃`
/// leaked outside an annotation wrapper (Tier A invariant). Idempotent: a
/// second pass on already-stripped output returns the same string.
#[must_use]
pub fn strip_annotation_wrappers(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(AFM_ANNOTATION_OPEN) {
        out.push_str(&rest[..at]);
        let after_open = &rest[at + AFM_ANNOTATION_OPEN.len()..];
        let Some(close_at) = after_open.find(AFM_ANNOTATION_CLOSE) else {
            // Malformed — preserve remainder so a Tier-A assertion can fire on
            // the leaked bracket.
            out.push_str(rest);
            return out;
        };
        rest = &after_open[close_at + AFM_ANNOTATION_CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

/// Assert `needle` is absent from `html` once afm-annotation wrappers are stripped.
///
/// The Tier A canary used by every integration test that watches for bracket
/// leaks: `assert_no_bare(&html, "［＃")`.
///
/// # Panics
///
/// Panics with a diagnostic snippet (first occurrence + total count) when
/// `needle` is found in the stripped output.
pub fn assert_no_bare(html: &str, needle: &str) {
    let stripped = strip_annotation_wrappers(html);
    assert!(
        !stripped.contains(needle),
        "bare {needle:?} leaked outside the afm-annotation wrapper.\n\
         first occurrence near:\n{}\n\
         total occurrences: {}",
        first_occurrence_context(&stripped, needle, 80),
        stripped.matches(needle).count(),
    );
}

/// Format a `±window` context snippet around the first `needle` in `haystack`.
///
/// Snaps to UTF-8 boundaries so the excerpt is always losslessly printable.
/// Returns the string `<needle missing>` when the substring is absent.
#[must_use]
pub fn first_occurrence_context(haystack: &str, needle: &str, window: usize) -> String {
    let Some(at) = haystack.find(needle) else {
        return "<needle missing>".to_owned();
    };
    let lo = snap_left(haystack, at.saturating_sub(window));
    let hi = snap_right(haystack, (at + needle.len() + window).min(haystack.len()));
    format!("...{}...", &haystack[lo..hi])
}

/// Round `i` down to the nearest UTF-8 character boundary in `s`.
#[must_use]
pub const fn snap_left(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `i` up to the nearest UTF-8 character boundary in `s`.
#[must_use]
pub const fn snap_right(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_returns_text_outside_wrappers() {
        let html =
            r#"<p>hello <span class="afm-annotation" hidden>［＃改ページ］</span> world</p>"#;
        assert_eq!(strip_annotation_wrappers(html), "<p>hello  world</p>");
    }

    #[test]
    fn strip_is_idempotent() {
        let html = r#"a <span class="afm-annotation" hidden>X</span> b"#;
        let once = strip_annotation_wrappers(html);
        let twice = strip_annotation_wrappers(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn strip_handles_malformed_open_without_close() {
        let html = r#"a <span class="afm-annotation" hidden>X b"#;
        // Should not panic; remainder preserved so a Tier-A assertion can fire.
        let out = strip_annotation_wrappers(html);
        assert!(out.contains("X b"));
    }

    #[test]
    fn first_occurrence_context_snaps_to_char_boundaries() {
        let text = "ああああ［＃改ページ］ええええ";
        let ctx = first_occurrence_context(text, "［＃", 4);
        // Excerpt must be valid UTF-8 and contain the needle.
        assert!(ctx.contains("［＃"));
    }

    #[test]
    fn first_occurrence_context_reports_missing() {
        assert_eq!(
            first_occurrence_context("plain text", "［＃", 10),
            "<needle missing>"
        );
    }

    #[test]
    fn snap_helpers_are_monotonic() {
        let s = "abcあいう";
        assert_eq!(snap_left(s, 0), 0);
        assert_eq!(snap_right(s, s.len()), s.len());
        assert!(snap_left(s, s.len()) <= s.len());
    }

    #[test]
    fn assert_no_bare_passes_for_clean_input() {
        assert_no_bare("<p>plain paragraph</p>", "［＃");
    }

    #[test]
    #[should_panic(expected = "bare")]
    fn assert_no_bare_panics_on_leak() {
        assert_no_bare("<p>prefix ［＃改ページ］ suffix</p>", "［＃");
    }

    #[test]
    fn assert_no_bare_tolerates_wrapped_occurrences() {
        let html =
            r#"<p>prefix <span class="afm-annotation" hidden>［＃改ページ］</span> suffix</p>"#;
        assert_no_bare(html, "［＃");
    }
}
