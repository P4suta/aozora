//! Test utilities for aozora-parser.
//!
//! This module hosts the *predicates* that codify every "must never
//! be" output shape the renderer must avoid — see the [Invariant
//! catalog](#invariant-catalog) below. Each predicate returns
//! `Result<(), Violation>` so it can be composed from unit tests,
//! property tests, the corpus sweep, and fuzz harnesses on equal
//! footing. [`assert_invariants`] runs every predicate and aggregates
//! their diagnostics.
//!
//! The matching *generator* strategies live in the `aozora-test-utils`
//! crate so `proptest` does not become a transitive runtime
//! dependency of `aozora-parser`.
//!
//! # Invariant catalog
//!
//! | Tier | Predicate | Shape forbidden in rendered HTML |
//! |------|-----------|-----------------------------------|
//! | A    | [`check_no_bare_bracket`]           | Bare `［＃` outside `afm-annotation` wrapper (Tier-A canary). |
//! | B    | [`check_no_sentinel_leak`]          | PUA sentinels U+E001–U+E004 (lexer-internal markers). |
//! | C    | [`check_heading_integrity`]         | `<h1>`–`<h6>` bodies must not carry `afm-indent`, `afm-container-indent`, or `afm-annotation` tokens. |
//! | D    | [`check_html_tag_balance`]          | Tags must balance ([`check_well_formed`] returns `Ok`). |
//! | E    | [`check_annotation_wrapper_shape`]  | `afm-annotation` wrappers must close, carry `hidden`, and never nest. |
//! | F    | [`check_no_xss_marker`]             | Raw `<script`, `javascript:`, or `on<event>=` must never appear. |
//! | G    | [`check_css_class_contract`]        | Every `afm-*` class token must be in the pinned [`AFM_CLASSES`] list. |
//! | I    | [`check_escape_invariants`]         | No double-encoded entities (`&amp;lt;`, `&amp;amp;`, …). |
//! | J    | [`check_content_model`]             | `<rt>` / `<rp>` must appear only inside `<ruby>`. |
//! | K    | [`check_markup_completeness`]       | Every `<ruby>` with an `<rp>(</rp>` must also carry its closing `<rp>)</rp>`. |
//!
//! Tier H (no setext `<h2>` from a decorative rule) and Tier L (no
//! empty heading) are unit-test-only for now — they depend on being
//! able to witness the pre-/post-promotion AST, which the shape-only
//! HTML predicate cannot observe.
//!
//! # Visibility
//!
//! `#[doc(hidden)] pub` rather than `#[cfg(test)] mod` so integration
//! tests in `tests/` (which are separate crate roots) can reach these
//! helpers without duplicating them. Marked `doc(hidden)` because the
//! module is not part of the public aozora-parser API.
//!
//! [`AFM_CLASSES`]: crate::aozora::classes::AFM_CLASSES

#![doc(hidden)]

use core::error::Error;
use core::fmt;
use std::collections::HashSet;

use aozora_lex::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
use aozora_syntax::{AozoraNode, Container};

use crate::aozora::AFM_CLASSES;
use crate::parse;

// ---------------------------------------------------------------------------
// Registry traversal
// ---------------------------------------------------------------------------

/// Parse `input` and return every Aozora node in source order.
///
/// Drives behavioural tests that care about "which recognisers fired"
/// rather than the shape of any tree. The traversal walks the lexer's
/// normalized text byte-by-byte and dispatches each PUA sentinel
/// through the registry, so the resulting `Vec` follows source order
/// without ever materialising a tree.
///
/// Block-open sentinels surface as `AozoraNode::Container { kind }`
/// at the open position; block-close sentinels are skipped (the
/// container appears once, on enter, mirroring how AST consumers see
/// it).
#[must_use]
pub fn collect_aozora(input: &str) -> Vec<AozoraNode> {
    let result = parse(input);
    let normalized = &result.artifacts.normalized;
    let registry = &result.artifacts.registry;
    let mut out = Vec::new();
    for (bpos, ch) in normalized.char_indices() {
        let pos = u32::try_from(bpos).expect("normalized fits u32 per Phase 0 cap");
        match ch {
            INLINE_SENTINEL => {
                if let Some(node) = registry.inline_at(pos) {
                    out.push(node.clone());
                }
            }
            BLOCK_LEAF_SENTINEL => {
                if let Some(node) = registry.block_leaf_at(pos) {
                    out.push(node.clone());
                }
            }
            BLOCK_OPEN_SENTINEL => {
                if let Some(kind) = registry.block_open_at(pos) {
                    out.push(AozoraNode::Container(Container { kind }));
                }
            }
            BLOCK_CLOSE_SENTINEL => {
                // Skipped — the open already produced one entry.
                let _ = pos;
            }
            _ => {}
        }
    }
    out
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
    context_window(haystack, at, needle.len(), window)
}

/// Format a `±window` context snippet around a byte offset in `haystack`.
///
/// `len` is the length of the "needle" that starts at `offset`; it scopes
/// the mark so the caller can report a specific tag or class name without
/// having to call the string-based [`first_occurrence_context`]. Used by
/// predicates whose offending location is structural rather than a
/// substring match (tag-balance, heading contamination).
#[must_use]
pub fn first_occurrence_context_bytes(haystack: &str, offset: usize, window: usize) -> String {
    if offset > haystack.len() {
        return "<offset out of range>".to_owned();
    }
    context_window(haystack, offset, 0, window)
}

