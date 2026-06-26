//! Segment-cache foundation for incremental re-parse (#237, Stage A).
//!
//! [`SegmentedParse`] splits a document into **independently-lexable
//! segments** so a later edit can re-lex only the segment it touched
//! instead of the whole document. This module is the behaviour-preserving
//! foundation: it computes the safe segmentation, caches each segment's
//! diagnostics, and proves — via the `corpus_incremental_merge` gate —
//! that reassembling the per-segment diagnostics is byte-for-byte
//! equivalent to a whole-document parse. The runtime that wires this into
//! the LSP edit path arrives in Stage A2/A3.
//!
//! # Why segmentation is subtle
//!
//! Three independent hazards make a naive blank-line split wrong:
//!
//! 1. **Structure straddling a boundary.** An inline delimiter pair
//!    (`《》`, `［＃…］`) or a block container (字下げ / 見出し / 注記 …) can
//!    span a blank line; re-lexing half of it in isolation invents a
//!    phantom unclosed/unmatched bracket or wrongly nests a container. A cut is
//!    therefore admitted only where the block-container nesting depth is
//!    `0` **and** no resolved delimiter pair straddles it.
//! 2. **The sanitize coordinate gap.** The lexer first *sanitizes* the raw
//!    source — CRLF→LF, BOM strip, `〔…〕` accent decomposition,
//!    decorative-rule isolation, PUA neutralization — which shifts byte
//!    offsets, and diagnostics are reported in **sanitized** coordinates.
//!    Real Aozora Bunko files are CRLF, so sanitized ≠ source for nearly
//!    every document. Segments are therefore cut on the *raw* source
//!    (so each segment re-runs sanitize and reproduces sanitize-stage
//!    diagnostics like `SourceContainsPua`), but each segment's diagnostics
//!    are rebased by the segment's **cumulative sanitized length**, not its
//!    raw offset. The structural safety check (depth / pair-straddle) is
//!    evaluated at that sanitized offset against the whole-document parse's
//!    [`source_nodes`](crate::Tree::source_nodes) /
//!    [`pairs`](crate::Tree::pairs), which live in sanitized coordinates.
//! 3. **Whole-document-scoped diagnostics.** A handful of the parser's checks
//!    are document-global — forward-reference resolution (bouten ambiguity,
//!    縦中横 and standalone-gaiji targeting) and end-of-document kaeriten
//!    pairing — and a segment sees only its own slice of that context, so it
//!    can differ from the full parse in *either* direction (missing a real
//!    diagnostic or inventing a phantom). These are taken wholesale from the
//!    whole-document parse ([`SegmentedParse::whole_document_scoped`]) and
//!    excluded from the per-segment caches; Stage A2/A3 recomputes them
//!    globally on edit. See [`is_whole_document_scoped`] for the exact set.
//!
//! Candidate boundaries are blank-line boundaries on the raw source. Where a
//! candidate is unsafe, the flanking segments are merged into one run and
//! re-lexed together, so the result is always correct — at worst the
//! document is one whole-document segment.

use core::cmp::Ordering;
use core::ops::Range;

use crate::incremental_owned::{
    candidate_boundaries, carries_structure, is_whole_document_scoped, shift_u32, shift_usize,
    structurally_safe,
};
use crate::{Diagnostic, Document};

/// A document's safe segmentation into independently-lexable spans, with
/// each segment's diagnostics cached.
///
/// Construct with [`SegmentedParse::of`]. Segment byte ranges are over the
/// raw source and concatenate back to it exactly. When no safe split
/// exists the segmentation degrades to a single whole-document segment —
/// never to an incorrect split.
#[derive(Debug, Clone)]
pub struct SegmentedParse {
    /// The raw source, owned so the segmentation is self-contained.
    source: Box<str>,
    /// Segments in source order; concatenating their raw ranges reproduces
    /// `source`.
    segments: Vec<Segment>,
    /// Diagnostics that depend on the whole preceding document and so cannot
    /// be reproduced by any single segment (see the module docs). Stored in
    /// whole-document sanitized coordinates.
    whole_scoped: Vec<Diagnostic>,
}

