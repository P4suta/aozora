//! Borrowed-AST HTML rendering.
//!
//! Mirrors `aozora_parser::html::render_from_artifacts_to_writer` but
//! consumes a [`BorrowedLexOutput`] (Plan B.2 output) directly. The
//! algorithm and emitted bytes are identical; only the input shape and
//! per-node renderer change.
//!
//! # Algorithm
//!
//! Single forward `match_indices` sweep over the lexer's normalized
//! text, capturing every PUA sentinel + `\n` in O(n). Plain text
//! between matches flows through the bulk-copy escape pass; sentinels
//! dispatch into [`crate::render_node::render`] via the borrowed
//! registry's [`EytzingerMap::get`] lookup.
//!
//! Block structure mirrors the legacy walker: a two-state machine
//! [`RenderState::ensure_in_paragraph`] / [`RenderState::close_paragraph`]
//! emits `<p>` / `</p>` symmetrically; standalone block nodes (and
//! container open/close events) flush the current paragraph first.

use core::fmt;

use aozora_lex::BorrowedLexOutput;
use aozora_syntax::Container;
use aozora_syntax::borrowed::AozoraNode;
use memchr::{memchr_iter, memchr3_iter};

use crate::render_node;

/// First UTF-8 byte of every PUA sentinel (E001..E004). Used by
/// [`render_into`] to fast-scan candidate positions; the next two
/// bytes (0x80 + variant) are validated before the position is
/// treated as a sentinel.
const SENTINEL_LEAD_BYTE: u8 = 0xEE;
/// Second UTF-8 byte shared by every PUA sentinel.
const SENTINEL_MID_BYTE: u8 = 0x80;
/// Third UTF-8 byte of [`INLINE_SENTINEL`] (U+E001).
const INLINE_SENTINEL_TAIL: u8 = 0x81;
/// Third UTF-8 byte of [`BLOCK_LEAF_SENTINEL`] (U+E002).
const BLOCK_LEAF_SENTINEL_TAIL: u8 = 0x82;
/// Third UTF-8 byte of [`BLOCK_OPEN_SENTINEL`] (U+E003).
const BLOCK_OPEN_SENTINEL_TAIL: u8 = 0x83;
/// Third UTF-8 byte of [`BLOCK_CLOSE_SENTINEL`] (U+E004).
const BLOCK_CLOSE_SENTINEL_TAIL: u8 = 0x84;

/// Render a `BorrowedLexOutput` into a fresh `String`.
///
/// Allocates roughly `2 × normalized.len()` upfront — the same growth
/// strategy as `aozora_parser::html::render_to_string`. For streaming
/// consumers prefer [`render_into`] to avoid the intermediate `String`.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`fmt::Write`] sink. The internal `expect` covers the trivially
/// unreachable case.
#[must_use]
pub fn render_to_string(out: &BorrowedLexOutput<'_>) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_into(out, &mut s).expect("writing to String never fails");
    s
}