fn context_window(haystack: &str, at: usize, len: usize, window: usize) -> String {
    let lo = snap_left(haystack, at.saturating_sub(window));
    let hi = snap_right(haystack, (at + len + window).min(haystack.len()));
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

// ---------------------------------------------------------------------------
// Invariant violations
// ---------------------------------------------------------------------------

/// Every way an afm HTML output can violate a codified invariant.
///
/// One variant per predicate so the aggregator [`assert_invariants`] can
/// route its diagnostics without losing structure. Snippets are short
/// (≤ ±80 bytes around the offending locus) so proptest shrinking does
/// not hold large values in flight during long searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// Tier A — a bare `［＃` leaked outside any `afm-annotation` wrapper.
    BareBracket {
        first_offset: usize,
        snippet: String,
        total: usize,
    },
    /// Tier B — a PUA sentinel (U+E001–U+E004) reached the rendered HTML.
    SentinelLeak {
        codepoint: char,
        first_offset: usize,
        snippet: String,
    },
    /// Tier C — a heading (`<h1>`–`<h6>`) body contains a forbidden class.
    HeadingContaminated {
        level: u8,
        forbidden_class: String,
        snippet: String,
    },
    /// Tier D — a tag-balance violation from [`check_well_formed`].
    UnbalancedTag(WellFormedError),
    /// Tier E — the `afm-annotation` wrapper shape is malformed.
    AnnotationWrapper {
        violation: &'static str,
        snippet: String,
    },
    /// Tier F — an XSS marker leaked into the HTML.
    XssLeak {
        marker: &'static str,
        first_offset: usize,
        snippet: String,
    },
    /// Tier G — an `afm-*` class token is not in [`AFM_CLASSES`].
    UnknownCssClass { class: String, snippet: String },
    /// Tier I — a double-encoded HTML entity (e.g. `&amp;lt;`) slipped in.
    DoubleEncodedEntity { snippet: String },
    /// Tier J — HTML content-model violation (orphan `<rt>`, `<rp>`, …).
    ContentModel {
        violation: &'static str,
        snippet: String,
    },
    /// Tier K — `<ruby>` element missing its `<rp>(</rp>` ↔ `<rp>)</rp>` pair.
    MarkupIncomplete {
        violation: &'static str,
        snippet: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BareBracket {
                total,
                snippet,
                first_offset,
            } => write!(
                f,
                "Tier A: bare `［＃` leaked outside afm-annotation wrapper \
                 ({total} occurrence(s); first near offset {first_offset}): {snippet}",
            ),
            Self::SentinelLeak {
                codepoint,
                first_offset,
                snippet,
            } => write!(
                f,
                "Tier B: lexer PUA sentinel U+{codepoint:04X} leaked to rendered HTML \
                 (first near offset {first_offset}): {snippet}",
                codepoint = *codepoint as u32,
            ),
            Self::HeadingContaminated {
                level,
                forbidden_class,
                snippet,
            } => write!(
                f,
                "Tier C: <h{level}> body carries forbidden class `{forbidden_class}`: {snippet}",
            ),
            Self::UnbalancedTag(e) => write!(f, "Tier D: {e}"),
            Self::AnnotationWrapper { violation, snippet } => {
                write!(f, "Tier E: afm-annotation wrapper {violation}: {snippet}")
            }
            Self::XssLeak {
                marker,
                first_offset,
                snippet,
            } => write!(
                f,
                "Tier F: XSS marker `{marker}` leaked (first near offset {first_offset}): {snippet}",
            ),
            Self::UnknownCssClass { class, snippet } => write!(
                f,
                "Tier G: unknown CSS class `{class}` (not in AFM_CLASSES): {snippet}",
            ),
            Self::DoubleEncodedEntity { snippet } => write!(
                f,
                "Tier I: double-encoded entity (e.g. `&amp;lt;`) leaked into output: {snippet}",
            ),
            Self::ContentModel { violation, snippet } => write!(
                f,
                "Tier J: content-model violation ({violation}): {snippet}",
            ),
            Self::MarkupIncomplete { violation, snippet } => {
                write!(f, "Tier K: ruby markup incomplete ({violation}): {snippet}")
            }
        }
    }
}

impl Error for Violation {}

// ---------------------------------------------------------------------------
// Predicates — one per tier
// ---------------------------------------------------------------------------

/// Tier A — no bare `［＃` outside `afm-annotation` wrappers.
///
/// The Tier-A canary in predicate form. Strips every annotation
/// wrapper and asserts the remainder contains no `［＃`. Succeeds when
/// the input has no annotations at all (stripping is a no-op).
///
/// # Errors
///
/// Returns [`Violation::BareBracket`] with a ±80-byte snippet and the
/// total leak count when a bare `［＃` survives the strip.
pub fn check_no_bare_bracket(html: &str) -> Result<(), Violation> {
    const NEEDLE: &str = "［＃";
    let stripped = strip_annotation_wrappers(html);
    if let Some(offset) = stripped.find(NEEDLE) {
        let total = stripped.matches(NEEDLE).count();
        return Err(Violation::BareBracket {
            first_offset: offset,
            snippet: first_occurrence_context(&stripped, NEEDLE, 80),
            total,
        });
    }
    Ok(())
}

