//! Owned-AST incremental re-parse engine for #237 Stage B'.
//!
//! Stage A's [`segmented`](crate::segmented) foundation proved — over the
//! reference corpus — *where* a document can be cut into independently-lexable
//! spans. Stage B' carries that insight onto the owned AST: it caches the
//! owned lex output and, on an edit, re-lexes only the minimal balanced region
//! around the edit before splicing the owned node table.
//!
//! This module currently hosts the **region finder** ([`minimal_balanced_region`])
//! and the shared "where is it safe to cut the document" helpers that both the
//! Stage-A engine and this owned engine consume. The owned-table splice that
//! consumes the region arrives in a later PR; nothing here is wired to a
//! production consumer yet — it is internal and unit-tested.
//!
//! All coordinates here are **sanitized-source** byte offsets (the space every
//! [`OwnedLexOutput::source_span`](crate::SourceNodeOwned::source_span) and
//! [`OwnedLexOutput::pairs`](crate::OwnedLexOutput::pairs) indexes); the
//! raw↔sanitized bridge belongs to the later wiring PR. See the
//! [`segmented`](crate::segmented) module doc for why the cut is subtle.

use core::ops::Range;

use crate::{Diagnostic, NodeRefOwned, OwnedLexOutput, PairLink, SourceNodeOwned};

/// Whether `s` carries document structure that an incremental segment re-lex
/// must not silently absorb: a line terminator (could move a blank-line
/// boundary) or a directive opener `［` (could introduce a container or
/// forward reference, both whole-document-scoped concerns).
pub(crate) fn carries_structure(s: &str) -> bool {
    s.bytes().any(|b| b == b'\n' || b == b'\r') || s.contains('［')
}

/// `value + delta` as `usize`, or `None` on under/overflow.
pub(crate) fn shift_usize(value: u32, delta: i64) -> Option<usize> {
    usize::try_from(i64::from(value) + delta).ok()
}

/// `value + delta` clamped into `u32`, or `None` on under/overflow.
pub(crate) fn shift_u32(value: u32, delta: i64) -> Option<u32> {
    u32::try_from(i64::from(value) + delta).ok()
}

/// Whether a diagnostic's classification depends on the whole document and so
/// cannot be reliably computed from an isolated segment.
///
/// These are the parser's document-global checks, which a segment can get
/// wrong in *either* direction (a real diagnostic missed, or a phantom
/// invented), so they are never trusted per-segment and are taken wholesale
/// from the whole-document parse:
///
/// - **Forward-reference resolution** — bouten target ambiguity
///   ([`Diagnostic::BoutenTargetAmbiguous`], look-back
///   `source[..directive]`), 縦中横 target resolution
///   ([`Diagnostic::TcyTargetNotFound`]), standalone-gaiji forward resolution
///   ([`Diagnostic::UnresolvedGaiji`]), and directive recognition that
///   depends on a matching partner
///   ([`Diagnostic::UnrecognisedContainerDirective`]).
/// - **Container / kanbun / end-of-document pairing** — bracketed kaeriten
///   (返り点) whose partner may sit in a later segment
///   ([`Diagnostic::BracketedKaeritenNoPair`]), kaeriten whose enclosing
///   漢文 context spans segments ([`Diagnostic::KaeritenOutsideKanbun`]), and
///   container-close family mismatches
///   ([`Diagnostic::MismatchedContainerClose`],
///   [`Diagnostic::MismatchedBoutenContainer`]). Block-directive
///   classification is itself context-dependent — a deeply-nested
///   heading/indent structure (e.g. 論語-style repeated `中見出し` blocks) can
///   be classified as a container only with the whole-document context, so a
///   segment re-lexed in isolation pairs its closes differently and invents a
///   phantom mismatch.
///
/// Keep in sync with the `corpus_incremental_merge` gate's
/// `WHOLE_DOCUMENT_SCOPED` list (the gate fails if a new divergent class
/// appears, so completeness is enforced over the reference corpus).
pub(crate) fn is_whole_document_scoped(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic,
        Diagnostic::BoutenTargetAmbiguous { .. }
            | Diagnostic::TcyTargetNotFound { .. }
            | Diagnostic::UnresolvedGaiji { .. }
            | Diagnostic::UnrecognisedContainerDirective { .. }
            | Diagnostic::BracketedKaeritenNoPair { .. }
            | Diagnostic::KaeritenOutsideKanbun { .. }
            | Diagnostic::MismatchedContainerClose { .. }
            | Diagnostic::MismatchedBoutenContainer { .. }
    )
}