/// One independently-lexable segment.
#[derive(Debug, Clone)]
struct Segment {
    /// Raw-source byte range of this segment.
    raw: Range<u32>,
    /// Sanitized-coordinate offset of this segment's start in the
    /// whole-document sanitized buffer — the amount each cached diagnostic
    /// is shifted by to reach whole-document coordinates.
    san_start: u32,
    /// This segment's own sanitized length (the next segment's `san_start`
    /// minus this one's). Cached so an incremental re-lex can shift the
    /// trailing segments by the sanitized-length delta.
    san_len: u32,
    /// This segment's diagnostics, in segment-local sanitized coordinates.
    diagnostics: Vec<Diagnostic>,
}

impl SegmentedParse {
    /// Compute the safe segmentation of `source` from one whole-document
    /// parse, caching each segment's diagnostics.
    #[must_use]
    pub fn of(source: &str) -> Self {
        let document = Document::new(source);
        let tree = document.parse();
        let nodes = tree.source_nodes();
        let pairs = tree.pairs();

        // Candidate blank-line boundaries on the raw source.
        let cuts = candidate_boundaries(source);

        // Re-lex each candidate segment to obtain its sanitized length and
        // diagnostics. Candidate segment `i` = `[bounds[i], bounds[i + 1])`.
        let mut bounds = Vec::with_capacity(cuts.len() + 2);
        bounds.push(0usize);
        bounds.extend(cuts.iter().copied());
        bounds.push(source.len());

        let candidates: Vec<Candidate> = bounds
            .windows(2)
            .map(|w| relex_segment(source, w[0]..w[1]))
            .collect();

        // Cumulative sanitized offset at each candidate boundary.
        // `san_off[i]` is the whole-document sanitized offset of candidate
        // segment `i`'s start.
        let mut san_off = vec![0u32; candidates.len() + 1];
        for (i, cand) in candidates.iter().enumerate() {
            san_off[i + 1] = san_off[i].saturating_add(cand.san_len);
        }

        // Group candidates into runs delimited by *structurally safe*
        // boundaries; unsafe boundaries merge their neighbours.
        let mut segments = Vec::new();
        let mut run_start = 0usize;
        for boundary in 1..=candidates.len() {
            let is_cut =
                boundary == candidates.len() || structurally_safe(san_off[boundary], nodes, pairs);
            if is_cut {
                segments.push(build_segment(
                    source,
                    &candidates,
                    san_off[run_start],
                    run_start..boundary,
                ));
                run_start = boundary;
            }
        }

        // Forward-reference diagnostics depend on the whole preceding
        // document and so are taken from the whole-document parse, never from
        // a segment (which can resolve them wrongly in *either* direction — both
        // missing a real one and inventing a phantom). The per-segment caches
        // exclude this class entirely (see `relex_segment`).
        let whole_scoped = tree
            .diagnostics()
            .iter()
            .filter(|d| is_whole_document_scoped(d))
            .cloned()
            .collect();

        Self {
            source: source.into(),
            segments,
            whole_scoped,
        }
    }

    /// The raw source this segmentation was computed from.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Number of segments (always `>= 1`).
    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Whether the document was split into more than one segment. `false`
    /// means it degraded to a single whole-document segment.
    #[must_use]
    pub fn is_segmented(&self) -> bool {
        self.segments.len() > 1
    }

    /// The raw-source byte range of each segment, in order. Concatenating
    /// the segments reproduces the source exactly.
    #[must_use]
    pub fn segment_ranges(&self) -> Vec<Range<usize>> {
        self.segments
            .iter()
            .map(|s| s.raw.start as usize..s.raw.end as usize)
            .collect()
    }

    /// Diagnostics that depend on the whole preceding document and cannot be
    /// reproduced by an isolated segment (forward-reference ambiguity; see
    /// the module docs). In whole-document sanitized coordinates.
    #[must_use]
    pub fn whole_document_scoped(&self) -> &[Diagnostic] {
        &self.whole_scoped
    }

    /// The complete whole-document diagnostics, reassembled from the cached
    /// per-segment locals (rebased into whole-document coordinates) plus the
    /// whole-document-scoped diagnostics, sorted by position.
    ///
    /// By construction this equals `Document::new(source).parse()`'s
    /// diagnostics as a positional multiset; the `corpus_incremental_merge`
    /// gate proves it over the reference corpus.
    #[must_use]
    pub fn merged_diagnostics(&self) -> Vec<Diagnostic> {
        let mut merged = reassemble_local(&self.segments);
        merged.extend(self.whole_scoped.iter().cloned());
        merged.sort_by(diagnostic_order);
        merged
    }

