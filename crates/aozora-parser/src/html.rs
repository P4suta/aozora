//! HTML rendering — `parse`-and-render plus direct-from-artifacts.
//!
//! # Algorithm
//!
//! Single forward `match_indices` sweep over the lexer's normalized
//! text, capturing every structural character (the four PUA
//! sentinels + `\n`) in `O(n)` with no intermediate allocations
//! between matches. Plain text between matches is bulk-copied via
//! `push_str` (one `memcpy` per run) so the inner loop stays
//! branch-predictable on long Japanese prose runs where every
//! character is 3 UTF-8 bytes and a char-by-char walk would pay 3×
//! the bookkeeping cost.
//!
//! HTML escaping inside each plain chunk is handled by a second,
//! nested `match_indices` over `<>&"'` — same bulk-copy strategy,
//! same complexity. The two passes share no state, so the inner
//! escape pass can be inlined or extracted without affecting the
//! outer block walker.
//!
//! Block structure is encoded as an explicit two-state machine
//! [`RenderState::Outside`] ↔ [`RenderState::InParagraph`]; the
//! transitions live on a single method that always emits
//! `<p>` / `</p>` symmetrically, so the output is well-formed by
//! construction. Container nesting is delegated to
//! [`crate::aozora::html::render`] (it emits an opening `<div>` on
//! enter and a closing `</div>` on exit, which we drive by querying
//! the registry at the open and close sentinel positions).
//!
//! # Block model
//!
//! - Two consecutive newlines (`\n\n`) close the current paragraph.
//! - A single newline inside a paragraph emits `<br />`.
//! - `BLOCK_LEAF_SENTINEL` (`U+E002`) flushes the paragraph and
//!   emits the corresponding standalone block element.
//! - `BLOCK_OPEN_SENTINEL` / `BLOCK_CLOSE_SENTINEL` (`U+E003` /
//!   `U+E004`) flush the paragraph and emit the container's
//!   opening / closing tag respectively.
//! - `INLINE_SENTINEL` (`U+E001`) inside a paragraph dispatches the
//!   inline node into the current paragraph buffer.

use core::fmt;

use aozora_lex::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
use aozora_syntax::{AozoraNode, Container};

use crate::aozora::html::render as render_node;
use crate::{ParseArtifacts, parse};

/// Render `input` to HTML and return the result as a `String`.
///
/// Thin wrapper over [`render_to_writer`]. For streaming consumers
/// (HTTP responses, file writers, indexer pipelines) prefer the
/// writer-based form to avoid the intermediate `String` allocation.
///
/// # Panics
///
/// Does not panic: `String` cannot fail as a [`fmt::Write`] sink.
/// See [`render_from_artifacts_to_writer`] for the upstream length
/// invariant (`u32::MAX` source cap from Phase 0 sanitize).
#[must_use]
pub fn render_to_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len().saturating_mul(2));
    render_to_writer(input, &mut out).expect("writing to String never fails");
    out
}

/// Render `input` to HTML, streaming each emitted byte into `writer`.
///
/// `writer` can be any [`fmt::Write`] sink — a `String`, a
/// `core::fmt::Formatter`, or a custom destination such as a Tower
/// `Body`-shaped streaming response. The renderer never buffers more
/// than one inter-sentinel chunk at a time, so the caller controls
/// memory shape.
///
/// # Errors
///
/// Propagates any error returned by `writer`. The renderer never
/// produces an error of its own — every internal failure path is
/// either an `unreachable!()` covered by `is_structural_char`'s
/// closure of the predicate set, or a registry-miss that the lexer's
/// V2/V3 diagnostics flagged earlier.
pub fn render_to_writer<W: fmt::Write>(input: &str, writer: &mut W) -> fmt::Result {
    render_from_artifacts_to_writer(&parse(input).artifacts, writer)
}