/// Candidate blank-line boundaries on the source: the byte offset of an
/// empty line that follows another line. Cutting there keeps a CRLF (`\r\n`)
/// terminator intact and starts the next segment on a blank line, matching
/// the whole-document decorative-rule isolation context.
pub(crate) fn candidate_boundaries(source: &str) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut cuts = Vec::new();
    let mut j = 1usize;
    while j < bytes.len() {
        if bytes[j - 1] == b'\n' {
            let empty_line_here = bytes[j] == b'\n'
                || (bytes[j] == b'\r' && j + 1 < bytes.len() && bytes[j + 1] == b'\n');
            if empty_line_here {
                cuts.push(j);
            }
        }
        j += 1;
    }
    cuts
}

/// Whether a cut at sanitized offset `san_off` keeps every block container
/// and resolved delimiter pair whole.
pub(crate) fn structurally_safe(
    san_off: u32,
    nodes: &[SourceNodeOwned],
    pairs: &[PairLink],
) -> bool {
    // Block-container nesting depth, via the same lenient LIFO the
    // normalizer uses (a stray close on an empty stack is ignored). Reject
    // the cut if a classified span strictly contains it, or depth is
    // non-zero at it.
    let mut depth: i32 = 0;
    for sn in nodes {
        if sn.source_span.start >= san_off {
            break; // nodes are sorted by source_span.start
        }
        if sn.source_span.end > san_off {
            return false; // a classified span straddles the cut
        }
        match sn.node {
            NodeRefOwned::BlockOpen(_) => depth += 1,
            NodeRefOwned::BlockClose(_) => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    if depth != 0 {
        return false;
    }
    // No resolved delimiter pair straddles the cut.
    !pairs
        .iter()
        .any(|pair| pair.open.start < san_off && pair.close.end > san_off)
}

/// The minimal region of `cached`'s sanitized buffer that must be re-lexed to
/// absorb an edit spanning sanitized byte range `edit`, bounded by
/// structurally-safe blank-line cuts. The returned range is in SANITIZED
/// coordinates, contains `edit`, and both endpoints are safe cut points
/// (document start/end, or a structurally-safe blank-line boundary), so the
/// region can be re-lexed in isolation without inventing phantom
/// unclosed/unmatched brackets or wrongly nesting a container.
///
/// Returns `None` when no sub-document benefit is provable:
/// - `cached` carries any whole-document-scoped diagnostic (forward-reference /
///   container pairing), which any edit can perturb beyond the region;
/// - `edit` is out of bounds (start > end, or end > sanitized length);
/// - the minimal safe region is the whole document (no interior safe cut).
// Consumed by the owned-table splice in a later #237 Stage B' PR; only the
// unit tests exercise it for now, so it reads as dead code in non-test builds.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "consumed by the owned-table splice in a later #237 Stage B' PR"
    )
)]
pub(crate) fn minimal_balanced_region(
    cached: &OwnedLexOutput,
    edit: Range<usize>,
) -> Option<Range<u32>> {
    if cached.diagnostics.iter().any(is_whole_document_scoped) {
        return None;
    }
    let san = &cached.sanitized;
    let len = u32::try_from(san.len()).ok()?;
    if edit.start > edit.end || edit.end > san.len() {
        return None;
    }
    let es = u32::try_from(edit.start).ok()?;
    let ee = u32::try_from(edit.end).ok()?;

    // Safe cut points in sanitized coordinates, ascending. Document ends are
    // always safe (depth 0, no straddle); interior blank-line boundaries are
    // admitted only where they keep every container and pair whole.
    let mut cuts: Vec<u32> = Vec::new();
    cuts.push(0);
    for b in candidate_boundaries(san) {
        let Ok(b_u32) = u32::try_from(b) else {
            continue;
        };
        if b_u32 != 0
            && b_u32 != len
            && structurally_safe(b_u32, &cached.source_nodes, &cached.pairs)
        {
            cuts.push(b_u32);
        }
    }
    cuts.push(len);

    // `candidate_boundaries` returns ascending offsets, and 0/len bracket
    // them, so `cuts` is already sorted. The greatest cut <= es and the least
    // cut >= ee both exist because 0 <= es <= ee <= len.
    let region_start = cuts.iter().copied().filter(|&c| c <= es).max()?;
    let region_end = cuts.iter().copied().filter(|&c| c >= ee).min()?;

    if region_start == 0 && region_end == len {
        return None; // whole document — no benefit
    }
    Some(region_start..region_end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// Parse `src` to a real owned lex output.
    fn owned(src: &str) -> OwnedLexOutput {
        Document::new(src).parse_owned()
    }

    /// The full ascending safe-cut set the region finder works over, for
    /// asserting endpoint membership.
    fn safe_cuts(cached: &OwnedLexOutput) -> Vec<u32> {
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        let mut cuts = vec![0u32];
        for b in candidate_boundaries(&cached.sanitized) {
            let b = u32::try_from(b).unwrap();
            if b != 0 && b != len && structurally_safe(b, &cached.source_nodes, &cached.pairs) {
                cuts.push(b);
            }
        }
        cuts.push(len);
        cuts
    }

    fn assert_endpoint_safe(cached: &OwnedLexOutput, off: u32) {
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        assert!(
            off == 0 || off == len || structurally_safe(off, &cached.source_nodes, &cached.pairs),
            "endpoint {off} must be a safe cut (0/len or structurally safe)"
        );
    }

    #[test]
    fn edit_inside_paragraph_shrinks_below_whole_doc() {
        // Three blank-line-separated paragraphs, all plain text.
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        // Edit inside the middle paragraph "かきくけこ".
        let mid = src.find("かきくけこ").unwrap();
        let edit = mid..mid + "かき".len();
        let region = minimal_balanced_region(&cached, edit.clone()).expect("interior region");
        // Strictly smaller than the whole document.
        assert!(
            region.start > 0 || region.end < len,
            "region {region:?} must be strictly smaller than 0..{len}"
        );
        assert_ne!(region, 0..len);
        // Contains the edit.
        assert!(region.start as usize <= edit.start && region.end as usize >= edit.end);
        // Both endpoints are safe cuts.
        assert_endpoint_safe(&cached, region.start);
        assert_endpoint_safe(&cached, region.end);
        let cuts = safe_cuts(&cached);
        assert!(cuts.contains(&region.start), "start in safe-cut set");
        assert!(cuts.contains(&region.end), "end in safe-cut set");
    }

    #[test]
    fn single_paragraph_has_no_interior_cut() {
        let src = "あいうえおかきくけこ\n";
        let cached = owned(src);
        let edit = 3..6;
        assert_eq!(minimal_balanced_region(&cached, edit), None);
    }

    #[test]
    fn whole_document_scoped_diagnostic_yields_none() {
        // An unresolved standalone gaiji reference is whole-document-scoped.
        let src = "前の段落\n\n※［＃存在しない外字、第1水準1-2-3］\n\n後の段落\n";
        let cached = owned(src);
        assert!(
            cached.diagnostics.iter().any(is_whole_document_scoped),
            "fixture must carry a whole-document-scoped diagnostic, got {:?}",
            cached.diagnostics
        );
        // Region declines regardless of where the edit sits.
        assert_eq!(minimal_balanced_region(&cached, 0..1), None);
        let mid = src.find("後の段落").unwrap();
        assert_eq!(minimal_balanced_region(&cached, mid..mid + 3), None);
    }

    #[test]
    fn out_of_bounds_edit_yields_none() {
        let src = "あいうえお\n\nかきくけこ\n";
        let cached = owned(src);
        let len = cached.sanitized.len();
        // end past sanitized length.
        assert_eq!(minimal_balanced_region(&cached, 0..len + 10), None);
        // start > end (built without a literal reversed range to satisfy the
        // reversed_empty_ranges lint).
        let reversed = Range {
            start: 5usize,
            end: 2,
        };
        assert_eq!(minimal_balanced_region(&cached, reversed), None);
    }

    #[test]
    fn edit_spanning_blank_line_widens_to_both_paragraphs() {
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        // Edit straddles the blank line between paragraph 1 and 2.
        let p1 = src.find("うえお").unwrap();
        let p2_end = src.find("かきく").unwrap() + "かきく".len();
        let edit = p1..p2_end;
        let region = minimal_balanced_region(&cached, edit.clone()).expect("region");
        // Must contain the whole straddled range, hence both flanking paragraphs.
        assert!(region.start as usize <= edit.start);
        assert!(region.end as usize >= edit.end);
        // Region must include the entire first paragraph text and the second.
        assert!((region.start as usize) <= src.find("あいうえお").unwrap());
        assert!((region.end as usize) >= src.find("かきくけこ").unwrap() + "かきくけこ".len());
        assert_endpoint_safe(&cached, region.start);
        assert_endpoint_safe(&cached, region.end);
    }

    #[test]
    fn crlf_source_region_is_in_sanitized_coordinates() {
        // CRLF source (as real Aozora Bunko files are). Sanitize strips the
        // \r, so sanitized offsets are smaller than raw offsets.
        let src = "あいうえお\r\n\r\nかきくけこ\r\n\r\nさしすせそ\r\n";
        let cached = owned(src);
        let san = &cached.sanitized;
        assert!(!san.contains('\r'), "sanitized buffer drops CR");
        let len = u32::try_from(san.len()).unwrap();
        // Edit the middle paragraph in SANITIZED coordinates.
        let mid = san.find("かきくけこ").unwrap();
        let edit = mid..mid + "かき".len();
        let region = minimal_balanced_region(&cached, edit.clone()).expect("region");
        assert_ne!(region, 0..len);
        assert!(region.start as usize <= edit.start && region.end as usize >= edit.end);
        assert_endpoint_safe(&cached, region.start);
        assert_endpoint_safe(&cached, region.end);
        // The region must not reach into the third paragraph.
        let p3 = san.find("さしすせそ").unwrap();
        assert!(
            (region.end as usize) <= p3,
            "region {region:?} must not include paragraph 3 at {p3}"
        );
    }

    #[test]
    fn crlf_single_paragraph_yields_none() {
        let src = "あいうえおかきくけこ\r\n";
        let cached = owned(src);
        assert_eq!(minimal_balanced_region(&cached, 0..3), None);
    }

    #[test]
    fn empty_document_yields_none() {
        let cached = owned("");
        assert_eq!(cached.sanitized.len(), 0, "empty source sanitizes to empty");
        // Zero-width edit on the empty document: the region is 0..0 == 0..len,
        // i.e. the whole (empty) document, so there is no sub-document benefit.
        let zero = 0usize;
        assert_eq!(minimal_balanced_region(&cached, zero..zero), None);
    }

    #[test]
    fn boundary_landing_edits_return_empty_safe_regions() {
        // A zero-width edit at a document end or exactly on an interior safe
        // cut yields the minimal empty region pinned to that offset — a genuine
        // cut, so the PR3 splice can re-lex the inserted text from a clean
        // boundary. (Edits go through a variable to avoid the
        // `reversed_empty_ranges` lint on literal equal-bound ranges.)
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        let at_offset = |at: usize| minimal_balanced_region(&cached, at..at);

        // Document start and end.
        assert_eq!(at_offset(0), Some(Range { start: 0, end: 0 }));
        assert_eq!(
            at_offset(len as usize),
            Some(Range {
                start: len,
                end: len
            }),
        );

        // Exactly on an interior safe cut (a blank-line boundary).
        let interior = safe_cuts(&cached)
            .into_iter()
            .find(|&c| c != 0 && c != len)
            .expect("a three-paragraph doc has an interior safe cut");
        assert_eq!(
            at_offset(interior as usize),
            Some(Range {
                start: interior,
                end: interior,
            }),
        );
    }
}
