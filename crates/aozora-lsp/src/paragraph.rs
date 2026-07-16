//! Per-paragraph document model.
//!
//! A document is a list of paragraphs (separated by `\n\n` runs).
//! Each paragraph owns its own [`Rope`] text + tree-sitter [`Tree`] +
//! extracted [`GaijiSpan`] table + per-paragraph
//! [`crate::line_index::LineIndex`]. Editing a single paragraph never
//! touches another paragraph's memory: Rope mutations stay local, the
//! tree-sitter reparse covers only that paragraph's ~1-10 KB span,
//! and the snapshot rebuild for unchanged paragraphs is a single
//! `Arc` pointer bump.
//!
//! ## Structure
//!
//! - [`ParagraphBuffer`] — writers' single-paragraph state. Lives
//!   inside `DocBuffer::paragraphs`. Mutable text + tree.
//! - [`ParagraphSnapshot`] — readers' single-paragraph view.
//!   Immutable, `Arc`-shareable across snapshot generations.
//!   Carries pre-computed line index + gaiji spans (with
//!   doc-absolute byte offsets).
//! - [`paragraph_byte_ranges`] — the splitter: takes a `&Rope` and
//!   returns the byte ranges of each paragraph (`\n\n` boundaries).
//! - [`build_paragraph_snapshot`] — promote a `ParagraphBuffer` plus
//!   its byte-range-in-doc into a fully-populated immutable
//!   `ParagraphSnapshot`.
//!
//! ## Boundary policy
//!
//! Split at every `\n\n` run. The first newline goes to the LEFT
//! paragraph; the second newline starts the RIGHT paragraph. This
//! preserves byte-for-byte equality with the source when paragraphs
//! are concatenated. Empty rope yields one empty paragraph
//! `0..0`. Hard-cap at [`MAX_PARAGRAPH_BYTES`] so a never-blank-line
//! input still produces bounded segments.

use std::ops::Range;
use std::sync::Arc;

use ropey::Rope;
use tree_sitter::{InputEdit, Parser, Point, Tree};

use crate::gaiji_spans::{GaijiSpan, extract_gaiji_spans_from_tree};
use crate::line_index::LineIndex;

/// Hard cap on paragraph size. A never-blank-line input still
/// produces paragraphs no bigger than this; subsequent edits that
/// would grow a paragraph past the cap trigger a re-segmentation.
pub const MAX_PARAGRAPH_BYTES: usize = 64 * 1024;

/// Mutable single-paragraph state. Owned by [`crate::state::DocBuffer`].
#[derive(Debug)]
pub struct ParagraphBuffer {
    pub text: Rope,
    pub tree: Option<Tree>,
}

impl ParagraphBuffer {
    /// Build from owned text + a fresh tree (cold-start path).
    /// Caller is responsible for invoking the parser; we don't hold
    /// one because the parser lives on `DocBuffer` (one per doc).
    #[must_use]
    pub const fn new(text: Rope) -> Self {
        Self { text, tree: None }
    }

    /// Replace the tree with a freshly-parsed one against the
    /// current text. Centralised so callers pre/post-edit go
    /// through the same chunked-input callback.
    pub fn reparse(&mut self, parser: &mut Parser) {
        self.tree = parser.parse_with_options(&mut chunk_callback(&self.text), None, None);
    }

    /// Apply an `InputEdit` against the paragraph-local tree, then
    /// reparse via chunked input. The caller has already mutated
    /// `self.text`; the `InputEdit`'s byte offsets are
    /// **paragraph-local** (NOT doc-absolute).
    pub fn apply_edit(&mut self, parser: &mut Parser, edit: InputEdit) {
        let mut prior = self.tree.take();
        if let Some(tree) = prior.as_mut() {
            tree.edit(&edit);
        }
        self.tree =
            parser.parse_with_options(&mut chunk_callback(&self.text), prior.as_ref(), None);
    }

    /// Tree id (root node id) — stable identifier for "is this the
    /// same tree as before". Used by the snapshot rebuild to skip
    /// walks for unchanged paragraphs.
    #[must_use]
    pub fn tree_id(&self) -> Option<usize> {
        self.tree.as_ref().map(|t| t.root_node().id())
    }
}

