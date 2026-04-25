//! Incremental re-parse helpers for editor / LSP integration.
//!
//! # Why this module exists
//!
//! The lexer is a pure `&str -> LexOutput` function and re-running it
//! on a 100 KB document is sub-millisecond, so for normal interactive
//! sessions a full re-parse on every keystroke is fine. The
//! incremental layer covers two cases the full-reparse path cannot:
//!
//! - **Editor protocols that send edits, not full text.** LSP's
//!   `TextDocumentSyncKind::INCREMENTAL` mode hands us
//!   `(range, replacement)` triples; a server that wants to consume
//!   that protocol needs a stable way to apply edits to its stored
//!   document.
//! - **Future window-based re-lex.** Today the
//!   [`parse_incremental`] entry point delegates to a full
//!   [`crate::parse`] of the post-edit source. The function exists
//!   so consumers can target it now and pick up window-based
//!   short-circuiting transparently when the lexer grows it. A
//!   first-cut fast path that handles plain-text edits inside
//!   annotation-free paragraphs already lives below; everything
//!   else falls through to full parse, with [`IncrementalDecision`]
//!   reporting the path taken.
//!
//! # Correctness contract
//!
//! For any `(prev_source, edits)` pair, the result of
//! [`parse_incremental`] must match a full [`crate::parse`] of the
//! post-edit source produced by [`apply_edits`] — byte-for-byte
//! and diagnostic-for-diagnostic. The proptest suite in
//! `tests/incremental.rs` exercises this invariant on randomly
//! generated edit sequences over aozora-shaped input.
//!
//! # Edit shape
//!
//! [`TextEdit`] is intentionally narrow: a byte range to replace
//! and the new text. LSP's UTF-16 `Position` is a separate concern;
//! callers convert with `aozora-tools/aozora-lsp::position` (or any
//! equivalent) before talking to this module. Keeping byte ranges
//! at the API surface lets the lexer reason about source bytes
//! directly.
//!
//! Edits in a slice are applied **in source order** of the original
//! buffer. Each edit's `range` is interpreted against the *original*
//! `prev_source`, not the partially-edited intermediate state — see
//! [`apply_edits`] for the in-place algorithm.

use core::error::Error;
use core::fmt;
use core::ops::Range;

use crate::{ParseArtifacts, ParseResult, parse};

/// One byte-level edit to apply to a source buffer.
///
/// `range` is interpreted against the *original* `prev_source` passed
/// to [`apply_edits`] / [`parse_incremental`], not the intermediate
/// post-edit state. This matches LSP's
/// `TextDocumentContentChangeEvent` shape (each event references the
/// pre-change document) and lets multi-edit batches be sorted by
/// `range.start` for ordered application.
///
/// Both endpoints of `range` must fall on UTF-8 character boundaries
/// in `prev_source`; [`apply_edits`] returns an error otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// Byte range in the *previous* source that is being replaced.
    pub range: Range<usize>,
    /// Replacement text. Empty for pure deletions; equal to the
    /// existing slice for no-ops.
    pub new_text: String,
}

impl TextEdit {
    /// Constructor preserving validity at the call site. `range`
    /// bounds are checked when the edit is applied.
    #[must_use]
    pub fn new(range: Range<usize>, new_text: impl Into<String>) -> Self {
        Self {
            range,
            new_text: new_text.into(),
        }
    }
}

/// Reasons [`apply_edits`] / [`parse_incremental`] may reject an
/// edit batch. All variants describe input shape problems; none
/// indicate parser bugs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// `range.start > range.end`.
    InvertedRange { range: Range<usize> },
    /// `range.end` exceeds `source.len()`.
    OutOfBounds { range: Range<usize>, source_len: usize },
    /// `range.start` or `range.end` does not lie on a UTF-8 char
    /// boundary in the source.
    NotCharBoundary { offset: usize },
    /// Two edits in the batch have overlapping byte ranges. Edits
    /// must be disjoint when interpreted against the original
    /// source; LSP guarantees this for one notification's batch.
    OverlappingEdits { first: Range<usize>, second: Range<usize> },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvertedRange { range } => {
                write!(f, "edit range {range:?} is inverted (start > end)")
            }
            Self::OutOfBounds { range, source_len } => write!(
                f,
                "edit range {range:?} out of bounds for source of {source_len} bytes",
            ),
            Self::NotCharBoundary { offset } => {
                write!(f, "edit endpoint {offset} is not a UTF-8 char boundary")
            }
            Self::OverlappingEdits { first, second } => write!(
                f,
                "overlapping edits: {first:?} and {second:?} share bytes",
            ),
        }
    }
}