/// Render directly from a [`ParseArtifacts`] bundle into a `String`.
///
/// Useful for callers that already retained artifacts from a prior
/// [`parse`] (LSP backends, watch-mode tools that diff renders). For
/// streaming output use [`render_from_artifacts_to_writer`].
///
/// # Panics
///
/// Does not panic: `String` cannot fail as a [`fmt::Write`] sink.
/// See [`render_from_artifacts_to_writer`] for the upstream length
/// invariant.
#[must_use]
pub fn render_from_artifacts(artifacts: &ParseArtifacts) -> String {
    // Capacity heuristic: plain prose adds `<p>`/`</p>\n` per
    // paragraph (~7 bytes), HTML escape is ~5% on natural text, and
    // each inline sentinel expands to ~50 bytes of markup. We can't
    // know the sentinel count without scanning, so settle on a
    // conservative 2× factor; reallocation cost is amortised by
    // `String::push_str`'s exponential growth.
    let mut out = String::with_capacity(artifacts.normalized.len().saturating_mul(2));
    render_from_artifacts_to_writer(artifacts, &mut out)
        .expect("writing to String never fails");
    out
}

/// Render from a [`ParseArtifacts`] bundle, streaming HTML into
/// `writer`.
///
/// The body is shared with [`render_to_writer`]; this is the form
/// to use when the caller has already paid for the parse and wants
/// to fan it out to multiple sinks.
///
/// # Errors
///
/// Propagates `writer` failures.
///
/// # Panics
///
/// Does not panic on its own. The lexer's Phase 0 sanitize caps
/// input length at `u32::MAX` bytes, which guarantees every
/// `match_indices` position fits in `u32`; if that invariant is
/// ever broken upstream, the index conversion would panic.
pub fn render_from_artifacts_to_writer<W: fmt::Write>(
    artifacts: &ParseArtifacts,
    writer: &mut W,
) -> fmt::Result {
    let normalized = &artifacts.normalized;
    let registry = &artifacts.registry;
    let mut state = RenderState::default();

    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    for (pos, match_str) in normalized.match_indices(is_structural_char) {
        // Bulk-copy the plain chunk between the previous cursor and
        // this match into the current paragraph (opening one if
        // needed). One `write_str` per chunk, escape pass over each.
        if cursor < pos {
            state.ensure_in_paragraph(writer)?;
            escape_text_chunk(&normalized[cursor..pos], writer)?;
        }

        // Single-codepoint sentinels — first char is the only char.
        let ch = match_str
            .chars()
            .next()
            .expect("match_indices yields non-empty match");
        let byte_pos = u32::try_from(pos).expect("normalized fits u32 per Phase 0 cap");

        match ch {
            INLINE_SENTINEL => {
                if let Some(node) = registry.inline_at(byte_pos) {
                    state.ensure_in_paragraph(writer)?;
                    render_node(node, true, writer)?;
                }
            }
            BLOCK_LEAF_SENTINEL => {
                if let Some(node) = registry.block_leaf_at(byte_pos) {
                    state.before_block_emit(writer)?;
                    render_node(node, true, writer)?;
                    state.after_block_emit();
                }
            }
            BLOCK_OPEN_SENTINEL => {
                if let Some(kind) = registry.block_open_at(byte_pos) {
                    state.before_block_emit(writer)?;
                    let node = AozoraNode::Container(Container { kind });
                    render_node(&node, true, writer)?;
                    state.after_block_emit();
                }
            }
            BLOCK_CLOSE_SENTINEL => {
                if let Some(kind) = registry.block_close_at(byte_pos) {
                    state.before_block_emit(writer)?;
                    let node = AozoraNode::Container(Container { kind });
                    render_node(&node, false, writer)?;
                    state.after_block_emit();
                }
            }
            '\n' => {
                // `\n\n` closes the current paragraph; bare `\n`
                // inside a paragraph is a hard break, except when it
                // is the *trailing* `\n` of the buffer — in that case
                // the post-loop flush will close the paragraph and a
                // `<br />` would dangle at the end of the line. Peek
                // the next byte directly; it is ASCII either way so
                // the byte view is safe.
                match bytes.get(pos + 1) {
                    Some(&b'\n') => state.close_paragraph(writer)?,
                    Some(_) if state.in_paragraph => writer.write_str("<br />\n")?,
                    Some(_) | None => {}
                }
            }
            _ => unreachable!("is_structural_char admitted only sentinels and \\n"),
        }
        cursor = pos + match_str.len();
    }

    // Flush trailing plain chunk and close any dangling paragraph.
    if cursor < normalized.len() {
        state.ensure_in_paragraph(writer)?;
        escape_text_chunk(&normalized[cursor..], writer)?;
    }
    state.close_paragraph(writer)?;
    Ok(())
}