/// Immutable per-paragraph snapshot.
///
/// Held inside `Arc` inside [`crate::state::DocSnapshot::paragraphs`].
/// Carries the data each LSP request handler needs to operate against
/// this paragraph without touching the writer side.
///
/// ## Coordinate frames
///
/// **Two coordinate frames live in this struct on purpose:**
///
/// - `byte_range`, `gaiji_spans[*].start_byte`, `gaiji_spans[*].end_byte`
///   are **document-absolute**. Handlers can hand these to
///   `LineIndex::position(text, byte)` against a doc-wide source
///   without knowing which paragraph contains them.
/// - `text`, `line_index`, `tree` reflect this paragraph's content
///   in **paragraph-local** coordinates (the tree-sitter parse was
///   fed only this paragraph's bytes; the line index counts only
///   this paragraph's `\n`s; the text starts at byte 0). Consumers
///   that mix in document-absolute positions translate via
///   `byte_range.start`.
///
/// Mixing the two is intentional — `gaiji_spans` is a pre-shifted
/// convenience for its consumer (the `aozora/gaijiSpans` request),
/// while keeping `tree` paragraph-local lets us share the
/// `Arc<Tree>` across snapshot generations untouched even when the
/// paragraph's absolute position shifted.
#[derive(Debug)]
pub struct ParagraphSnapshot {
    /// Document-absolute byte range this paragraph occupies in the
    /// containing snapshot.
    pub byte_range: Range<usize>,
    /// The paragraph's text, materialised once at snapshot build
    /// time so handlers can take `&str` slices without rope
    /// chunk-walking. **Paragraph-local** coordinates.
    pub text: Arc<str>,
    /// Per-paragraph line index. **Paragraph-local** line / column
    /// numbers. Document-level line numbers are recovered by adding
    /// the cumulative paragraph-newline counts that
    /// [`crate::semantic_tokens::semantic_tokens_full`] tracks while
    /// iterating paragraphs.
    pub line_index: Arc<LineIndex>,
    /// Gaiji spans within this paragraph, with **doc-absolute**
    /// `start_byte` / `end_byte` so handlers don't need to know
    /// which paragraph owns a span to translate it.
    pub gaiji_spans: Arc<[Arc<GaijiSpan>]>,
    /// Tree-sitter tree for this paragraph (cheap shallow `Arc`
    /// clone of the buffer-side tree). Available to wait-free
    /// readers that need structural information (semantic tokens,
    /// future linked editing). **Paragraph-local** byte coordinates;
    /// consumers shift via `byte_range.start`.
    pub tree: Option<Tree>,
    /// Snapshot of the tree's root id at build time. Re-using a
    /// `ParagraphSnapshot` across snapshot generations is keyed on
    /// `tree_id` matching the live paragraph's tree id (see
    /// `crate::state::DocSnapshot::rebuild_now`).
    pub tree_id: Option<usize>,
}

/// Build an immutable [`ParagraphSnapshot`] from a mutable paragraph
/// plus its document-absolute byte offset.
#[must_use]
pub(crate) fn build_paragraph_snapshot(
    paragraph: &ParagraphBuffer,
    byte_offset: usize,
) -> ParagraphSnapshot {
    let text_string = paragraph.text.to_string();
    let len = text_string.len();
    let line_index = LineIndex::new(&text_string);
    let text: Arc<str> = Arc::from(text_string);
    let gaiji_spans: Arc<[Arc<GaijiSpan>]> = paragraph.tree.as_ref().map_or_else(
        || Arc::from(Vec::new()),
        |t| {
            let local = extract_gaiji_spans_from_tree(t, &text);
            shift_to_absolute(&local, byte_offset)
        },
    );
    ParagraphSnapshot {
        byte_range: byte_offset..byte_offset.saturating_add(len),
        text,
        line_index: Arc::new(line_index),
        gaiji_spans,
        tree: paragraph.tree.clone(),
        tree_id: paragraph.tree_id(),
    }
}

/// Translate every span's byte offsets from paragraph-local to
/// doc-absolute by adding `offset` to `start_byte` / `end_byte`.
/// Pointer-bumps the description / mencode `Arc<str>` fields.
fn shift_to_absolute(spans: &[Arc<GaijiSpan>], offset: usize) -> Arc<[Arc<GaijiSpan>]> {
    let off = u32::try_from(offset).unwrap_or(u32::MAX);
    let out: Vec<Arc<GaijiSpan>> = spans
        .iter()
        .map(|s| {
            Arc::new(GaijiSpan {
                start_byte: s.start_byte.saturating_add(off),
                end_byte: s.end_byte.saturating_add(off),
                description: Arc::clone(&s.description),
                mencode: s.mencode.clone(),
            })
        })
        .collect();
    out.into()
}