/// Tier B — rendered HTML contains no lexer PUA sentinel (U+E001–U+E004).
///
/// **Caveat**: if the source itself contained one of these codepoints,
/// the lexer's Phase 0 sanitize emits a diagnostic but does not strip
/// the character, so the sentinel may flow through to the output. This
/// predicate is therefore meaningful only on sources that produced no
/// lexer diagnostics. Property tests that feed random input should
/// gate on `aozora_lexer::lex(src).diagnostics.is_empty()` before calling.
///
/// # Errors
///
/// Returns [`Violation::SentinelLeak`] with the offending codepoint
/// and a snippet when any of U+E001, U+E002, U+E003, or U+E004 appears
/// in `html`.
pub fn check_no_sentinel_leak(html: &str) -> Result<(), Violation> {
    const SENTINELS: &[char] = &['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];
    for &c in SENTINELS {
        let mut buf = [0u8; 4];
        let needle: &str = c.encode_utf8(&mut buf);
        if let Some(offset) = html.find(needle) {
            return Err(Violation::SentinelLeak {
                codepoint: c,
                first_offset: offset,
                snippet: first_occurrence_context_bytes(html, offset, 80),
            });
        }
    }
    Ok(())
}

/// Tier C — heading bodies carry only legitimate classes.
///
/// `<h1>`–`<h6>` bodies must not contain `afm-indent`,
/// `afm-container-indent`, or `afm-annotation` as class tokens. Other
/// Aozora markup (bouten, gaiji, tcy, kaeriten) is allowed inside a
/// heading — only indent markers and raw-annotation wrappers are bugs
/// here (see commit 7f5463a for the landing fix).
///
/// # Errors
///
/// Returns [`Violation::HeadingContaminated`] on the first offending
/// heading found.
pub fn check_heading_integrity(html: &str) -> Result<(), Violation> {
    const FORBIDDEN: &[&str] = &["afm-indent", "afm-container-indent", "afm-annotation"];
    for level in 1u8..=6 {
        let open_marker = format!("<h{level}");
        let close_marker = format!("</h{level}>");
        let mut search_from = 0usize;
        while let Some(rel) = html[search_from..].find(open_marker.as_str()) {
            let tag_start = search_from + rel;
            // The byte after `<hN` must be `>` or whitespace to avoid
            // matching `<h10>` style tags which don't exist but future-
            // proofs the check.
            let after = tag_start + open_marker.len();
            if after >= html.len() {
                break;
            }
            let b = html.as_bytes()[after];
            if b != b'>' && !b.is_ascii_whitespace() {
                search_from = after;
                continue;
            }
            let Some(gt_rel) = html[tag_start..].find('>') else {
                break;
            };
            let body_start = tag_start + gt_rel + 1;
            let Some(close_rel) = html[body_start..].find(close_marker.as_str()) else {
                break;
            };
            let body_end = body_start + close_rel;
            let body = &html[body_start..body_end];
            let tokens = collect_class_tokens(body);
            for &forbidden in FORBIDDEN {
                if tokens.contains(forbidden) {
                    return Err(Violation::HeadingContaminated {
                        level,
                        forbidden_class: forbidden.to_owned(),
                        snippet: first_occurrence_context_bytes(html, tag_start, 80),
                    });
                }
            }
            search_from = body_end + close_marker.len();
        }
    }
    Ok(())
}

/// Tier D — every open tag has a matching close tag (void elements exempted).
///
/// Delegates to [`check_well_formed`] and lifts the first
/// [`WellFormedError`] into a [`Violation::UnbalancedTag`].
///
/// # Errors
///
/// Returns [`Violation::UnbalancedTag`] carrying the first structural
/// error found.
pub fn check_html_tag_balance(html: &str) -> Result<(), Violation> {
    let errors = check_well_formed(html);
    if let Some(first) = errors.into_iter().next() {
        return Err(Violation::UnbalancedTag(first));
    }
    Ok(())
}

/// Tier E — `afm-annotation` wrappers are well-shaped.
///
/// Asserts that every `<span class="afm-annotation" hidden>` has a
/// matching `</span>`, that no such wrapper is nested inside another
/// (stripping would be non-idempotent), and that every wrapper carries
/// the `hidden` attribute.
///
/// Idempotency is verified by running [`strip_annotation_wrappers`]
/// twice and comparing — if the stripper is idempotent on this input,
/// the wrapper shape is sane.
///
/// # Errors
///
/// Returns [`Violation::AnnotationWrapper`] describing the specific
/// shape violation.
pub fn check_annotation_wrapper_shape(html: &str) -> Result<(), Violation> {
    let once = strip_annotation_wrappers(html);
    let twice = strip_annotation_wrappers(&once);
    if once != twice {
        return Err(Violation::AnnotationWrapper {
            violation: "strip_annotation_wrappers is not idempotent",
            snippet: first_occurrence_context(html, AFM_ANNOTATION_OPEN, 80),
        });
    }
    // Nested wrapper detection: an open occurring before the next close.
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find(AFM_ANNOTATION_OPEN) {
        let open_at = search_from + rel;
        let after_open = &html[open_at + AFM_ANNOTATION_OPEN.len()..];
        let next_open = after_open.find(AFM_ANNOTATION_OPEN);
        let next_close = after_open.find(AFM_ANNOTATION_CLOSE);
        match (next_open, next_close) {
            (Some(no), Some(nc)) if no < nc => {
                return Err(Violation::AnnotationWrapper {
                    violation: "nested afm-annotation open before the enclosing close",
                    snippet: first_occurrence_context_bytes(html, open_at, 80),
                });
            }
            (_, None) => {
                return Err(Violation::AnnotationWrapper {
                    violation: "afm-annotation open without matching </span>",
                    snippet: first_occurrence_context_bytes(html, open_at, 80),
                });
            }
            (_, Some(nc)) => {
                search_from = open_at + AFM_ANNOTATION_OPEN.len() + nc + AFM_ANNOTATION_CLOSE.len();
            }
        }
    }
    // Check for `<span class="afm-annotation"` *without* the `hidden` attribute
    // — the exact shape is the only one we emit, so anything else is a bug.
    let variant = r#"<span class="afm-annotation""#;
    let mut scan_from = 0;
    while let Some(rel) = html[scan_from..].find(variant) {
        let at = scan_from + rel;
        // Check the next non-space content is ` hidden>`.
        let after = &html[at + variant.len()..];
        let trimmed = after.trim_start();
        if !trimmed.starts_with("hidden>") && !trimmed.starts_with("hidden ") {
            return Err(Violation::AnnotationWrapper {
                violation: "afm-annotation span missing `hidden` attribute",
                snippet: first_occurrence_context_bytes(html, at, 80),
            });
        }
        scan_from = at + variant.len();
    }
    Ok(())
}

/// Tier F — no XSS markers reach the rendered HTML as *executable*
/// constructs.
///
/// Forbidden shapes:
///
/// * Literal `<script` (case-insensitive) — a raw script-tag opener.
///   Safe as a bare substring check because every text-content path in
///   `aozora_parser::html` and `aozora_parser::aozora::html` escapes
///   `<` to `&lt;`. So `<script` can appear *only* if the renderer
///   emits the tag itself, which is always a bug.
/// * `javascript:` inside an attribute value (between `<` and `>`).
///   The substring may legitimately appear in rendered prose, so we
///   require it to live inside a tag body — the only position where
///   a browser would act on it.
/// * `on<event>=` (`onerror=`, `onload=`, `onclick=`, …) inside an
///   attribute position. Same tag-context requirement as above — the
///   `onerror=alert(1)` text inside a `<span hidden>` annotation
///   wrapper is harmless plain text, not a handler.
///
/// # Errors
///
/// Returns [`Violation::XssLeak`] naming the detected marker and a
/// ±80-byte snippet.
pub fn check_no_xss_marker(html: &str) -> Result<(), Violation> {
    if let Some(offset) = find_ascii_ignore_case(html, "<script") {
        return Err(Violation::XssLeak {
            marker: "<script",
            first_offset: offset,
            snippet: first_occurrence_context_bytes(html, offset, 80),
        });
    }
    if let Some(offset) = find_javascript_uri_in_tag(html) {
        return Err(Violation::XssLeak {
            marker: "javascript:",
            first_offset: offset,
            snippet: first_occurrence_context_bytes(html, offset, 80),
        });
    }
    if let Some(offset) = find_event_handler_attribute(html) {
        return Err(Violation::XssLeak {
            marker: "on<event>=",
            first_offset: offset,
            snippet: first_occurrence_context_bytes(html, offset, 80),
        });
    }
    Ok(())
}

/// Tier G — every `afm-*` class token is recognised.
///
/// A class token is recognised when it is either a direct entry in
/// [`AFM_CLASSES`] or of the form `afm-X-N` where `afm-X` is in the
/// list and `N` is a non-negative integer (covers the numeric-suffix
/// modifier classes `afm-indent-N`, `afm-align-end-N`,
/// `afm-container-indent-N`).
///
/// Non-`afm-` classes (e.g. classes from a downstream Markdown
/// integration that wraps the aozora layer with its own renderer)
/// are ignored.
///
/// # Errors
///
/// Returns [`Violation::UnknownCssClass`] on the first unrecognised
/// afm class.
pub fn check_css_class_contract(html: &str) -> Result<(), Violation> {
    let tokens = collect_class_tokens(html);
    for token in &tokens {
        if !token.starts_with("afm-") {
            continue;
        }
        if is_recognised_afm_class(token) {
            continue;
        }
        return Err(Violation::UnknownCssClass {
            class: token.clone(),
            snippet: first_occurrence_context(html, token, 80),
        });
    }
    Ok(())
}

/// Tier I — no double-encoded HTML entities.
///
/// Detects the patterns that indicate the escape pass ran twice —
/// `&amp;lt;`, `&amp;gt;`, `&amp;amp;`, `&amp;quot;`, `&amp;#x27;` —
/// any of which means `&lt;` was re-escaped into `&amp;lt;`.
///
/// # Errors
///
/// Returns [`Violation::DoubleEncodedEntity`] at the first offender.
pub fn check_escape_invariants(html: &str) -> Result<(), Violation> {
    const DOUBLE_ENCODED: &[&str] = &[
        "&amp;lt;",
        "&amp;gt;",
        "&amp;amp;",
        "&amp;quot;",
        "&amp;#x27;",
        "&amp;#39;",
    ];
    for &needle in DOUBLE_ENCODED {
        if let Some(offset) = html.find(needle) {
            return Err(Violation::DoubleEncodedEntity {
                snippet: first_occurrence_context_bytes(html, offset, 80),
            });
        }
    }
    Ok(())
}

/// Tier J — HTML content-model correctness for the handful of elements
/// the afm renderer emits under structural constraints.
///
/// Specifically: every `<rt>` and `<rp>` must appear inside an open
/// `<ruby>` element. A stray `<rt>` outside `<ruby>` indicates the
/// renderer emitted a fragment without its containing element (e.g. a
/// post-process bug that detached the base).
///
/// # Errors
///
/// Returns [`Violation::ContentModel`] naming the orphaned element.
pub fn check_content_model(html: &str) -> Result<(), Violation> {
    let mut ruby_depth: i32 = 0;
    let mut i = 0usize;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(gt) = html[i..].find('>') else {
            break;
        };
        let inside = &html[i + 1..i + gt];
        let trimmed = inside.trim();
        let (is_close, body) = trimmed
            .strip_prefix('/')
            .map_or((false, trimmed), |rest| (true, rest.trim_start()));
        let name_end = body
            .char_indices()
            .find(|(_, c)| !c.is_ascii_alphanumeric())
            .map_or(body.len(), |(ix, _)| ix);
        let name = body[..name_end].to_ascii_lowercase();
        match name.as_str() {
            "ruby" if is_close => ruby_depth = ruby_depth.saturating_sub(1),
            "ruby" => ruby_depth += 1,
            "rt" | "rp" if ruby_depth == 0 => {
                return Err(Violation::ContentModel {
                    violation: "<rt> or <rp> outside <ruby>",
                    snippet: first_occurrence_context_bytes(html, i, 80),
                });
            }
            _ => {}
        }
        i += gt + 1;
    }
    Ok(())
}