    /// Recompute the segmentation for `new_text` after the single-region edit
    /// that replaced `edit_old` (a byte range in *this* segmentation's
    /// source) with the corresponding slice of `new_text`, reusing the cached
    /// diagnostics of every segment the edit does not touch.
    ///
    /// Falls back to a full [`SegmentedParse::of`] — always correct — whenever
    /// the fast path cannot be proven safe:
    ///
    /// - the cached document has whole-document-scoped diagnostics
    ///   (forward-reference / container pairing), which any edit can perturb;
    /// - the edit is not contained in a single segment's interior; or
    /// - the edited bytes (old or new) carry document structure — a line break
    ///   (`\n` / `\r`, which could move a blank-line boundary) or a directive
    ///   opener `［` (which could introduce a container or forward reference).
    ///
    /// Under the fast path exactly one segment is re-lexed; the rest are
    /// reused with their byte ranges and sanitized offsets shifted by the
    /// edit's length deltas. The returned [`IncrementalOutcome`] reports how
    /// many segments were reused. The `reparse_incremental_equals_full_parse`
    /// corpus gate proves the result's diagnostics equal a from-scratch parse
    /// of `new_text`.
    #[must_use]
    pub fn reparse_incremental(
        &self,
        new_text: &str,
        edit_old: Range<usize>,
    ) -> (Self, IncrementalOutcome) {
        if let Some(fast) = self.try_reuse(new_text, &edit_old) {
            return fast;
        }
        let full = Self::of(new_text);
        let relexed = u64::try_from(full.segments.len()).unwrap_or(u64::MAX);
        (
            full,
            IncrementalOutcome {
                reused_segments: 0,
                relexed_segments: relexed,
                reused: false,
            },
        )
    }

    /// The fast path of [`Self::reparse_incremental`]; `None` when a
    /// precondition fails (the caller then does a full parse).
    fn try_reuse(
        &self,
        new_text: &str,
        edit_old: &Range<usize>,
    ) -> Option<(Self, IncrementalOutcome)> {
        // (1) whole-document-scoped diagnostics make any edit unsafe to localise.
        if !self.whole_scoped.is_empty() {
            return None;
        }
        // Validate the edit range against the cached source.
        let old_source = self.source.as_ref();
        if edit_old.start > edit_old.end || edit_old.end > old_source.len() {
            return None;
        }
        let old_len = edit_old.end - edit_old.start;
        // Replacement length: new_text == old with `edit_old` swapped out.
        let new_total = i64::try_from(new_text.len()).ok()?;
        let old_total = i64::try_from(old_source.len()).ok()?;
        let new_len = new_total - old_total + i64::try_from(old_len).ok()?;
        let new_len = usize::try_from(new_len).ok()?;
        let new_edit_end = edit_old.start.checked_add(new_len)?;
        if new_edit_end > new_text.len() {
            return None;
        }

        // The edit must actually transform `old_source` into `new_text`: the
        // bytes outside `edit_old` are unchanged. Verifying this makes the
        // fast path robust to an incorrectly specified edit — a mismatch falls
        // back to a (correct) full parse.
        if old_source.as_bytes().get(..edit_old.start) != new_text.as_bytes().get(..edit_old.start)
            || old_source.as_bytes().get(edit_old.end..) != new_text.as_bytes().get(new_edit_end..)
        {
            return None;
        }

        // (3) The edited bytes (old and new) must carry no document structure.
        let old_slice = old_source.get(edit_old.clone())?;
        let new_slice = new_text.get(edit_old.start..new_edit_end)?;
        if carries_structure(old_slice) || carries_structure(new_slice) {
            return None;
        }

        // (2) The edit must sit inside exactly one segment's interior.
        let start = u32::try_from(edit_old.start).ok()?;
        let end = u32::try_from(edit_old.end).ok()?;
        let idx = self
            .segments
            .iter()
            .position(|s| s.raw.start <= start && end <= s.raw.end)?;

        // Re-lex the edited segment with its new content.
        let delta = i64::try_from(new_len).ok()? - i64::try_from(old_len).ok()?;
        let seg = &self.segments[idx];
        let new_seg_end = shift_usize(seg.raw.end, delta)?;
        let new_seg_raw = seg.raw.start as usize..new_seg_end;
        let relexed = relex_segment(new_text, new_seg_raw);
        // The edit may have *introduced* a whole-document-scoped diagnostic
        // (e.g. broken a `※［＃…］` gaiji or a container directive whose
        // sanitized span the raw edit landed inside). The cached whole-scoped
        // set is then stale, so fall back to a full parse.
        if relexed.has_whole_scoped {
            return None;
        }
        let san_delta = i64::from(relexed.san_len) - i64::from(seg.san_len);

        // Build the new segment list: prefix unchanged, the edited segment
        // replaced, the suffix shifted by the byte and sanitized deltas.
        let mut segments = Vec::with_capacity(self.segments.len());
        segments.extend(self.segments[..idx].iter().cloned());
        segments.push(Segment {
            raw: seg.raw.start..u32::try_from(new_seg_end).ok()?,
            san_start: seg.san_start,
            san_len: relexed.san_len,
            diagnostics: relexed.diagnostics,
        });
        for s in &self.segments[idx + 1..] {
            segments.push(Segment {
                raw: shift_u32(s.raw.start, delta)?..shift_u32(s.raw.end, delta)?,
                san_start: shift_u32(s.san_start, san_delta)?,
                san_len: s.san_len,
                diagnostics: s.diagnostics.clone(),
            });
        }

        let reused = u64::try_from(segments.len().saturating_sub(1)).unwrap_or(u64::MAX);
        Some((
            Self {
                source: new_text.into(),
                segments,
                whole_scoped: Vec::new(),
            },
            IncrementalOutcome {
                reused_segments: reused,
                relexed_segments: 1,
                reused: true,
            },
        ))
    }
}