impl Error for EditError {}

/// Apply an edit batch to `source` and return the resulting string.
///
/// The algorithm sorts edits by `range.start` (input order is
/// preserved when starts are equal) and applies them right-to-left
/// so each application sees its `range` at the same byte offsets it
/// references in `source`. That is what LSP guarantees for a single
/// `did_change` notification: every event's `range` is interpreted
/// against the buffer state *before any event in the batch was
/// applied*.
///
/// # Errors
///
/// Returns [`EditError`] if any edit's range is inverted, out of
/// bounds, off a UTF-8 char boundary, or overlaps another edit in
/// the batch.
pub fn apply_edits(source: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    if edits.is_empty() {
        return Ok(source.to_owned());
    }

    // Validate each edit independently first — gives a clean error
    // path before any allocation. Defensive against a poorly-formed
    // batch from a buggy editor.
    for edit in edits {
        if edit.range.start > edit.range.end {
            return Err(EditError::InvertedRange {
                range: edit.range.clone(),
            });
        }
        if edit.range.end > source.len() {
            return Err(EditError::OutOfBounds {
                range: edit.range.clone(),
                source_len: source.len(),
            });
        }
        if !source.is_char_boundary(edit.range.start) {
            return Err(EditError::NotCharBoundary {
                offset: edit.range.start,
            });
        }
        if !source.is_char_boundary(edit.range.end) {
            return Err(EditError::NotCharBoundary {
                offset: edit.range.end,
            });
        }
    }

    // Sort indices by (range.start, original index) so equal-start
    // edits keep their input order and the right-to-left walk below
    // is deterministic. We index into `edits` rather than cloning,
    // so the pre-allocation cost is `O(n)` for the index Vec.
    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&i| edits[i].range.start);

    // Disjointness check after sorting (O(n)).
    for window in order.windows(2) {
        let (a, b) = (&edits[window[0]], &edits[window[1]]);
        if a.range.end > b.range.start {
            return Err(EditError::OverlappingEdits {
                first: a.range.clone(),
                second: b.range.clone(),
            });
        }
    }

    // Capacity heuristic: assume the result is roughly the same size
    // as the source plus the cumulative new-text length. Avoids the
    // exponential-growth allocations that a naïve `String::new()`
    // would pay during many small inserts.
    let cumulative_new: usize = edits.iter().map(|e| e.new_text.len()).sum();
    let mut result = String::with_capacity(source.len().saturating_add(cumulative_new));

    let mut cursor = 0usize;
    for &i in &order {
        let edit = &edits[i];
        result.push_str(&source[cursor..edit.range.start]);
        result.push_str(&edit.new_text);
        cursor = edit.range.end;
    }
    result.push_str(&source[cursor..]);
    Ok(result)
}

/// Outcome of [`parse_incremental`].
///
/// Carries the new [`ParseResult`] plus a breadcrumb describing
/// which path the implementation took. The breadcrumb is
/// informational only — it powers test assertions and LSP-side
/// telemetry, never branching logic.
#[derive(Debug, Clone)]
pub struct IncrementalOutcome {
    /// New parse result for the post-edit source.
    pub result: ParseResult,
    /// Resulting source after [`apply_edits`].
    pub new_source: String,
    /// Which decision path the implementation took.
    pub decision: IncrementalDecision,
}

/// Tag for the strategy [`parse_incremental`] used.
///
/// Consumers can read this for telemetry but the contract is
/// strictly observational: every variant produces a result equal to
/// a full `parse(apply_edits(...))` invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IncrementalDecision {
    /// The edit batch was empty; the previous parse was returned
    /// unchanged after a clone.
    Noop,
    /// Every edit landed inside an annotation-free region (no
    /// `［＃`, `《》`, `｜`, `※`, `〔〕` triggers in the post-edit
    /// surrounding text *and* the previous parse produced no
    /// registry entries / diagnostics), so the lexer's output
    /// reduces to the source bytes verbatim. The fast path skips
    /// `lex()` entirely, building the new [`ParseResult`] from the
    /// post-edit source string with empty registry and diagnostics.
    PlainTextWindow,
    /// The fast paths did not apply, so the implementation called
    /// [`parse`] over the post-edit source. The default route.
    FullReparse,
}