impl ParagraphSnapshot {
    /// Produce an `Arc<ParagraphSnapshot>` placed at `new_start`.
    ///
    /// Operates on `&Arc<Self>` (not `&Self`) so the no-shift path
    /// can return `Arc::clone(prior)` — a single atomic increment
    /// with zero allocations. The shift path allocates a new
    /// `Arc<Self>` whose `text` / `line_index` / `tree` fields are
    /// `Arc::clone`d from the prior (sharing memory across snapshot
    /// generations) and whose gaiji-span list is rebuilt with
    /// shifted offsets.
    ///
    /// `DocSnapshot` rebuilds use this for paragraphs whose tree didn't
    /// change but whose absolute position shifted because a
    /// preceding paragraph grew or shrank.
    #[must_use]
    pub fn shifted_to(prior: &Arc<Self>, new_start: usize) -> Arc<Self> {
        if prior.byte_range.start == new_start {
            // No shift — pure pointer bump, no work at all.
            return Arc::clone(prior);
        }
        let len = prior.byte_range.len();
        let prior_start = prior.byte_range.start;
        let new_spans = shift_existing_spans(&prior.gaiji_spans, prior_start, new_start);
        Arc::new(Self {
            byte_range: new_start..new_start.saturating_add(len),
            text: Arc::clone(&prior.text),
            line_index: Arc::clone(&prior.line_index),
            gaiji_spans: new_spans,
            tree: prior.tree.clone(),
            tree_id: prior.tree_id,
        })
    }
}

fn shift_existing_spans(
    spans: &[Arc<GaijiSpan>],
    prior_start: usize,
    new_start: usize,
) -> Arc<[Arc<GaijiSpan>]> {
    if prior_start == new_start {
        // Same position — every span's bytes stay; just clone the
        // outer Arc<[...]> fat pointer and let the inner Arc<GaijiSpan>
        // entries pointer-bump via `Vec::from(slice).into()`.
        return spans.to_vec().into();
    }
    let new_signed = i64::try_from(new_start).unwrap_or(i64::MAX);
    let prior_signed = i64::try_from(prior_start).unwrap_or(i64::MAX);
    let delta_signed = new_signed - prior_signed;
    let out: Vec<Arc<GaijiSpan>> = spans
        .iter()
        .map(|s| {
            let new_start_byte = i64::from(s.start_byte) + delta_signed;
            let new_end_byte = i64::from(s.end_byte) + delta_signed;
            Arc::new(GaijiSpan {
                start_byte: u32::try_from(new_start_byte).unwrap_or(u32::MAX),
                end_byte: u32::try_from(new_end_byte).unwrap_or(u32::MAX),
                description: Arc::clone(&s.description),
                mencode: s.mencode.clone(),
            })
        })
        .collect();
    out.into()
}

/// Compute paragraph byte ranges over `rope` per the
/// "split at `\n\n` runs" policy. Empty rope yields `[0..0]`.
/// Worst case (no blank lines for a long stretch) hard-splits at
/// the [`MAX_PARAGRAPH_BYTES`] cap.
///
/// All emitted boundaries land on UTF-8 char boundaries, even when
/// the cap fallback fires inside a multi-byte codepoint — `\n\n`
/// boundaries are ASCII-aligned by construction, and the cap path
/// snaps forward to the next char boundary so downstream
/// `Rope::byte_slice` calls never panic on mid-codepoint splits.
#[must_use]
pub(crate) fn paragraph_byte_ranges(rope: &Rope) -> Vec<Range<usize>> {
    let total = rope.len_bytes();
    let mut out: Vec<Range<usize>> = Vec::new();
    if total == 0 {
        out.push(0..0);
        return out;
    }
    let bytes: Vec<u8> = rope.bytes().collect();
    let mut start = 0usize;
    while start < total {
        let mut end = start;
        let mut soft_end: Option<usize> = None;
        while end < total {
            // Cap fallback: prefer a soft \n boundary that sits
            // inside the cap window; otherwise hard-split and snap
            // forward to the next char boundary so the resulting
            // range is safe to slice through `Rope::byte_slice`.
            if end - start >= MAX_PARAGRAPH_BYTES {
                end = soft_end.unwrap_or(end).max(start + 1);
                while end < total && !is_byte_char_boundary(&bytes, end) {
                    end += 1;
                }
                break;
            }
            // \n\n boundary: include the first \n in this segment;
            // the next iteration starts at the second \n.
            if bytes[end] == b'\n' && end + 1 < total && bytes[end + 1] == b'\n' {
                end += 1;
                break;
            }
            // Single \n is a soft boundary candidate for the cap path.
            if bytes[end] == b'\n' {
                soft_end = Some(end + 1);
            }
            end += 1;
        }
        out.push(start..end.min(total));
        start = end.min(total);
    }
    if out.is_empty() {
        out.push(0..total);
    }
    out
}