/// Block-level walker state.
///
/// Two flags drive the structural emission:
///
/// - `in_paragraph` — a `<p>` has been opened and is still waiting
///   for `</p>`. `close_paragraph` is the only path that emits the
///   close tag (always with the trailing `\n` separator).
/// - `pending_block_separator` — a non-paragraph block element was
///   the most recent block emission. Block elements rendered by
///   [`crate::aozora::html::render`] produce no trailing newline,
///   so this flag tells the next block emission (a paragraph open
///   or another standalone block) to prepend a separator `\n` and
///   reset itself. Trailing `\n` in the output buffer therefore
///   appears only when more content follows; a standalone block
///   element finishes without dangling whitespace.
///
/// Together they make the renderer's spacing convention match the
/// fixtures exactly without lookahead: each block element decides
/// "do I need a separator before me?" based on what came before,
/// not what comes after.
#[derive(Debug, Default)]
struct RenderState {
    in_paragraph: bool,
    pending_block_separator: bool,
}

impl RenderState {
    fn flush_pending_separator<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.pending_block_separator {
            out.write_char('\n')?;
            self.pending_block_separator = false;
        }
        Ok(())
    }

    fn ensure_in_paragraph<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if !self.in_paragraph {
            self.flush_pending_separator(out)?;
            out.write_str("<p>")?;
            self.in_paragraph = true;
        }
        Ok(())
    }

    fn close_paragraph<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.in_paragraph {
            out.write_str("</p>\n")?;
            self.in_paragraph = false;
            // Paragraph close already emitted its own `\n`; the next
            // block does not need an extra separator.
            self.pending_block_separator = false;
        }
        Ok(())
    }

    fn before_block_emit<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        self.close_paragraph(out)?;
        self.flush_pending_separator(out)
    }

    fn after_block_emit(&mut self) {
        self.pending_block_separator = true;
    }
}

/// Predicate matched by the outer `match_indices` sweep.
///
/// Inlined as an `#[inline]` pure function so the optimiser can fold
/// it into the `match_indices` adapter — letting LLVM lower the scan
/// to a `memchr`-class implementation when the predicate is small
/// enough.
#[inline]
fn is_structural_char(c: char) -> bool {
    matches!(
        c,
        INLINE_SENTINEL | BLOCK_LEAF_SENTINEL | BLOCK_OPEN_SENTINEL | BLOCK_CLOSE_SENTINEL | '\n'
    )
}

/// Bulk-escape a plain-text slice into the output writer.
///
/// Uses a nested `match_indices` over the five HTML-unsafe ASCII
/// characters; the typical Japanese-prose chunk hits zero matches
/// and reduces to a single `write_str`. The escape pass shares the
/// outer's bulk-copy discipline.
fn escape_text_chunk<W: fmt::Write>(chunk: &str, out: &mut W) -> fmt::Result {
    let mut cursor = 0usize;
    for (pos, m) in chunk.match_indices(is_html_unsafe_char) {
        out.write_str(&chunk[cursor..pos])?;
        let ch = m.chars().next().expect("non-empty match");
        out.write_str(html_entity_for(ch))?;
        cursor = pos + m.len();
    }
    out.write_str(&chunk[cursor..])
}

#[inline]
const fn is_html_unsafe_char(c: char) -> bool {
    matches!(c, '<' | '>' | '&' | '"' | '\'')
}