/// Re-parse a document after applying `edits` to `prev_source`.
///
/// `prev` is the previous parse of `prev_source`. The implementation
/// tries the [`IncrementalDecision::PlainTextWindow`] fast path
/// first; when that doesn't apply it falls back to
/// [`IncrementalDecision::FullReparse`]. Either path returns a
/// [`ParseResult`] equivalent (byte-for-byte on `normalized`,
/// structurally on `registry`, count-equal on `diagnostics`) to a
/// full [`parse`] of the post-edit source.
///
/// # Errors
///
/// Propagates [`EditError`] from [`apply_edits`].
pub fn parse_incremental(
    prev: &ParseResult,
    prev_source: &str,
    edits: &[TextEdit],
) -> Result<IncrementalOutcome, EditError> {
    if edits.is_empty() {
        return Ok(IncrementalOutcome {
            result: prev.clone(),
            new_source: prev_source.to_owned(),
            decision: IncrementalDecision::Noop,
        });
    }

    let new_source = apply_edits(prev_source, edits)?;

    if let Some(result) = try_plain_text_window(prev, edits, &new_source) {
        return Ok(IncrementalOutcome {
            result,
            new_source,
            decision: IncrementalDecision::PlainTextWindow,
        });
    }

    let result = parse(&new_source);
    Ok(IncrementalOutcome {
        result,
        new_source,
        decision: IncrementalDecision::FullReparse,
    })
}

/// Plain-text window fast path.
///
/// Fires only when the previous parse produced no annotations and no
/// diagnostics, *and* every edit's replacement text is itself trigger-
/// and Phase-0-clean. Under those conditions the lexer's pipeline
/// reduces to identity — the full parse would just produce
/// `normalized == source` with empty registry and empty diagnostics
/// — so we can skip [`parse`] entirely and synthesise the result
/// from the post-edit source.
///
/// The conservative trigger set (see [`is_aozora_trigger`]) plus the
/// guards against `\r` and BOM cover every classifier branch the
/// lexer might take. Anything that escapes the predicate is
/// guaranteed to round-trip through full parse without difference,
/// so callers can rely on the contract: this function returns
/// `Some(_)` only when the result is provably equal to a full parse.
fn try_plain_text_window(
    prev: &ParseResult,
    edits: &[TextEdit],
    new_source: &str,
) -> Option<ParseResult> {
    // Pre-edit document must already be plain text (empty
    // registry + no diagnostics). If the lexer produced anything,
    // the document has triggers in it whose interaction with the
    // edit cannot be reasoned about locally.
    if !prev.artifacts.registry.is_empty() || !prev.diagnostics.is_empty() {
        return None;
    }
    // Sanity: prev's normalized text must equal prev_source (no
    // Phase 0 transform happened). The two-step check below uses
    // normalized as the witness since we don't carry prev_source
    // into the function.
    //
    // Triggers in `prev.artifacts.normalized` would mean the lexer
    // saw them but happened to leave them unclassified (e.g. a stray
    // `※` between non-recognised contexts). An edit that brings
    // such triggers into a recognisable shape — say, juxtaposing
    // `※[` after deleting an intervening run — would then surface
    // a fresh diagnostic in the full re-parse. The fast path can't
    // anticipate that, so it bails when *any* trigger character
    // appears in the prior buffer.
    if !aozora_triggers_absent(&prev.artifacts.normalized) {
        return None;
    }
    // Replacement text must be trigger-free *and* free of
    // characters Phase 0 sanitize transforms (CR, BOM, accent
    // decomposition brackets are already in the trigger set).
    for edit in edits {
        if !is_plain_phase0_clean(&edit.new_text) {
            return None;
        }
    }
    // Build the trivial parse result for plain text input.
    Some(ParseResult {
        diagnostics: Vec::new(),
        artifacts: ParseArtifacts {
            normalized: new_source.to_owned(),
            registry: crate::PlaceholderRegistry::default(),
        },
    })
}