/// Render a `BorrowedLexOutput` into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited
/// from the lexer's `Span` width contract; in practice unreachable
/// (Phase 0 sanitize already gates on this bound).
pub fn render_into<W: fmt::Write>(out: &BorrowedLexOutput<'_>, writer: &mut W) -> fmt::Result {
    let normalized = out.normalized;
    let registry = &out.registry;
    let mut state = RenderState::default();

    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    // Byte-level structural scan. The legacy implementation used
    // `match_indices(is_structural_char)`, which dispatches the
    // predicate through `Chars::next` → `next_code_point` for every
    // codepoint of the document. For Japanese-text-heavy corpora
    // (3-byte UTF-8 per char) that is ~67 k char-iter calls per
    // 200 KB doc, almost all returning false. Samply attributed the
    // bulk of `render_to_string` time to this scan and its
    // `MatchIndicesInternal::next` machinery.
    //
    // Replacement: every PUA sentinel (E001..E004) shares the
    // 2-byte UTF-8 prefix `0xEE 0x80`. The other structural
    // character is `\n` (0x0A). One `memchr2` finds candidate
    // positions at memory-bandwidth speed (Two-Way + AVX2 SIMD via
    // the `memchr` crate); each candidate is then validated with two
    // byte loads to confirm the full sentinel codepoint, falling
    // through cleanly for the rare PUA-collision case (Phase 0's
    // `scan_for_sentinel_collisions` records a diagnostic but does
    // not delete colliding bytes — they must render as plain text).
    let iter = memchr::memchr2_iter(SENTINEL_LEAD_BYTE, b'\n', bytes);
    for cand_pos in iter {
        let (kind, len) = match bytes[cand_pos] {
            b'\n' => (Structural::Newline, 1),
            SENTINEL_LEAD_BYTE => {
                if cand_pos + 2 < bytes.len()
                    && bytes[cand_pos + 1] == SENTINEL_MID_BYTE
                    && let Some(k) = sentinel_for_tail_byte(bytes[cand_pos + 2])
                {
                    (k, 3)
                } else {
                    // PUA collision in source: bytes flow through as
                    // plain. Skip; the next-iteration's pending-plain
                    // emission picks them up in the chunk between
                    // `cursor` and the next match.
                    continue;
                }
            }
            // memchr2 only matches the two needles above.
            _ => unreachable!("memchr2 hit non-needle byte"),
        };

        if cursor < cand_pos {
            state.ensure_in_paragraph(writer)?;
            escape_text_chunk(&normalized[cursor..cand_pos], writer)?;
        }
        let byte_pos = u32::try_from(cand_pos).expect("normalized fits u32 per Phase 0 cap");

        match kind {
            Structural::Inline => {
                if let Some(&node) = registry.inline.get(&byte_pos) {
                    state.ensure_in_paragraph(writer)?;
                    render_node::render(node, true, writer)?;
                }
            }
            Structural::BlockLeaf => {
                if let Some(&node) = registry.block_leaf.get(&byte_pos) {
                    state.before_block_emit(writer)?;
                    render_node::render(node, true, writer)?;
                    state.after_block_emit();
                }
            }
            Structural::BlockOpen => {
                if let Some(&kind) = registry.block_open.get(&byte_pos) {
                    state.before_block_emit(writer)?;
                    let node = AozoraNode::Container(Container { kind });
                    render_node::render(node, true, writer)?;
                    state.after_block_emit();
                }
            }
            Structural::BlockClose => {
                if let Some(&kind) = registry.block_close.get(&byte_pos) {
                    state.before_block_emit(writer)?;
                    let node = AozoraNode::Container(Container { kind });
                    render_node::render(node, false, writer)?;
                    state.after_block_emit();
                }
            }
            Structural::Newline => match bytes.get(cand_pos + 1) {
                Some(&b'\n') => state.close_paragraph(writer)?,
                Some(_) if state.in_paragraph => writer.write_str("<br />\n")?,
                Some(_) | None => {}
            },
        }
        cursor = cand_pos + len;
    }

    if cursor < normalized.len() {
        state.ensure_in_paragraph(writer)?;
        escape_text_chunk(&normalized[cursor..], writer)?;
    }
    state.close_paragraph(writer)?;
    Ok(())
}

/// Structural-character classification for [`render_into`]'s byte-level
/// scanner. `Newline` consumes 1 byte; the four sentinel variants each
/// consume 3 (`0xEE 0x80 0x8X`).
#[derive(Clone, Copy)]
enum Structural {
    Inline,
    BlockLeaf,
    BlockOpen,
    BlockClose,
    Newline,
}

/// Decode the third UTF-8 byte of a PUA-sentinel candidate. Returns
/// `Some` only for the four well-known sentinels; any other byte is
/// a collision (plain text that happens to share the prefix) and
/// rejects this position.
#[inline]
fn sentinel_for_tail_byte(b: u8) -> Option<Structural> {
    match b {
        INLINE_SENTINEL_TAIL => Some(Structural::Inline),
        BLOCK_LEAF_SENTINEL_TAIL => Some(Structural::BlockLeaf),
        BLOCK_OPEN_SENTINEL_TAIL => Some(Structural::BlockOpen),
        BLOCK_CLOSE_SENTINEL_TAIL => Some(Structural::BlockClose),
        _ => None,
    }
}

/// Block-level walker state — mirrors the owned renderer's state
/// machine 1:1 so the emitted bytes match. See
/// `aozora_parser::html::RenderState` for the design rationale.
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