/// How an incremental re-parse reused the prior segmentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncrementalOutcome {
    /// Segments carried over unchanged (cache hits).
    pub reused_segments: u64,
    /// Segments re-lexed (cache misses). `1` on the fast path; the whole
    /// document on a fallback.
    pub relexed_segments: u64,
    /// Whether the incremental fast path applied. `false` means a full parse.
    pub reused: bool,
}

/// Reassemble the per-segment local diagnostics into whole-document
/// coordinates by shifting each segment's cached diagnostics by the
/// segment's sanitized start offset.
fn reassemble_local(segments: &[Segment]) -> Vec<Diagnostic> {
    let mut local = Vec::new();
    for seg in segments {
        let shift = i64::from(seg.san_start);
        local.extend(seg.diagnostics.iter().map(|d| d.clone().shifted(shift)));
    }
    local
}

/// Total order over diagnostics by position, then by debug representation as
/// a tiebreak (so two diagnostics at the same span sort deterministically).
fn diagnostic_order(a: &Diagnostic, b: &Diagnostic) -> Ordering {
    let (sa, sb) = (a.span(), b.span());
    (sa.start, sa.end)
        .cmp(&(sb.start, sb.end))
        .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
}

/// A candidate segment's re-lex result.
struct Candidate {
    raw: Range<usize>,
    san_len: u32,
    diagnostics: Vec<Diagnostic>,
    /// Whether the isolated re-lex itself produced a whole-document-scoped
    /// diagnostic. Used by the incremental fast path: if an edit makes a
    /// segment produce one (e.g. by breaking a `※［＃…］` gaiji or a container
    /// directive), the cached empty whole-scoped set is stale and the fast
    /// path must fall back to a full parse.
    has_whole_scoped: bool,
}