/// True iff `text` is free of every byte the lexer's Phase 0
/// transforms or that would later trigger a classifier branch.
///
/// Rejects:
/// - `\r` (CRLF folding),
/// - `\u{FEFF}` (BOM),
/// - the four PUA sentinels U+E001..U+E004 — Phase 0 emits a
///   `SourceContainsPua` warning for each occurrence so a buffer
///   containing one is not "plain text" from the lexer's view, and
/// - the eight aozora trigger glyphs (see [`is_aozora_trigger`]).
///
/// A buffer that passes this predicate is guaranteed to lex as a
/// single `Plain` span with no diagnostics.
///
/// # Hot-path layout
///
/// Single-pass over `text.chars()` with a two-tier predicate:
///
/// 1. **ASCII bitmap fast path** ([`ASCII_DIRTY_MASK`]): for `c < 128`
///    the test is `(MASK >> c) & 1` — one cmp + one shift + one AND,
///    with no memory load. Replaces five separate `text.contains()`
///    scans the prior implementation made.
/// 2. **Non-ASCII matches!**: 12 distinct codepoints (BOM, 4 PUA
///    sentinels collapsed to a range pattern, 8 trigger glyphs). The
///    compiler lowers `matches!` with a range arm to a near-optimal
///    decision tree. We don't trade up to a `phf::Set` because
///    matches! over const codepoints stays fully inlined and avoids
///    every memory indirection.
#[inline]
fn is_plain_phase0_clean(text: &str) -> bool {
    !text.chars().any(is_phase0_dirty_char)
}

/// 128-bit bitmap of ASCII codepoints that disqualify a text from the
/// Phase 0 fast path. Today only `\r` (0x0D) is set; if the lexer ever
/// adds new ASCII transforms (e.g. tab folding) extending this is a
/// one-line change.
const ASCII_DIRTY_MASK: u128 = 1u128 << 0x0D;

/// Per-char "dirty" predicate. Hot inner loop of
/// [`is_plain_phase0_clean`].
///
/// The ASCII fast path (`c < 128`) uses a single u128 bitmap probe;
/// for non-ASCII, a `matches!` with one range arm covers all 12
/// trigger codepoints. Marked `#[inline]` so callers see the
/// fully-folded predicate.
#[inline]
const fn is_phase0_dirty_char(c: char) -> bool {
    if (c as u32) < 128 {
        // Branchless ASCII bitmap probe: register-resident u128.
        ((ASCII_DIRTY_MASK >> (c as u32)) & 1) != 0
    } else {
        matches!(
            c,
            '\u{FEFF}'                       // BOM
            | '\u{E001}'..='\u{E004}'        // 4 PUA sentinels collapsed
            | '［' | '］' | '《' | '》'
            | '｜' | '※' | '〔' | '〕'
        )
    }
}

/// True iff `text` contains none of the eight aozora trigger glyphs:
/// `［ ］ 《 》 ｜ ※ 〔 〕`.
///
/// Stricter subset of [`is_plain_phase0_clean`] — does *not* reject
/// `\r`, BOM, or PUA sentinels. Used by the fast path to gate on the
/// prior buffer (which already has those diagnostics, but may carry
/// trigger glyphs without them in unrecognised contexts).
///
/// The trigger set is duplicated inline with [`is_phase0_dirty_char`]'s
/// non-ASCII matches!; the test
/// `trigger_set_inline_predicates_stay_in_sync` pins them.
#[inline]
fn aozora_triggers_absent(text: &str) -> bool {
    !text.chars().any(is_aozora_trigger)
}

/// True iff `c` is one of the eight aozora trigger glyphs. SSOT for
/// the trigger set; both [`aozora_triggers_absent`] and the non-ASCII
/// arm of [`is_phase0_dirty_char`]'s matches! must include every
/// codepoint listed here.
#[inline]
const fn is_aozora_trigger(c: char) -> bool {
    matches!(
        c,
        '［' | '］' | '《' | '》' | '｜' | '※' | '〔' | '〕'
    )
}