/// Tier K — every `<ruby>` that opens with `<rp>(</rp>` also closes
/// with the matching `<rp>)</rp>` before `</ruby>`.
///
/// Ruby elements emitted by the afm renderer always carry both rp
/// parentheses; an asymmetric shape is a render bug. We scan every
/// `<ruby>` … `</ruby>` slice and enforce the pair.
///
/// # Errors
///
/// Returns [`Violation::MarkupIncomplete`] describing the missing half.
pub fn check_markup_completeness(html: &str) -> Result<(), Violation> {
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find("<ruby>") {
        let ruby_start = search_from + rel;
        let Some(close_rel) = html[ruby_start..].find("</ruby>") else {
            return Err(Violation::MarkupIncomplete {
                violation: "<ruby> without matching </ruby>",
                snippet: first_occurrence_context_bytes(html, ruby_start, 80),
            });
        };
        let ruby_end = ruby_start + close_rel;
        let body = &html[ruby_start..ruby_end];
        let open_paren = body.contains("<rp>(</rp>");
        let close_paren = body.contains("<rp>)</rp>");
        if open_paren != close_paren {
            return Err(Violation::MarkupIncomplete {
                violation: if open_paren {
                    "<ruby> carries `<rp>(</rp>` without `<rp>)</rp>`"
                } else {
                    "<ruby> carries `<rp>)</rp>` without `<rp>(</rp>`"
                },
                snippet: first_occurrence_context_bytes(html, ruby_start, 80),
            });
        }
        search_from = ruby_end + "</ruby>".len();
    }
    Ok(())
}