/// Re-lex `source[raw]` in isolation, capturing its sanitized length and its
/// **local** diagnostics — whole-document-scoped diagnostics (forward
/// references) are dropped here because a segment cannot resolve them; they
/// are supplied separately from the whole-document parse.
fn relex_segment(source: &str, raw: Range<usize>) -> Candidate {
    let document = Document::new(&source[raw.clone()]);
    let tree = document.parse();
    let san_len = u32::try_from(tree.sanitized().len()).unwrap_or(u32::MAX);
    let mut has_whole_scoped = false;
    let diagnostics = tree
        .diagnostics()
        .iter()
        .filter(|d| {
            if is_whole_document_scoped(d) {
                has_whole_scoped = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    Candidate {
        raw,
        san_len,
        diagnostics,
        has_whole_scoped,
    }
}

/// Build a [`Segment`] for the run of candidates `run` (a half-open index
/// range into `candidates`), whose start sits at sanitized offset
/// `san_start`. A single-candidate run reuses the cached diagnostics; a
/// merged run re-lexes the concatenated raw span so the diagnostics reflect
/// the joined text (no phantom unclosed/unmatched brackets).
fn build_segment(
    source: &str,
    candidates: &[Candidate],
    san_start: u32,
    run: Range<usize>,
) -> Segment {
    let raw = candidates[run.start].raw.start..candidates[run.end - 1].raw.end;
    let san_len = candidates[run.clone()]
        .iter()
        .map(|c| c.san_len)
        .fold(0u32, u32::saturating_add);
    let diagnostics = if run.len() == 1 {
        candidates[run.start].diagnostics.clone()
    } else {
        relex_segment(source, raw.clone()).diagnostics
    };
    Segment {
        raw: u32::try_from(raw.start).unwrap_or(u32::MAX)
            ..u32::try_from(raw.end).unwrap_or(u32::MAX),
        san_start,
        san_len,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_debug(mut diags: Vec<Diagnostic>) -> Vec<String> {
        diags.sort_by(diagnostic_order);
        diags.iter().map(|d| format!("{d:?}")).collect()
    }

    fn assert_merge_equals_whole(src: &str) {
        let whole_doc = Document::new(src);
        let whole = sorted_debug(whole_doc.parse().diagnostics().to_vec());
        let merged = sorted_debug(SegmentedParse::of(src).merged_diagnostics());
        assert_eq!(whole, merged, "segmented merge must equal whole-doc parse");
    }

    /// Plain LF paragraphs split at every blank line and merge back.
    #[test]
    fn plain_lf_paragraphs_split_and_merge() {
        let src = "first paragraph\n\nsecond paragraph\n\nthird";
        let seg = SegmentedParse::of(src);
        assert!(seg.is_segmented());
        assert_eq!(seg.segment_count(), 3);
        let rejoined: String = seg.segment_ranges().into_iter().map(|r| &src[r]).collect();
        assert_eq!(rejoined, src);
        assert_merge_equals_whole(src);
    }

    /// CRLF paragraphs (the real corpus shape) also split and merge — the
    /// per-segment diagnostics rebase by sanitized length, not raw offset.
    #[test]
    fn crlf_paragraphs_split_and_merge() {
        let src = "first paragraph\r\n\r\nsecond paragraph\r\n\r\nthird";
        let seg = SegmentedParse::of(src);
        assert!(seg.is_segmented());
        let rejoined: String = seg.segment_ranges().into_iter().map(|r| &src[r]).collect();
        assert_eq!(
            rejoined, src,
            "segments must concatenate back to the source"
        );
        assert_merge_equals_whole(src);
    }

    /// A no-blank-line document is a single segment.
    #[test]
    fn single_line_is_one_segment() {
        let seg = SegmentedParse::of("no blank lines here");
        assert!(!seg.is_segmented());
        assert_eq!(seg.segment_count(), 1);
        assert_eq!(seg.segment_ranges(), vec![0..19]);
        assert_eq!(seg.source(), "no blank lines here");
    }

    /// A block container (字下げ) that spans a blank line must not be split.
    #[test]
    fn container_spanning_blank_line_is_not_split() {
        let src = "［＃ここから２字下げ］\nアイウ\n\nエオカ\n［＃ここで字下げ終わり］";
        let seg = SegmentedParse::of(src);
        assert!(!seg.is_segmented(), "container must stay in one segment");
        assert_merge_equals_whole(src);
    }

    /// A sanitize-stage diagnostic (`SourceContainsPua`) raised inside a
    /// later segment is reproduced and rebased into whole-doc coordinates.
    #[test]
    fn pua_in_later_segment_is_reproduced_and_rebased() {
        let src = "clean first paragraph\n\nsecond \u{E001} here";
        let seg = SegmentedParse::of(src);
        assert!(seg.is_segmented());
        assert_merge_equals_whole(src);
        let merged = seg.merged_diagnostics();
        assert_eq!(merged.len(), 1, "exactly the one PUA diagnostic");
        assert!(
            matches!(merged[0], Diagnostic::SourceContainsPua { .. }),
            "got {:?}",
            merged[0],
        );
        // It is a local diagnostic, not a whole-document-scoped one.
        assert!(seg.whole_document_scoped().is_empty());
        // The span points into paragraph 2, not segment-local offset 0.
        assert!(
            merged[0].span().start >= 23,
            "span must be rebased past the cut"
        );
    }

    /// A forward-reference bouten whose single target occurs in an earlier
    /// paragraph is whole-document-scoped: no isolated segment can flag the
    /// ambiguity, so it is carried from the whole-document parse and still
    /// appears in the merge.
    #[test]
    fn forward_bouten_ambiguity_is_whole_document_scoped() {
        // "みかん" occurs in paragraphs 1 and 2; the directive in paragraph 3
        // references it, so the whole-document parse flags the ambiguity.
        let src = "みかんを食べた\n\nみかんは赤い\n\n「みかん」［＃「みかん」に傍点］";
        let seg = SegmentedParse::of(src);
        let whole_doc = Document::new(src);
        let whole = sorted_debug(whole_doc.parse().diagnostics().to_vec());
        let merged = sorted_debug(seg.merged_diagnostics());
        assert_eq!(
            whole, merged,
            "merge must reproduce the whole-doc diagnostics"
        );
        // The ambiguity is carried as a whole-document-scoped diagnostic
        // because the segment holding the directive cannot see the earlier
        // occurrences.
        assert!(
            seg.whole_document_scoped()
                .iter()
                .any(|d| matches!(d, Diagnostic::BoutenTargetAmbiguous { .. })),
            "expected a carried BoutenTargetAmbiguous, got {:?}",
            seg.whole_document_scoped(),
        );
    }

    /// Apply a byte-range replacement to produce the edited text, then assert
    /// that `reparse_incremental` yields the same diagnostics as a from-scratch
    /// parse of the edited text. Returns the outcome for further assertions.
    fn check_incremental(old: &str, edit: Range<usize>, replacement: &str) -> IncrementalOutcome {
        let mut new_text = String::new();
        new_text.push_str(&old[..edit.start]);
        new_text.push_str(replacement);
        new_text.push_str(&old[edit.end..]);

        let cached = SegmentedParse::of(old);
        let (incremental, outcome) = cached.reparse_incremental(&new_text, edit);

        assert_eq!(
            incremental.source(),
            new_text,
            "source must be the edited text"
        );
        assert_eq!(
            sorted_debug(incremental.merged_diagnostics()),
            sorted_debug(SegmentedParse::of(&new_text).merged_diagnostics()),
            "incremental diagnostics must equal a from-scratch parse",
        );
        outcome
    }

    /// A plain-prose edit inside one segment reuses the other segments.
    #[test]
    fn incremental_plain_edit_reuses_segments() {
        let old = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
        // Replace "second" with "edited" — interior of the middle segment.
        let at = old.find("second").unwrap();
        let outcome = check_incremental(old, at..at + "second".len(), "edited");
        assert!(
            outcome.reused,
            "plain interior edit must take the fast path"
        );
        assert_eq!(outcome.relexed_segments, 1);
        assert!(
            outcome.reused_segments >= 2,
            "the two untouched segments reuse"
        );
    }

    /// An edit that inserts a directive opener falls back to a full parse
    /// (it could introduce a container / forward reference) but stays correct.
    #[test]
    fn incremental_directive_edit_falls_back_but_correct() {
        let old = "alpha\n\nbeta\n\ngamma";
        let at = old.find("beta").unwrap();
        let outcome = check_incremental(old, at..at, "［＃ここから２字下げ］\n");
        assert!(!outcome.reused, "introducing a directive must fall back");
    }

    /// An edit that adds a blank line (document structure) falls back.
    #[test]
    fn incremental_blank_line_edit_falls_back() {
        let old = "alpha beta gamma";
        let at = old.find("beta").unwrap();
        let outcome = check_incremental(old, at..at, "beta\n\nmore ");
        assert!(!outcome.reused, "introducing a blank line must fall back");
    }

    /// A document whose cached parse carries a whole-document-scoped diagnostic
    /// always falls back (the global diagnostic could change).
    #[test]
    fn incremental_global_doc_falls_back() {
        let old = "みかんを食べた\n\nみかんは赤い\n\n「みかん」［＃「みかん」に傍点］";
        let cached = SegmentedParse::of(old);
        assert!(!cached.whole_document_scoped().is_empty());
        // A trivial plain edit in the first segment still falls back.
        let outcome = check_incremental(old, 0..0, "前置き ");
        assert!(
            !outcome.reused,
            "a doc with global diagnostics must fall back"
        );
    }
}