/// Return the byte range of the paragraph containing `range`, or
/// `None` if `range` falls outside `source`.
///
/// Paragraphs are delimited by runs of two or more `\n`; a single
/// `\n` is a hardbreak inside a paragraph. The boundary search uses
/// `str::rfind`/`str::find`, which lower to `memrchr`/`memchr` for
/// the ASCII char pattern and run independently of document size.
#[doc(hidden)]
#[must_use]
pub fn paragraph_containing(source: &str, range: Range<usize>) -> Option<&str> {
    if range.start > source.len() || range.end > source.len() {
        return None;
    }
    let prefix = &source[..range.start];
    // Find the most recent `\n\n` (or start of buffer).
    let para_start = prefix.rfind("\n\n").map_or(0, |idx| idx + 2);
    // From the edit end, scan forward for the next `\n\n` (or EOF).
    let after = &source[range.end..];
    let para_end = after.find("\n\n").map_or(source.len(), |idx| range.end + idx);
    Some(&source[para_start..para_end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ---------------------------------------------------------------
    // apply_edits — basic correctness
    // ---------------------------------------------------------------

    #[test]
    fn empty_edit_batch_is_identity() {
        let out = apply_edits("hello world", &[]).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn single_replacement_in_middle() {
        let out = apply_edits(
            "hello world",
            &[TextEdit::new(6..11, "rust")],
        )
        .unwrap();
        assert_eq!(out, "hello rust");
    }

    #[test]
    fn pure_insertion_at_beginning() {
        let out = apply_edits("world", &[TextEdit::new(0..0, "hello ")]).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn pure_insertion_at_end() {
        let src = "hello";
        let out = apply_edits(
            src,
            &[TextEdit::new(src.len()..src.len(), " world")],
        )
        .unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn pure_deletion() {
        let out = apply_edits(
            "hello world",
            &[TextEdit::new(5..6, "")],
        )
        .unwrap();
        assert_eq!(out, "helloworld");
    }

    // ---------------------------------------------------------------
    // apply_edits — multi-edit ordering
    // ---------------------------------------------------------------

    #[test]
    fn multiple_disjoint_edits_apply_in_source_order() {
        let out = apply_edits(
            "abcdefg",
            &[
                TextEdit::new(0..1, "X"),
                TextEdit::new(3..4, "Y"),
                TextEdit::new(6..7, "Z"),
            ],
        )
        .unwrap();
        assert_eq!(out, "XbcYefZ");
    }

    #[test]
    fn multi_edit_input_order_is_preserved_when_sorted_by_range() {
        // Same disjoint edits as above but provided out of order.
        let out = apply_edits(
            "abcdefg",
            &[
                TextEdit::new(6..7, "Z"),
                TextEdit::new(0..1, "X"),
                TextEdit::new(3..4, "Y"),
            ],
        )
        .unwrap();
        assert_eq!(out, "XbcYefZ");
    }

    #[test]
    fn equal_start_edits_apply_in_input_order() {
        // Two pure insertions at the same offset: input order
        // determines the resulting concatenation. The sort uses
        // `sort_by_key`, which is stable; subsequent runs must not
        // reorder them.
        let out = apply_edits("ab", &[
            TextEdit::new(1..1, "X"),
            TextEdit::new(1..1, "Y"),
        ]).unwrap();
        assert_eq!(out, "aXYb");
    }

    // ---------------------------------------------------------------
    // apply_edits — error paths
    // ---------------------------------------------------------------

    #[test]
    fn inverted_range_is_rejected() {
        // Build the inverted range from variables so clippy's
        // `reversed_empty_ranges` lint (which scans literal `a..b`
        // ranges) doesn't flag the test setup as obviously empty.
        let (start, end) = (3, 1);
        let err = apply_edits("hello", &[TextEdit::new(start..end, "x")]).unwrap_err();
        assert!(matches!(err, EditError::InvertedRange { .. }));
    }

    #[test]
    fn out_of_bounds_range_is_rejected() {
        let err = apply_edits("hello", &[TextEdit::new(0..99, "x")]).unwrap_err();
        assert!(matches!(err, EditError::OutOfBounds { .. }));
    }

    #[test]
    fn non_char_boundary_start_is_rejected() {
        // 「あ」 is 3 bytes in UTF-8; offset 1 is mid-character.
        let err = apply_edits("あ", &[TextEdit::new(1..3, "x")]).unwrap_err();
        assert!(matches!(err, EditError::NotCharBoundary { offset: 1 }));
    }

    #[test]
    fn non_char_boundary_end_is_rejected() {
        let err = apply_edits("あ", &[TextEdit::new(0..2, "x")]).unwrap_err();
        assert!(matches!(err, EditError::NotCharBoundary { offset: 2 }));
    }

    #[test]
    fn overlapping_edits_are_rejected() {
        let err = apply_edits(
            "abcdef",
            &[TextEdit::new(0..3, "X"), TextEdit::new(2..5, "Y")],
        )
        .unwrap_err();
        assert!(matches!(err, EditError::OverlappingEdits { .. }));
    }

    #[test]
    fn touching_but_disjoint_edits_are_accepted() {
        // `[0..2)` and `[2..4)` share endpoint 2 but no bytes.
        let out = apply_edits(
            "abcd",
            &[TextEdit::new(0..2, "AB"), TextEdit::new(2..4, "CD")],
        )
        .unwrap();
        assert_eq!(out, "ABCD");
    }

    // ---------------------------------------------------------------
    // apply_edits — multibyte UTF-8 correctness
    // ---------------------------------------------------------------

    #[test]
    fn multibyte_replacement_at_char_boundaries() {
        // 「あいう」 is 9 bytes (3 each); replace the middle char.
        let out = apply_edits("あいう", &[TextEdit::new(3..6, "X")]).unwrap();
        assert_eq!(out, "あXう");
    }

    #[test]
    fn cumulative_new_capacity_avoids_realloc_in_dense_inserts() {
        // Smoke test: many small inserts succeed without panicking.
        let edits: Vec<TextEdit> = (0..16)
            .map(|i| TextEdit::new(i..i, "+"))
            .collect();
        let out = apply_edits("abcdefghijklmnop", &edits).unwrap();
        assert_eq!(out, "+a+b+c+d+e+f+g+h+i+j+k+l+m+n+o+p");
    }

    // ---------------------------------------------------------------
    // parse_incremental — equivalence with full re-parse
    // ---------------------------------------------------------------

    #[test]
    fn parse_incremental_empty_edits_returns_prev() {
        let prev_source = "Hello, world.";
        let prev = parse(prev_source);
        let outcome = parse_incremental(&prev, prev_source, &[]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::Noop);
        assert_eq!(outcome.new_source, prev_source);
        // Diagnostic is not PartialEq (it embeds miette spans);
        // compare counts as a proxy.
        assert_eq!(
            outcome.result.diagnostics.len(),
            prev.diagnostics.len()
        );
        assert_eq!(outcome.result.artifacts.normalized, prev.artifacts.normalized);
    }

    #[test]
    fn parse_incremental_plain_paragraph_edit_takes_fast_path_tag() {
        let prev_source = "Hello, world.\n";
        let prev = parse(prev_source);
        let outcome = parse_incremental(
            &prev,
            prev_source,
            &[TextEdit::new(7..12, "Rust!")],
        )
        .unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::PlainTextWindow);
        assert_eq!(outcome.new_source, "Hello, Rust!.\n");
    }

    #[test]
    fn parse_incremental_aozora_edit_takes_full_reparse_tag() {
        let prev_source = "｜青梅《おうめ》\n";
        let prev = parse(prev_source);
        // Edit the reading inside the ruby annotation.
        let new_text = "｜青梅《おーめ》\n";
        let edit = TextEdit::new(0..prev_source.len(), new_text.to_owned());
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::FullReparse);
        assert_eq!(outcome.new_source, new_text);
    }

    #[test]
    fn parse_incremental_byte_equivalence_with_full_parse_on_aozora_edit() {
        let prev_source = "｜青梅《おうめ》\n";
        let prev = parse(prev_source);
        let new_text = "｜青梅《おーめ》\n";
        let edit = TextEdit::new(0..prev_source.len(), new_text.to_owned());
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        let full = parse(new_text);
        assert_eq!(outcome.result.artifacts.normalized, full.artifacts.normalized);
        assert_eq!(outcome.result.diagnostics.len(), full.diagnostics.len());
    }

    #[test]
    fn parse_incremental_fast_path_result_matches_full_parse_byte_for_byte() {
        // The PlainTextWindow path skips lex() entirely; the result
        // must still match what a full parse would produce for the
        // same input.
        let prev_source = "first line\n\nsecond line\n";
        let prev = parse(prev_source);
        let edit = TextEdit::new(6..10, "edit");
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::PlainTextWindow);
        let full = parse(&outcome.new_source);
        assert_eq!(outcome.result.artifacts.normalized, full.artifacts.normalized);
        assert_eq!(outcome.result.diagnostics.len(), full.diagnostics.len());
        assert_eq!(outcome.result.artifacts.registry.len(), full.artifacts.registry.len());
    }

    #[test]
    fn fast_path_rejects_edits_that_introduce_triggers() {
        // The new text adds `｜` which is a ruby trigger — fast
        // path must refuse and fall back to full reparse.
        let prev_source = "plain text\n";
        let prev = parse(prev_source);
        let edit = TextEdit::new(5..5, "｜青梅《おうめ》");
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::FullReparse);
        // And the resulting parse must reflect the new ruby.
        assert_eq!(outcome.result.artifacts.registry.len(), 1);
    }

    #[test]
    fn fast_path_rejects_when_prev_has_annotations() {
        // Prev document has triggers; even a plain-text edit
        // forces full reparse (we'd need to shift registry offsets
        // otherwise — that's the future smarter path).
        let prev_source = "前｜青梅《おうめ》後\n";
        let prev = parse(prev_source);
        // Insert plain text at the end (post-annotation).
        let edit = TextEdit::new(prev_source.len()..prev_source.len(), "tail");
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::FullReparse);
    }

    #[test]
    fn fast_path_rejects_carriage_return_in_replacement() {
        // `\r` triggers Phase 0 CRLF normalisation — the fast path
        // can't replicate that so it falls back.
        let prev_source = "plain\n";
        let prev = parse(prev_source);
        let edit = TextEdit::new(5..5, "\r\n");
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::FullReparse);
    }

    #[test]
    fn fast_path_rejects_bom_in_replacement() {
        // BOM gets stripped by Phase 0; fast path must defer.
        let prev_source = "plain\n";
        let prev = parse(prev_source);
        let edit = TextEdit::new(0..0, "\u{FEFF}");
        let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
        assert_eq!(outcome.decision, IncrementalDecision::FullReparse);
    }

    #[test]
    fn trigger_set_inline_predicates_stay_in_sync() {
        // The trigger set lives in two inline `matches!` patterns:
        //   - is_aozora_trigger (used by aozora_triggers_absent)
        //   - the non-ASCII arm of is_phase0_dirty_char
        //
        // This test pins both to the spec list: every glyph below
        // must be classified the same way by both predicates. A
        // future trigger added to one place but not the other gets
        // caught here.
        const SPEC_TRIGGERS: &[char] =
            &['［', '］', '《', '》', '｜', '※', '〔', '〕'];
        for &ch in SPEC_TRIGGERS {
            assert!(
                is_aozora_trigger(ch),
                "{ch:?} should be classified as trigger by is_aozora_trigger",
            );
            assert!(
                is_phase0_dirty_char(ch),
                "{ch:?} should be classified as dirty by is_phase0_dirty_char",
            );
            assert!(
                !aozora_triggers_absent(&ch.to_string()),
                "{ch:?} should make aozora_triggers_absent return false",
            );
        }
    }

    #[test]
    fn fast_path_rejects_pua_sentinel_in_replacement() {
        // Phase 0 emits `SourceContainsPua` for U+E001..U+E004; the
        // fast path must defer so a full parse can surface the
        // warning. Without this the LSP would silently swallow the
        // diagnostic when the user pastes a sentinel-shaped char.
        for sentinel in [
            crate::INLINE_SENTINEL,
            crate::BLOCK_LEAF_SENTINEL,
            crate::BLOCK_OPEN_SENTINEL,
            crate::BLOCK_CLOSE_SENTINEL,
        ] {
            let prev_source = "plain\n";
            let prev = parse(prev_source);
            let edit = TextEdit::new(0..0, sentinel.to_string());
            let outcome = parse_incremental(&prev, prev_source, &[edit]).unwrap();
            assert_eq!(
                outcome.decision,
                IncrementalDecision::FullReparse,
                "PUA sentinel U+{:04X} must force full reparse",
                sentinel as u32,
            );
            // The diagnostic must be present in the result, not lost.
            assert!(
                !outcome.result.diagnostics.is_empty(),
                "PUA sentinel must surface a diagnostic; got {:?}",
                outcome.result.diagnostics,
            );
        }
    }

    // ---------------------------------------------------------------
    // paragraph_containing — boundary helper
    // ---------------------------------------------------------------

    #[test]
    fn paragraph_containing_finds_full_buffer_when_no_blank_lines() {
        let p = paragraph_containing("hello world", 3..3).unwrap();
        assert_eq!(p, "hello world");
    }

    #[test]
    fn paragraph_containing_clips_to_blank_line_boundaries() {
        let src = "first\n\nmiddle\n\nlast";
        let p = paragraph_containing(src, 9..9).unwrap();
        assert_eq!(p, "middle");
    }

    #[test]
    fn paragraph_containing_returns_none_when_range_out_of_bounds() {
        assert!(paragraph_containing("hi", 0..99).is_none());
    }
}