/// Aggregate runner: apply every predicate and collect all diagnostics.
///
/// Returns `Ok(())` when every predicate is satisfied; otherwise returns
/// the full list of violations. Property tests that want one red per
/// failing invariant should pattern-match on individual predicates;
/// fixture / corpus callers that only need a pass/fail should use this.
///
/// # Errors
///
/// Returns `Err(Vec<Violation>)` containing every violation found.
pub fn assert_invariants(html: &str) -> Result<(), Vec<Violation>> {
    type Predicate = fn(&str) -> Result<(), Violation>;
    let predicates: &[Predicate] = &[
        check_no_bare_bracket,
        check_no_sentinel_leak,
        check_heading_integrity,
        check_html_tag_balance,
        check_annotation_wrapper_shape,
        check_no_xss_marker,
        check_css_class_contract,
        check_escape_invariants,
        check_content_model,
        check_markup_completeness,
    ];
    let violations: Vec<_> = predicates.iter().filter_map(|p| p(html).err()).collect();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

// ---------------------------------------------------------------------------
// Shared predicate helpers
// ---------------------------------------------------------------------------

/// Collect every token that appears inside a `class="..."` attribute
/// value anywhere in `html`. Used by the heading-integrity and
/// CSS-class-contract predicates; shared so the tokeniser stays
/// consistent across both.
fn collect_class_tokens(html: &str) -> HashSet<String> {
    const NEEDLE: &str = "class=\"";
    let mut out = HashSet::new();
    let mut rest = html;
    while let Some(at) = rest.find(NEEDLE) {
        let after = &rest[at + NEEDLE.len()..];
        let Some(close) = after.find('"') else {
            break;
        };
        let value = &after[..close];
        for tok in value.split_whitespace() {
            out.insert(tok.to_owned());
        }
        rest = &after[close + 1..];
    }
    out
}

/// Accept a class token when it is either directly in [`AFM_CLASSES`]
/// or of the form `<listed-base>-N` where `N` is a non-negative
/// decimal integer.
fn is_recognised_afm_class(class: &str) -> bool {
    const NUMERIC_SUFFIX_BASES: &[&str] = &["afm-indent", "afm-align-end", "afm-container-indent"];
    if AFM_CLASSES.contains(&class) {
        return true;
    }
    for &base in NUMERIC_SUFFIX_BASES {
        if let Some(rest) = class.strip_prefix(base)
            && let Some(tail) = rest.strip_prefix('-')
            && !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

/// Case-insensitive ASCII substring search. Both `needle` and the
/// scanned bytes are lower-cased before comparison; only ASCII chars
/// are folded so non-ASCII stays byte-comparable.
fn find_ascii_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    let haystack_bytes = haystack.as_bytes();
    let needle_lower: Vec<u8> = needle.bytes().map(|b| b.to_ascii_lowercase()).collect();
    if needle_lower.is_empty() || haystack_bytes.len() < needle_lower.len() {
        return None;
    }
    for i in 0..=haystack_bytes.len() - needle_lower.len() {
        let window = &haystack_bytes[i..i + needle_lower.len()];
        if window
            .iter()
            .zip(&needle_lower)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            return Some(i);
        }
    }
    None
}

/// Detect `on<event>=` attribute handlers inside tag bodies.
///
/// Walks `html` tracking whether the cursor is between `<` and `>`
/// (tag-body context). Only matches `on[a-z]+[ ]*=` in that context —
/// the same pattern in text content (`"use onerror= to hook errors"`
/// as prose) is harmless and must not fire.
fn find_event_handler_attribute(html: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut in_tag = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            _ if in_tag && i + 3 < bytes.len() => {
                let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace() || bytes[i - 1] == b'<';
                if prev_ok
                    && bytes[i].eq_ignore_ascii_case(&b'o')
                    && bytes[i + 1].eq_ignore_ascii_case(&b'n')
                    && bytes[i + 2].is_ascii_lowercase()
                {
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j].is_ascii_lowercase() {
                        j += 1;
                    }
                    while j < bytes.len() && bytes[j] == b' ' {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'=' {
                        return Some(i);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Detect `javascript:` URI scheme inside tag bodies. Same
/// tag-context rule as [`find_event_handler_attribute`] — the string
/// in plain prose is harmless, only the in-attribute occurrence is a
/// browser-executable XSS vector.
fn find_javascript_uri_in_tag(html: &str) -> Option<usize> {
    const NEEDLE: &[u8] = b"javascript:";
    let bytes = html.as_bytes();
    let mut in_tag = false;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            _ if in_tag && i + NEEDLE.len() <= bytes.len() => {
                let window = &bytes[i..i + NEEDLE.len()];
                if window
                    .iter()
                    .zip(NEEDLE)
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
                {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// HTML well-formedness validator (relocated from tests/common/mod.rs)
// ---------------------------------------------------------------------------

/// Kinds of structural violations [`check_well_formed`] reports.
///
/// `near` snippets surface ±48 characters around the offending byte
/// offset so the failure message is actionable without a full dump of
/// the rendered HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WellFormedError {
    UnclosedTag {
        tag: String,
        near: String,
    },
    ExtraClose {
        tag: String,
        near: String,
    },
    MisorderedClose {
        opened: String,
        closed: String,
        near: String,
    },
    MalformedTag {
        near: String,
    },
}

impl fmt::Display for WellFormedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedTag { tag, near } => {
                write!(f, "unclosed <{tag}> near {near:?}")
            }
            Self::ExtraClose { tag, near } => {
                write!(f, "extra </{tag}> near {near:?}")
            }
            Self::MisorderedClose {
                opened,
                closed,
                near,
            } => write!(
                f,
                "</{closed}> closes while <{opened}> is still open, near {near:?}"
            ),
            Self::MalformedTag { near } => {
                write!(f, "malformed tag near {near:?}")
            }
        }
    }
}

/// Return the list of well-formedness violations in `html`. Empty
/// vector means the document is balanced.
///
/// Runs in a single forward pass over the bytes; the open-tag stack
/// keeps O(depth) memory. Void-element recognition is a sorted
/// `&'static [&str]` scanned with `binary_search`.
#[must_use]
pub fn check_well_formed(html: &str) -> Vec<WellFormedError> {
    let mut errors = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(lt) = find_from(bytes, i, b'<') else {
            break;
        };
        let Some(gt) = find_from(bytes, lt + 1, b'>') else {
            errors.push(WellFormedError::MalformedTag {
                near: snippet(html, lt),
            });
            break;
        };
        let inside = &html[lt + 1..gt];
        match parse_tag(inside) {
            Some(Tag::Open(name)) => {
                if !is_void_element(&name) {
                    stack.push((name, lt));
                }
            }
            Some(Tag::Close(name)) => match stack.pop() {
                Some((top, _)) if top == name => {}
                Some((top, top_pos)) => {
                    errors.push(WellFormedError::MisorderedClose {
                        opened: top.clone(),
                        closed: name,
                        near: snippet(html, lt),
                    });
                    stack.push((top, top_pos));
                }
                None => errors.push(WellFormedError::ExtraClose {
                    tag: name,
                    near: snippet(html, lt),
                }),
            },
            Some(Tag::SelfClose | Tag::Doctype | Tag::Comment) => {}
            None => errors.push(WellFormedError::MalformedTag {
                near: snippet(html, lt),
            }),
        }
        i = gt + 1;
    }

    for (name, pos) in stack {
        errors.push(WellFormedError::UnclosedTag {
            tag: name,
            near: snippet(html, pos),
        });
    }
    errors
}

enum Tag {
    Open(String),
    Close(String),
    SelfClose,
    Doctype,
    Comment,
}

fn parse_tag(inside: &str) -> Option<Tag> {
    let trimmed = inside.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('!') {
        return Some(Tag::Doctype);
    }
    if trimmed.starts_with('?') {
        return Some(Tag::Comment);
    }
    let (is_close, body) = trimmed
        .strip_prefix('/')
        .map_or((false, trimmed), |rest| (true, rest.trim_start()));
    let (name, rest) = split_tag_name(body)?;
    if name.is_empty() {
        return None;
    }
    let self_closing = rest.trim_end().ends_with('/');
    if is_close {
        Some(Tag::Close(name))
    } else if self_closing {
        drop(name);
        Some(Tag::SelfClose)
    } else {
        Some(Tag::Open(name))
    }
}

fn split_tag_name(body: &str) -> Option<(String, &str)> {
    let end = body
        .char_indices()
        .find(|(_, c)| !is_tag_name_char(*c))
        .map_or(body.len(), |(i, _)| i);
    if end == 0 {
        return None;
    }
    let name = body[..end].to_ascii_lowercase();
    let rest = &body[end..];
    Some((name, rest))
}

const fn is_tag_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void_element(name: &str) -> bool {
    VOID_ELEMENTS.binary_search(&name).is_ok()
}

fn find_from(bytes: &[u8], start: usize, target: u8) -> Option<usize> {
    bytes
        .get(start..)
        .and_then(|slice| slice.iter().position(|&b| b == target))
        .map(|rel| rel + start)
}

fn snippet(html: &str, pos: usize) -> String {
    let lo = html
        .char_indices()
        .take_while(|(i, _)| i + 48 <= pos)
        .last()
        .map_or(0, |(i, _)| i);
    let hi = html
        .char_indices()
        .find(|(i, _)| *i >= pos + 48)
        .map_or(html.len(), |(i, _)| i);
    let lo = clamp_to_char_boundary(html, lo);
    let hi = clamp_to_char_boundary(html, hi);
    html.get(lo..hi).unwrap_or(html).to_owned()
}

fn clamp_to_char_boundary(s: &str, mut i: usize) -> usize {
    if i > s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // strip_annotation_wrappers (existing)
    // -------------------------------------------------------------------

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
        let out = strip_annotation_wrappers(html);
        assert!(out.contains("X b"));
    }

    #[test]
    fn first_occurrence_context_snaps_to_char_boundaries() {
        let text = "ああああ［＃改ページ］ええええ";
        let ctx = first_occurrence_context(text, "［＃", 4);
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

    // -------------------------------------------------------------------
    // Invariant predicates — unit pinning
    //
    // Test names are prefixed `invariant_unit_` so `just invariants`
    // can filter just these out of the broader suite.
    // -------------------------------------------------------------------

    fn clean_html() -> &'static str {
        r#"<p>hello world</p><div class="afm-container"><p>inside</p></div>"#
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_passes_on_clean_input() {
        check_no_bare_bracket(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_fires_on_leak() {
        let html = "<p>leak ［＃改ページ］ here</p>";
        let Err(Violation::BareBracket { total, .. }) = check_no_bare_bracket(html) else {
            panic!("expected BareBracket violation");
        };
        assert_eq!(total, 1);
    }

    #[test]
    fn invariant_unit_check_no_bare_bracket_tolerates_wrapper() {
        let html = r#"<span class="afm-annotation" hidden>［＃改ページ］</span>"#;
        check_no_bare_bracket(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_sentinel_leak_passes_on_clean_input() {
        check_no_sentinel_leak(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_sentinel_leak_fires_on_each_sentinel() {
        for c in ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
            let html = format!("x{c}y");
            let err = check_no_sentinel_leak(&html).expect_err("must leak");
            assert!(
                matches!(err, Violation::SentinelLeak { codepoint, .. } if codepoint == c),
                "expected SentinelLeak for {c:?}, got {err:?}",
            );
        }
    }

    #[test]
    fn invariant_unit_check_heading_integrity_passes_on_bouten_inside_heading() {
        let html =
            r#"<h1>本文<em class="afm-bouten afm-bouten-goma afm-bouten-right">強</em></h1>"#;
        check_heading_integrity(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_heading_integrity_fires_on_indent_leak() {
        let html =
            r#"<h1><span class="afm-indent afm-indent-2" data-amount="2"></span>第一篇</h1>"#;
        let Err(Violation::HeadingContaminated {
            level,
            forbidden_class,
            ..
        }) = check_heading_integrity(html)
        else {
            panic!("expected HeadingContaminated");
        };
        assert_eq!(level, 1);
        assert_eq!(forbidden_class, "afm-indent");
    }

    #[test]
    fn invariant_unit_check_heading_integrity_fires_on_annotation_leak() {
        let html = r#"<h2><span class="afm-annotation" hidden>［＃X］</span>第一篇</h2>"#;
        let Err(Violation::HeadingContaminated {
            level,
            forbidden_class,
            ..
        }) = check_heading_integrity(html)
        else {
            panic!("expected HeadingContaminated");
        };
        assert_eq!(level, 2);
        assert_eq!(forbidden_class, "afm-annotation");
    }

    #[test]
    fn invariant_unit_check_html_tag_balance_passes_on_clean_input() {
        check_html_tag_balance(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_html_tag_balance_fires_on_unclosed_div() {
        let html = "<p>x</p><div>y";
        let err = check_html_tag_balance(html).expect_err("must fire");
        assert!(matches!(err, Violation::UnbalancedTag(_)));
    }

    #[test]
    fn invariant_unit_check_annotation_wrapper_shape_passes_on_well_formed() {
        let html = r#"a <span class="afm-annotation" hidden>X</span> b"#;
        check_annotation_wrapper_shape(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_annotation_wrapper_shape_fires_on_missing_hidden() {
        let html = r#"a <span class="afm-annotation">X</span> b"#;
        let err = check_annotation_wrapper_shape(html).expect_err("must fire");
        assert!(matches!(err, Violation::AnnotationWrapper { .. }));
    }

    #[test]
    fn invariant_unit_check_annotation_wrapper_shape_fires_on_unclosed() {
        let html = r#"a <span class="afm-annotation" hidden>X b"#;
        let err = check_annotation_wrapper_shape(html).expect_err("must fire");
        assert!(matches!(err, Violation::AnnotationWrapper { .. }));
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_passes_on_clean_input() {
        check_no_xss_marker(clean_html()).unwrap();
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_fires_on_script_tag() {
        let err = check_no_xss_marker("<p><script>x</script></p>").expect_err("must fire");
        assert!(matches!(
            err,
            Violation::XssLeak {
                marker: "<script",
                ..
            }
        ));
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_fires_on_javascript_uri() {
        let err = check_no_xss_marker(r#"<a href="javascript:x">go</a>"#).expect_err("must fire");
        assert!(matches!(
            err,
            Violation::XssLeak {
                marker: "javascript:",
                ..
            }
        ));
    }

    #[test]
    fn invariant_unit_check_no_xss_marker_fires_on_onerror_attr() {
        let err = check_no_xss_marker("<img src=x onerror=alert(1)>").expect_err("must fire");
        assert!(matches!(
            err,
            Violation::XssLeak {
                marker: "on<event>=",
                ..
            }
        ));
    }

    #[test]
    fn invariant_unit_check_css_class_contract_passes_on_known_classes() {
        let html =
            r#"<div class="afm-container afm-container-indent afm-container-indent-2">x</div>"#;
        check_css_class_contract(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_css_class_contract_accepts_afm_indent_numeric_suffix() {
        let html = r#"<span class="afm-indent afm-indent-3">x</span>"#;
        check_css_class_contract(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_css_class_contract_ignores_non_afm_classes() {
        let html = r#"<pre class="language-rust">let x = 1;</pre>"#;
        check_css_class_contract(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_css_class_contract_fires_on_unknown_afm_class() {
        let html = r#"<span class="afm-mystery-variant">x</span>"#;
        let Err(Violation::UnknownCssClass { class, .. }) = check_css_class_contract(html) else {
            panic!("expected UnknownCssClass");
        };
        assert_eq!(class, "afm-mystery-variant");
    }

    #[test]
    fn invariant_unit_check_escape_invariants_passes_on_single_escape() {
        let html = "<p>&lt;script&gt;</p>";
        check_escape_invariants(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_escape_invariants_fires_on_double_encoded() {
        let html = "<p>&amp;lt;oops&amp;gt;</p>";
        let err = check_escape_invariants(html).expect_err("must fire");
        assert!(matches!(err, Violation::DoubleEncodedEntity { .. }));
    }

    #[test]
    fn invariant_unit_check_content_model_passes_on_ruby_shape() {
        let html = "<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>";
        check_content_model(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_content_model_fires_on_orphan_rt() {
        let html = "<p><rt>orphan</rt></p>";
        let err = check_content_model(html).expect_err("must fire");
        assert!(matches!(err, Violation::ContentModel { .. }));
    }

    #[test]
    fn invariant_unit_check_markup_completeness_passes_on_symmetric_rp() {
        let html = "<ruby>x<rp>(</rp><rt>y</rt><rp>)</rp></ruby>";
        check_markup_completeness(html).unwrap();
    }

    #[test]
    fn invariant_unit_check_markup_completeness_fires_on_missing_close_paren() {
        let html = "<ruby>x<rp>(</rp><rt>y</rt></ruby>";
        let err = check_markup_completeness(html).expect_err("must fire");
        assert!(matches!(err, Violation::MarkupIncomplete { .. }));
    }

    #[test]
    fn invariant_unit_assert_invariants_aggregates_clean_pass() {
        let html =
            r#"<ruby>青<rp>(</rp><rt>あ</rt><rp>)</rp></ruby><span class="afm-tcy">20</span>"#;
        assert_invariants(html).unwrap();
    }

    #[test]
    fn invariant_unit_assert_invariants_collects_multiple_violations() {
        // Bare bracket + unknown class + missing rp in one sample.
        let html = r#"<ruby>x<rp>(</rp><rt>y</rt></ruby><span class="afm-unknown">［＃X］</span>"#;
        let violations = assert_invariants(html).expect_err("must fire");
        assert!(!violations.is_empty());
    }

    // -------------------------------------------------------------------
    // Well-formedness validator smoke (inherited from tests/common/mod.rs)
    // -------------------------------------------------------------------

    #[test]
    fn invariant_unit_well_formed_accepts_balanced_doc() {
        assert!(check_well_formed("<p>x<em>y</em></p>").is_empty());
    }

    #[test]
    fn invariant_unit_well_formed_flags_unclosed() {
        let errs = check_well_formed("<p>x");
        assert!(
            errs.iter()
                .any(|e| matches!(e, WellFormedError::UnclosedTag { .. }))
        );
    }

    #[test]
    fn invariant_unit_well_formed_flags_extra_close() {
        let errs = check_well_formed("</p>");
        assert!(
            errs.iter()
                .any(|e| matches!(e, WellFormedError::ExtraClose { .. }))
        );
    }

    #[test]
    fn invariant_unit_well_formed_accepts_void_elements() {
        assert!(check_well_formed("<p>x<br>y<hr></p>").is_empty());
    }
}