#[inline]
const fn html_entity_for(c: char) -> &'static str {
    match c {
        '<' => "&lt;",
        '>' => "&gt;",
        '&' => "&amp;",
        '"' => "&quot;",
        '\'' => "&#39;",
        _ => "",
    }
}

// Keep the legacy alias working for `aozora-tools` and any in-tree
// example that retained the old name. Free of cost — re-export.
#[doc(hidden)]
pub use render_to_string as render_root_to_string;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn plain_paragraph_wraps_in_p() {
        let html = render_to_string("Hello.");
        assert_eq!(html, "<p>Hello.</p>\n");
    }

    #[test]
    fn ruby_is_emitted_semantically() {
        let html = render_to_string("｜青梅《おうめ》");
        assert!(html.contains("<ruby>青梅"), "missing ruby tag: {html}");
        assert!(html.contains("<rt>おうめ"), "missing rt tag: {html}");
    }

    #[test]
    fn page_break_bracket_annotation_emits_page_break_div() {
        let html = render_to_string("前\n\n［＃改ページ］\n\n後");
        assert!(
            html.contains(r#"<div class="afm-page-break"></div>"#),
            "missing page-break div: {html}"
        );
        assert!(!html.contains("［＃"), "［＃ leaked: {html}");
    }

    #[test]
    fn unknown_bracket_annotation_produces_annotation_wrapper() {
        let html = render_to_string("前［＃ほげふが］後");
        assert!(
            html.contains(r#"class="afm-annotation""#),
            "missing annotation wrapper: {html}"
        );
        assert!(
            !html.contains("［＃ほげふが］") || html.contains(">［＃ほげふが］<"),
            "annotation not consumed: {html}"
        );
    }

    #[test]
    fn html_special_characters_are_escaped() {
        let html = render_to_string("a < b & c > d");
        assert!(html.contains("&lt;"), "< not escaped: {html}");
        assert!(html.contains("&gt;"), "> not escaped: {html}");
        assert!(html.contains("&amp;"), "& not escaped: {html}");
        assert!(!html.contains("<scr"), "raw bytes leaked into HTML: {html}");
    }

    #[test]
    fn double_newline_separates_paragraphs() {
        let html = render_to_string("first\n\nsecond");
        assert_eq!(html, "<p>first</p>\n<p>second</p>\n");
    }

    #[test]
    fn single_newline_in_paragraph_emits_hardbreak() {
        let html = render_to_string("verse line one\nverse line two");
        assert!(html.contains("<br />"), "missing hardbreak: {html}");
        // Only one paragraph: no </p><p> seam.
        assert_eq!(html.matches("<p>").count(), 1);
        assert_eq!(html.matches("</p>").count(), 1);
    }

    #[test]
    fn empty_input_produces_empty_output() {
        assert_eq!(render_to_string(""), "");
    }

    #[test]
    fn run_of_blank_lines_does_not_emit_empty_paragraphs() {
        let html = render_to_string("first\n\n\n\n\nsecond");
        assert_eq!(html.matches("<p>").count(), 2);
        assert_eq!(html.matches("</p>").count(), 2);
        assert!(!html.contains("<p></p>"));
    }

    // ---------------------------------------------------------------
    // State machine — `<p>`/`</p>` must always balance.
    //
    // Drives the rest of the test suite via [`assert_well_formed`]:
    // any new fixture goes through it so a future refactor that
    // breaks the balance shows up as soon as a single test exercises
    // the regressed path.
    // ---------------------------------------------------------------

    /// Assert the basic structural invariants of any rendered HTML:
    /// `<p>` and `</p>` counts match, `<div>` and `</div>` counts
    /// match, no `<p></p>` empty-paragraph holes, no PUA sentinels,
    /// no bare `［＃`, no rolled-back hardbreak.
    fn assert_well_formed(html: &str) {
        let p_open = html.matches("<p>").count();
        let p_close = html.matches("</p>").count();
        assert_eq!(
            p_open, p_close,
            "<p>/</p> imbalance: {p_open} opens vs {p_close} closes in: {html:?}"
        );
        let div_open = html.matches("<div").count();
        let div_close = html.matches("</div>").count();
        assert_eq!(
            div_open, div_close,
            "<div>/</div> imbalance: {div_open} opens vs {div_close} closes in: {html:?}"
        );
        assert!(!html.contains("<p></p>"), "empty <p></p> emitted: {html:?}");
        for sentinel in ["\u{E001}", "\u{E002}", "\u{E003}", "\u{E004}"] {
            assert!(
                !html.contains(sentinel),
                "PUA sentinel {sentinel:?} leaked: {html:?}"
            );
        }
    }

    // ---------------------------------------------------------------
    // Sentinel adjacency
    // ---------------------------------------------------------------

    #[test]
    fn inline_sentinel_at_start_of_paragraph_opens_p_correctly() {
        let html = render_to_string("｜青梅《おうめ》文字");
        assert_well_formed(&html);
        // Opening `<p>` must precede the inline node, not be tucked
        // inside it; a regression that emits `<ruby>...<p>` would
        // fail tag-balance.
        let p_at = html.find("<p>").expect("paragraph opens");
        let ruby_at = html.find("<ruby>").expect("ruby renders");
        assert!(p_at < ruby_at, "<p> must precede <ruby>: {html:?}");
    }

    #[test]
    fn inline_sentinel_at_end_of_paragraph_closes_p_correctly() {
        let html = render_to_string("文字｜青梅《おうめ》");
        assert_well_formed(&html);
        // Inline must be inside the paragraph, not after `</p>`.
        let close_at = html.find("</p>").expect("paragraph closes");
        let ruby_at = html.find("<ruby>").expect("ruby renders");
        assert!(ruby_at < close_at, "<ruby> must precede </p>: {html:?}");
    }

    #[test]
    fn two_adjacent_inline_sentinels_render_in_order() {
        let html = render_to_string("｜青梅《おうめ》｜鶴見《つるみ》");
        assert_well_formed(&html);
        assert_eq!(html.matches("<ruby>").count(), 2);
        let first = html.find("青梅").expect("first ruby base present");
        let second = html.find("鶴見").expect("second ruby base present");
        assert!(first < second, "ruby order preserved: {html:?}");
    }

    #[test]
    fn block_leaf_at_very_start_emits_no_leading_paragraph() {
        let html = render_to_string("［＃改ページ］\n\n後");
        assert_well_formed(&html);
        // Output begins with the div, not a leading `<p>`.
        assert!(
            html.starts_with(r#"<div class="afm-page-break""#),
            "block leaf must lead the output: {html:?}"
        );
    }

    #[test]
    fn block_leaf_at_very_end_emits_no_trailing_paragraph() {
        let html = render_to_string("前\n\n［＃改ページ］");
        assert_well_formed(&html);
        // Trailing `\n` must not introduce a dangling `<p>`.
        assert!(
            html.trim_end().ends_with("</div>"),
            "block leaf must close the output cleanly: {html:?}"
        );
    }

    #[test]
    fn block_leaf_immediately_followed_by_inline_resumes_paragraph() {
        let html = render_to_string("［＃改ページ］\n\n｜青梅《おうめ》");
        assert_well_formed(&html);
        assert!(html.contains(r#"<div class="afm-page-break">"#));
        assert!(html.contains("<ruby>青梅"));
    }

    #[test]
    fn two_consecutive_block_leafs_each_emit_a_div_separated_by_newline() {
        let html = render_to_string("［＃改ページ］\n\n［＃改丁］");
        assert_well_formed(&html);
        let break_at = html
            .find(r#"class="afm-page-break""#)
            .expect("first block leaf");
        let section_at = html
            .find(r#"class="afm-section-break"#)
            .expect("second block leaf");
        assert!(
            break_at < section_at,
            "leafs must render in order: {html:?}"
        );
        // A separator `\n` must sit between the two leafs (otherwise
        // they'd render adjacent like `</div><div>`).
        let between = &html[break_at..section_at];
        assert!(
            between.contains('\n'),
            "missing \\n separator between consecutive block leafs: {between:?}"
        );
    }

    // ---------------------------------------------------------------
    // Containers
    // ---------------------------------------------------------------

    #[test]
    fn empty_container_emits_balanced_div() {
        // Open immediately followed by close (no body) — must still
        // produce a paired `<div>...</div>` in the output.
        let html = render_to_string("［＃ここから2字下げ］\n\n［＃ここで字下げ終わり］");
        assert_well_formed(&html);
        assert!(
            html.contains("afm-container-indent"),
            "missing container class: {html:?}"
        );
    }

    #[test]
    fn container_with_paragraph_body_wraps_p_inside_div() {
        // Aozora spec: `［＃ここから…］` and `［＃ここで…終わり］` must
        // each occupy their own line bordered by blank lines for the
        // lexer to classify them as container open/close.
        let html = render_to_string("［＃ここから2字下げ］\n\n本文\n\n［＃ここで字下げ終わり］");
        assert_well_formed(&html);
        let open_at = html.find("<div").expect("container open tag missing");
        assert!(
            html.contains("afm-container-indent"),
            "container kind class missing: {html:?}"
        );
        let body_at = html.find("本文").expect("body missing");
        let close_at = html.rfind("</div>").expect("container close missing");
        assert!(
            open_at < body_at && body_at < close_at,
            "container body must sit between div open and close: {html:?}"
        );
    }

    // ---------------------------------------------------------------
    // HTML escape — boundary cases against sentinels
    // ---------------------------------------------------------------

    #[test]
    fn lt_immediately_before_inline_sentinel_is_escaped_independently() {
        let html = render_to_string("<｜青梅《おうめ》");
        assert_well_formed(&html);
        // The `<` must be escaped to `&lt;` and not consume the
        // sentinel that follows.
        assert!(html.contains("&lt;"), "< not escaped: {html:?}");
        assert!(html.contains("<ruby>"), "ruby not rendered: {html:?}");
    }

    #[test]
    fn lt_immediately_after_inline_sentinel_is_escaped() {
        let html = render_to_string("｜青梅《おうめ》<");
        assert_well_formed(&html);
        assert!(html.contains("</ruby>"));
        assert!(html.contains("&lt;"));
    }

    #[test]
    fn all_five_html_entities_round_trip_through_escape() {
        let html = render_to_string(r#"<>&"' all in one"#);
        assert_well_formed(&html);
        assert!(html.contains("&lt;"));
        assert!(html.contains("&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;"));
        assert!(html.contains("&#39;"));
    }

    #[test]
    fn pre_escaped_entity_in_source_is_double_escaped_at_render() {
        // CommonMark/aozora source containing `&amp;` is just text;
        // a renderer that emits `&amp;` raw would silently turn into
        // `&` in the browser. We re-escape conservatively.
        let html = render_to_string("&amp;");
        assert_well_formed(&html);
        assert!(
            html.contains("&amp;amp;"),
            "expected double-escape, got: {html:?}"
        );
    }

    #[test]
    fn xss_marker_is_neutralised_in_text_content() {
        let html = render_to_string("<script>alert(1)</script>");
        assert_well_formed(&html);
        // The `<` is escaped at every position; no live `<script>`
        // can survive the renderer's escape pass.
        assert!(
            !html.contains("<script"),
            "raw <script tag survived: {html:?}"
        );
        assert!(html.contains("&lt;script"));
    }

    // ---------------------------------------------------------------
    // Whitespace / empty inputs
    // ---------------------------------------------------------------

    #[test]
    fn whitespace_only_input_produces_at_most_a_paragraph_with_whitespace() {
        // Pure whitespace is ambiguous (CommonMark would elide it);
        // here the contract is "no panic, balanced HTML, no PUA leak".
        // Whether the output is empty or a single whitespace-bearing
        // paragraph is implementation-defined but must be well-formed.
        let inputs = ["   ", "\t\t", "    \t"];
        for input in inputs {
            let html = render_to_string(input);
            assert_well_formed(&html);
        }
    }

    #[test]
    fn single_character_input() {
        let html = render_to_string("x");
        assert_eq!(html, "<p>x</p>\n");
    }

    #[test]
    fn leading_blank_lines_do_not_produce_phantom_paragraph() {
        let html = render_to_string("\n\n\n\nfirst");
        assert_well_formed(&html);
        assert_eq!(html, "<p>first</p>\n");
    }

    #[test]
    fn trailing_blank_lines_do_not_produce_phantom_paragraph() {
        let html = render_to_string("first\n\n\n\n");
        assert_well_formed(&html);
        assert_eq!(html, "<p>first</p>\n");
    }

    // ---------------------------------------------------------------
    // Regression pins — bugs caught during Stage 2
    // ---------------------------------------------------------------

    #[test]
    fn regression_trailing_newline_does_not_emit_dangling_br() {
        // 2026-04-25: an early version of the block walker emitted
        // `<br />` for any single `\n` it encountered, including the
        // trailing newline of the buffer. Spec fixtures expect the
        // trailing `\n` to feed `</p>` directly with no `<br />`
        // before it. Pin via a direct equality.
        let html = render_to_string("責［＃「責」にばつ傍点］めて\n");
        assert_well_formed(&html);
        assert!(
            !html.contains("<br />\n</p>"),
            "trailing newline emitted a dangling <br />: {html:?}"
        );
    }

    #[test]
    fn regression_standalone_block_leaf_has_no_trailing_newline() {
        // 2026-04-25: an early version unconditionally pushed `\n`
        // after every block element. The spec fixture for a
        // standalone `［＃改ページ］` expects the output to end with
        // `</div>` and no trailing `\n`. The fix introduced
        // `pending_block_separator`: the separator is emitted only
        // when more content follows.
        let html = render_to_string("［＃改ページ］\n");
        assert_eq!(html, r#"<div class="afm-page-break"></div>"#);
    }

    #[test]
    fn regression_block_leaf_between_paragraphs_separated_by_newline() {
        // 2026-04-25: the `pending_block_separator` flag must fire
        // when the next emission is a paragraph open (not just
        // another block). Pin the exact mid-paragraph layout.
        let html = render_to_string("前［＃改ページ］後\n");
        assert_eq!(
            html,
            "<p>前</p>\n<div class=\"afm-page-break\"></div>\n<p>後</p>\n"
        );
    }

    // ---------------------------------------------------------------
    // Front-door consistency
    // ---------------------------------------------------------------

    #[test]
    fn render_to_string_equals_render_from_artifacts_via_parse() {
        // The convenience wrapper [`render_to_string`] must produce
        // the exact same bytes as [`render_from_artifacts`] over the
        // result of [`parse`]. Drift between the two front doors is
        // a silent API split.
        let inputs = [
            "Hello.",
            "｜青梅《おうめ》",
            "前［＃改ページ］後",
            "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］",
            "first\n\nsecond",
        ];
        for input in inputs {
            let direct = render_to_string(input);
            let two_step = render_from_artifacts(&parse(input).artifacts);
            assert_eq!(
                direct, two_step,
                "front-door drift on input {input:?}:\n  direct:   {direct:?}\n  two-step: {two_step:?}"
            );
        }
    }

    #[test]
    fn render_is_deterministic_across_repeated_calls() {
        // Calling the renderer twice on the same input must produce
        // byte-identical output. A regression here usually points at
        // hidden state (e.g. a static counter in the lexer).
        let inputs = [
            "",
            "Hello.",
            "｜青梅《おうめ》｜鶴見《つるみ》",
            "前［＃改ページ］後",
            "<>&\"'",
            "［＃ここから2字下げ］\n本文1\n本文2\n［＃ここで字下げ終わり］",
        ];
        for input in inputs {
            let a = render_to_string(input);
            let b = render_to_string(input);
            assert_eq!(a, b, "non-deterministic render for {input:?}");
        }
    }
}