/// HTML-escape a plain-text chunk (the bytes between two structural
/// matches in [`render_into`]).
///
/// The five HTML-unsafe ASCII characters (`< > & " '`) are rare in
/// Japanese-text-heavy corpora — most chunks contain none. Two
/// `memchr` passes (`memchr3` for `< > &` then `memchr` for `"`)
/// fast-skip those clean chunks at memory-bandwidth speed; only when
/// at least one needle hits do we fall through to a byte loop that
/// merges the candidate positions and emits the escapes in document
/// order. Single-quote `'` (0x27) is folded into the same byte loop
/// because it has no `memchr_iter` partner — three needle scans are
/// enough to cover the rare cases without paying for a 5-needle
/// general scan, which `memchr` doesn't expose.
fn escape_text_chunk<W: fmt::Write>(chunk: &str, out: &mut W) -> fmt::Result {
    let bytes = chunk.as_bytes();

    // Fast-reject: no HTML-unsafe byte → bulk write the whole chunk.
    let mut iter_lt_gt_amp = memchr3_iter(b'<', b'>', b'&', bytes);
    let first_lt_gt_amp = iter_lt_gt_amp.next();
    let mut iter_quote = memchr_iter(b'"', bytes);
    let first_quote = iter_quote.next();
    let mut iter_apos = memchr_iter(b'\'', bytes);
    let first_apos = iter_apos.next();

    if first_lt_gt_amp.is_none() && first_quote.is_none() && first_apos.is_none() {
        return out.write_str(chunk);
    }

    // Slow path: merge the three iterators in document order.
    // Re-derive the iterators so we can use the post-`first_*`
    // peekable state cleanly. Cost is one duplicate memchr scan;
    // negligible because this branch only runs when the chunk
    // actually has unsafe bytes (rare on Japanese prose).
    let mut cursor = 0usize;
    let mut next_lt_gt_amp = first_lt_gt_amp;
    let mut next_quote = first_quote;
    let mut next_apos = first_apos;

    loop {
        // Pick the smallest of the three pending positions.
        let pos = [next_lt_gt_amp, next_quote, next_apos]
            .into_iter()
            .flatten()
            .min();
        let Some(pos) = pos else { break };

        if cursor < pos {
            out.write_str(&chunk[cursor..pos])?;
        }
        let entity = match bytes[pos] {
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'&' => "&amp;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            // The match is exhaustive over the bytes the three
            // memchr scans yield — if we ever hit this branch,
            // either memchr returned a position outside its needle
            // set (impossible) or an iterator was advanced
            // incorrectly. Either way an unreachable! is the only
            // honest reaction.
            _ => unreachable!("escape iterator yielded non-needle byte"),
        };
        out.write_str(entity)?;
        cursor = pos + 1;

        // Advance whichever iterator just produced this position.
        if next_lt_gt_amp == Some(pos) {
            next_lt_gt_amp = iter_lt_gt_amp.next();
        }
        if next_quote == Some(pos) {
            next_quote = iter_quote.next();
        }
        if next_apos == Some(pos) {
            next_apos = iter_apos.next();
        }
    }
    out.write_str(&chunk[cursor..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;
    use pretty_assertions::assert_eq;

    fn render(src: &str) -> String {
        let arena = Arena::new();
        let out = aozora_lex::lex_into_arena(src, &arena);
        render_to_string(&out)
    }

    #[test]
    fn plain_paragraph_wraps_in_p() {
        assert_eq!(render("Hello."), "<p>Hello.</p>\n");
    }

    #[test]
    fn ruby_emits_semantic_form() {
        let html = render("｜青梅《おうめ》");
        assert!(html.contains("<ruby>青梅"), "missing ruby tag: {html}");
        assert!(html.contains("<rt>おうめ"), "missing rt tag: {html}");
    }

    #[test]
    fn page_break_inside_text_emits_div() {
        let html = render("前\n\n［＃改ページ］\n\n後");
        assert!(html.contains(r#"<div class="afm-page-break"></div>"#));
        assert!(!html.contains("［＃"), "［＃ leaked: {html}");
    }

    #[test]
    fn paired_container_open_close_renders_div_pair() {
        let html = render("［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
        assert!(html.contains("afm-container-indent afm-container-indent-2"));
        assert!(html.contains("</div>"));
    }

    #[test]
    fn newline_inside_paragraph_emits_br() {
        let html = render("a\nb");
        assert!(html.contains("a<br />\nb"));
    }

    #[test]
    fn double_newline_closes_paragraph() {
        let html = render("a\n\nb");
        assert!(html.contains("<p>a</p>\n"));
        assert!(html.contains("<p>b</p>\n"));
    }

    #[test]
    fn html_unsafe_chars_in_plain_text_are_escaped() {
        let html = render("a<b>&\"'");
        assert!(html.contains("a&lt;b&gt;&amp;&quot;&#39;"));
    }

    #[test]
    fn empty_input_emits_empty_string() {
        assert_eq!(render(""), "");
    }
}