/// Return `true` when `bytes[idx]` does NOT continue a UTF-8
/// multi-byte sequence (i.e. it's the start of a codepoint, or it
/// equals `bytes.len()`). Continuation bytes have the bit pattern
/// `10xxxxxx`.
fn is_byte_char_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx >= bytes.len() {
        return idx == bytes.len();
    }
    bytes[idx] & 0b1100_0000 != 0b1000_0000
}

/// Tree-sitter chunked-input callback over a paragraph's local
/// `Rope`. Used by both the cold-start and incremental parse
/// paths.
pub(crate) fn chunk_callback<'r>(rope: &'r Rope) -> impl FnMut(usize, Point) -> &'r [u8] {
    let len = rope.len_bytes();
    move |byte_idx, _pos| -> &'r [u8] {
        if byte_idx >= len {
            return &[];
        }
        let (chunk, chunk_byte_idx, _, _) = rope.chunk_at_byte(byte_idx);
        let local = byte_idx - chunk_byte_idx;
        &chunk.as_bytes()[local..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from(s)
    }

    #[test]
    fn paragraph_byte_ranges_empty_yields_one_empty_range() {
        let r = rope("");
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges, vec![0..0]);
    }

    #[test]
    fn paragraph_byte_ranges_single_paragraph_no_split() {
        let r = rope("ただの一段落だけ");
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 0..r.len_bytes());
    }

    #[test]
    fn paragraph_byte_ranges_splits_on_blank_line() {
        let s = "段落1\n\n段落2";
        let r = rope(s);
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges.len(), 2);
        let blank_at = s.find("\n\n").unwrap();
        assert_eq!(ranges[0].end, blank_at + 1);
        assert_eq!(ranges[1].start, blank_at + 1);
        assert_eq!(ranges[1].end, s.len());
    }

    #[test]
    fn paragraph_byte_ranges_cover_full_rope_with_no_gaps() {
        let s = "一\n\n二\n\n三";
        let r = rope(s);
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, s.len());
        for w in ranges.windows(2) {
            assert_eq!(w[0].end, w[1].start, "no gap or overlap");
        }
    }

    #[test]
    fn build_snapshot_extracts_local_gaiji_at_doc_absolute_offset() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_aozora::LANGUAGE.into())
            .unwrap();
        let mut p = ParagraphBuffer::new(rope("※［＃「a」、X］"));
        p.reparse(&mut parser);
        let snap = build_paragraph_snapshot(&p, 1000);
        assert_eq!(snap.byte_range.start, 1000);
        assert_eq!(snap.gaiji_spans.len(), 1);
        let span = &snap.gaiji_spans[0];
        assert_eq!(span.start_byte, 1000); // local 0 + 1000 offset
        assert_eq!(&*span.description, "a");
    }

    /// Regression: a multi-byte body with no `\n\n` boundary forced
    /// the cap-fallback branch to split at exactly
    /// `start + MAX_PARAGRAPH_BYTES`. `MAX_PARAGRAPH_BYTES` is 65536
    /// and `あ` is 3 bytes, so the cap landed inside an `あ` and the
    /// downstream `Rope::byte_slice(range)` panicked.
    /// Pin: cap-fallback always emits ranges aligned to char boundaries.
    #[test]
    fn paragraph_byte_ranges_cap_fallback_aligns_to_char_boundary() {
        // 30000 × 3 bytes = 90000 > MAX_PARAGRAPH_BYTES, no \n at all.
        let s: String = "あ".repeat(30000);
        let r = rope(&s);
        let ranges = paragraph_byte_ranges(&r);
        for range in &ranges {
            assert!(
                s.is_char_boundary(range.start),
                "range.start = {} sits inside a multi-byte codepoint",
                range.start,
            );
            assert!(
                s.is_char_boundary(range.end),
                "range.end = {} sits inside a multi-byte codepoint",
                range.end,
            );
        }
        // And every range must be slice-able through the rope without
        // panicking — that's the actual end-to-end invariant.
        for range in &ranges {
            drop(r.byte_slice(range.clone()).to_string());
        }
    }

    /// Regression: even when the cap-fallback fires, the resulting
    /// ranges must still cover the full source with no gaps. Without
    /// the boundary-snap fix, the cap landed mid-codepoint and the
    /// downstream rope splice silently dropped bytes (or panicked).
    #[test]
    fn paragraph_byte_ranges_cap_fallback_covers_full_input() {
        let s: String = "あ".repeat(30000);
        let r = rope(&s);
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, s.len());
        for w in ranges.windows(2) {
            assert_eq!(w[0].end, w[1].start, "no gap or overlap");
        }
    }

    /// End-to-end pin for the same bug: constructing a `DocBuffer`
    /// from a giant multi-byte stream went through `Rope::byte_slice`
    /// against ranges that could land mid-codepoint, and panicked.
    /// Documents this scale exist in the wild — `tsumi-to-batsu-x100`
    /// is one of the workspace's own benchmark fixtures.
    #[test]
    fn doc_state_handles_giant_multibyte_paragraph_without_panic() {
        use crate::state::OpenDocument;
        // 30 000 あ's = ~90 KB without any newline — single-paragraph
        // stress for `DocBuffer::new`'s `paragraph_from_rope_slice`.
        let s: String = "あ".repeat(30_000);
        let state = OpenDocument::new(s.clone());
        // Round-trip text equality is the strongest possible check
        // that no bytes were dropped or misaligned.
        assert_eq!(&**state.snapshot().doc_text(), &s);
    }

    /// `\n\n\n` (triple newline) splits into three paragraphs because
    /// each `\n\n` boundary forms its own split. Earlier code
    /// versions with off-by-one boundary logic produced two
    /// paragraphs, dropping a `\n`. Pin the count explicitly.
    #[test]
    fn paragraph_byte_ranges_triple_newline_yields_three_paragraphs() {
        let r = rope("\n\n\n");
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);
    }

    /// Single `\n` is one paragraph (it never forms a `\n\n`
    /// boundary). The trailing `\n` belongs to the same paragraph
    /// that contains everything before it.
    #[test]
    fn paragraph_byte_ranges_single_trailing_newline_is_one_paragraph() {
        let r = rope("abc\n");
        assert_eq!(paragraph_byte_ranges(&r), vec![0..4]);
    }

    /// Document ending exactly with `\n\n` splits into the prefix
    /// (with first `\n`) and the trailing single-`\n` paragraph.
    /// Earlier on we accidentally collapsed the trailing run.
    #[test]
    fn paragraph_byte_ranges_trailing_blank_run_keeps_blank_paragraph() {
        let r = rope("X\n\n");
        let ranges = paragraph_byte_ranges(&r);
        // p0 = "X\n" (bytes 0..2), p1 = "\n" (bytes 2..3)
        assert_eq!(ranges, vec![0..2, 2..3]);
    }

    /// Even when the cap-fallback fires multiple times, each
    /// successive range still stays char-boundary-aligned and
    /// non-overlapping. Stress-test with ~3× the cap so we exercise
    /// at least three cap splits.
    #[test]
    fn paragraph_byte_ranges_repeated_cap_fallback_aligns_each_split() {
        // 80 000 `あ` * 3 bytes = 240 000 bytes ≈ 4× the 64 KB cap.
        let s: String = "あ".repeat(80_000);
        let r = rope(&s);
        let ranges = paragraph_byte_ranges(&r);
        assert!(ranges.len() >= 3, "expected ≥3 cap splits: {ranges:?}");
        for range in &ranges {
            assert!(s.is_char_boundary(range.start));
            assert!(s.is_char_boundary(range.end));
            // Slicing through the rope must succeed without panic.
            drop(r.byte_slice(range.clone()).to_string());
        }
        // Coverage: ranges must cover [0, total) with no gap and no
        // overlap, regardless of how many cap splits happened.
        for w in ranges.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, s.len());
    }

    /// `MAX_PARAGRAPH_BYTES` must be exactly 64 KiB. Pins the `64 * 1024`
    /// product so a `*`→`+` mutation (which would yield 1088) is caught.
    #[test]
    fn max_paragraph_bytes_is_64_kib() {
        assert_eq!(MAX_PARAGRAPH_BYTES, 65_536);
    }

    fn one_span(start_byte: u32, end_byte: u32) -> Vec<Arc<GaijiSpan>> {
        vec![Arc::new(GaijiSpan {
            start_byte,
            end_byte,
            description: Arc::from("x"),
            mencode: None,
        })]
    }

    /// `shift_existing_spans` applies the exact signed delta
    /// `new_start - prior_start` to every span's `start_byte` /
    /// `end_byte`. Concrete numbers pin the arithmetic:
    /// prior=10, new=50 ⇒ delta=+40, so span 100..200 ⇒ 140..240.
    ///
    /// Kills, via the exact offsets:
    /// - `==`→`!=` guard (would early-return the *unshifted* spans →100/200),
    /// - `-`→`+` / `-`→`/` in the delta (→60 or 5 ⇒ 160 or 105),
    /// - `+`→`-` / `+`→`*` on `start_byte` (→60 or 4000),
    /// - `+`→`-` / `+`→`*` on `end_byte` (→160 or 8000).
    #[test]
    fn shift_existing_spans_applies_exact_signed_delta() {
        let spans = one_span(100, 200);
        let out = shift_existing_spans(&spans, 10, 50);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_byte, 140, "100 + (50 - 10)");
        assert_eq!(out[0].end_byte, 240, "200 + (50 - 10)");
    }

    /// Same-position call is an identity on the byte offsets (documents
    /// the `prior_start == new_start` true-side of the early-return
    /// guard). Delta would be 0 either way, so this documents rather
    /// than kills the `==`→`!=` mutant — the kill lives in
    /// `shift_existing_spans_applies_exact_signed_delta`.
    #[test]
    fn shift_existing_spans_same_position_keeps_offsets() {
        let spans = one_span(100, 200);
        let out = shift_existing_spans(&spans, 42, 42);
        assert_eq!(out[0].start_byte, 100);
        assert_eq!(out[0].end_byte, 200);
    }

    /// Cap fallback: 30 000 × 3-byte `あ` with no newline. The cap fires
    /// at byte 65 536, which is the 2nd byte of an `あ`
    /// (65 536 % 3 == 1), so the char-boundary snap loop must move
    /// FORWARD to 65 538. A `+=`→`-=` mutation would step backward to
    /// 65 535 (also a boundary, so alignment-only assertions miss it) —
    /// this pins the exact forward offset.
    #[test]
    fn paragraph_byte_ranges_cap_snap_advances_forward_to_65538() {
        let s: String = "あ".repeat(30_000);
        let r = rope(&s);
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges[0].end, 65_538);
    }

    /// Cap fallback with a single `\n` soft boundary at byte 100
    /// (followed by `B`, so it is NOT a `\n\n` run). A long newline-free
    /// tail pushes the segment past the 64 KiB cap; the fallback must
    /// retreat to the recorded soft boundary at byte 101 (one past the
    /// `\n`).
    ///
    /// Pins byte 101, which kills:
    /// - `==`→`!=` at the single-`\n` test (`soft_end` would track every
    ///   non-newline byte and land at the cap edge, 65 536),
    /// - `+`→`-` in `Some(end + 1)` (→99),
    /// - `+`→`*` in `Some(end + 1)` (→100).
    #[test]
    fn paragraph_byte_ranges_cap_retreats_to_single_newline_soft_boundary() {
        let s = format!("{}\n{}", "A".repeat(100), "B".repeat(70_000));
        let r = rope(&s);
        let ranges = paragraph_byte_ranges(&r);
        assert_eq!(ranges[0].end, 101);
    }

    /// `is_byte_char_boundary` at the one-past-the-end index returns
    /// `true` (the `idx == bytes.len()` arm), and any index strictly
    /// past the end returns `false`. Kills the `==`→`!=` mutation on
    /// that arm, which would flip both.
    #[test]
    fn is_byte_char_boundary_at_and_past_len() {
        let bytes = b"ab";
        assert!(is_byte_char_boundary(bytes, 2), "idx == len is a boundary");
        assert!(!is_byte_char_boundary(bytes, 3), "idx > len is not");
        assert!(!is_byte_char_boundary(bytes, 100));
    }

    /// Interior contract of `is_byte_char_boundary`: leading byte and
    /// one-past-the-end are boundaries; continuation bytes are not.
    #[test]
    fn is_byte_char_boundary_interior_bytes() {
        let bytes = "あ".as_bytes(); // E3 81 82
        assert!(is_byte_char_boundary(bytes, 0), "leading byte");
        assert!(!is_byte_char_boundary(bytes, 1), "continuation byte");
        assert!(!is_byte_char_boundary(bytes, 2), "continuation byte");
        assert!(is_byte_char_boundary(bytes, 3), "idx == len");
    }
}
