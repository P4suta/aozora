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

/// A node's standalone-block padding (the `\n\n` inserted *before* its sentinel,
/// equal to the `\n\n` inserted *after*): `2` for a standalone block node, `0`
/// for an inline one. The normalizer pads only block-level nodes; an inline
/// region (傍点 / bare-range 太字 / 縦中横 / …) and an inline open/close get no
/// padding. Mirrors the normalize-stage rule that drives the `\n\n` + `<div>`
/// wrapping (see [`crate::RegionFormat::is_inline`] /
/// [`crate::RegionClose::is_inline`]).
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "consumed by the owned-table splice in a later #237 Stage B' PR"
    )
)]
fn standalone_pad(node: NodeRefOwned) -> u32 {
    match node {
        NodeRefOwned::BlockLeaf(_) => 2,
        NodeRefOwned::BlockOpen(rf) => u32::from(!rf.is_inline()) * 2,
        NodeRefOwned::BlockClose(rc) => u32::from(!rc.is_inline()) * 2,
        // [`NodeRefOwned::Inline`] gets no padding. The wildcard (mandatory —
        // `NodeRefOwned` is `#[non_exhaustive]`) also defaults any future
        // sentinel kind to no padding, the inline byte-1:1 assumption and the
        // conservative choice for the offset map.
        NodeRefOwned::Inline(_) | _ => 0,
    }
}

/// The normalized-text byte offset corresponding to sanitized-source offset
/// `san_off`. `san_off` must be a structurally-safe interstitial boundary (0,
/// sanitized_len, or a blank-line cut that no node's source_span straddles) —
/// exactly the boundaries [`minimal_balanced_region`] returns. At such a
/// position normalized == sanitized locally (plain text is 1:1); the only
/// divergence is the accumulated PUA sentinels (3 bytes each) plus standalone-
/// block "\n\n" padding (2 bytes lead + 2 trail) inserted before `san_off`.
///
/// The map is registry-free and closed-form. For every node fully before the
/// boundary (`source_span.end <= san_off`), the normalized stream replaced its
/// `footprint = end - start` sanitized bytes with `2·pad + 3` bytes (lead pad +
/// 3-byte sentinel + trail pad), a drift of `Δ = (2·pad + 3) − footprint`.
/// Summing `Δ` over those nodes and adding it to `san_off` lands the normalized
/// cursor, because plain runs between nodes are byte-identical. A node whose
/// `source_span.start == san_off` has `end > san_off`, so it sits *after* the
/// boundary and is correctly excluded.
///
/// Returns `None` if `san_off` exceeds the sanitized length, if the arithmetic
/// overflows, or (defensive tripwire) if the computed offset is not a char
/// boundary of `cached.normalized` — the boundary-never-in-padding proof means
/// this never fires for a valid interstitial boundary, but it converts any
/// surprise into a clean fallback rather than a bad splice.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "consumed by the owned-table splice in a later #237 Stage B' PR"
    )
)]
pub(crate) fn norm_offset(cached: &OwnedLexOutput, san_off: u32) -> Option<u32> {
    let san_len = u32::try_from(cached.sanitized.len()).ok()?;
    if san_off > san_len {
        return None;
    }

    // Nodes fully before the boundary, in source order (`source_nodes` is sorted
    // by `source_span.start`, and an interstitial boundary is never straddled,
    // so `end <= san_off` partitions cleanly).
    let k = cached
        .source_nodes
        .partition_point(|sn| sn.source_span.end <= san_off);

    let mut drift: i64 = 0;
    for sn in &cached.source_nodes[..k] {
        let footprint = i64::from(sn.source_span.end - sn.source_span.start);
        let pad = i64::from(standalone_pad(sn.node));
        drift += 2 * pad + 3 - footprint;
    }

    let norm = shift_u32(san_off, drift)?;
    // Defensive tripwire: a valid interstitial boundary never lands inside a
    // sentinel or padding run, so the result is always a char boundary; if it
    // somehow is not, decline rather than splice at a bad offset.
    if !cached.normalized.is_char_boundary(norm as usize) {
        return None;
    }
    Some(norm)
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

    // ---- norm_offset (sanitized → normalized offset map) ----

    use crate::{BoutenKind, BoutenPosition, RegionClose, RegionFormat};

    /// The registry as a `(position, NodeRefOwned)` vec, parallel to
    /// `source_nodes` (same source order — the established invariant).
    fn reg_entries(cached: &OwnedLexOutput) -> Vec<(u32, NodeRefOwned)> {
        cached.registry.iter_sorted().collect()
    }

    /// `norm_offset(0) == 0` and `norm_offset(sanitized_len) == normalized_len`
    /// — the whole accumulated drift lands the document end exactly.
    fn assert_endpoints(src: &str) {
        let cached = owned(src);
        let san_len = u32::try_from(cached.sanitized.len()).unwrap();
        let norm_len = u32::try_from(cached.normalized.len()).unwrap();
        assert_eq!(norm_offset(&cached, 0), Some(0), "start of {src:?}");
        assert_eq!(
            norm_offset(&cached, san_len),
            Some(norm_len),
            "end of {src:?}",
        );
    }

    /// THE key cross-check: for every node, the sanitized boundary just after
    /// its span maps to the normalized cursor just after its sentinel (`+3`)
    /// plus trailing standalone padding — derived straight from the registry
    /// positions the pipeline actually produced. Skips any boundary another
    /// node straddles (it would not be a clean interstitial point).
    fn assert_registry_ground_truth(src: &str) {
        let cached = owned(src);
        let reg = reg_entries(&cached);
        let nodes = &cached.source_nodes;
        assert_eq!(
            reg.len(),
            nodes.len(),
            "registry parallel to source_nodes for {src:?}",
        );
        let mut checked = 0usize;
        for k in 1..=nodes.len() {
            let b = nodes[k - 1].source_span.end;
            // A clean interstitial boundary is straddled by no node.
            if nodes
                .iter()
                .any(|sn| sn.source_span.start < b && sn.source_span.end > b)
            {
                continue;
            }
            let expected = reg[k - 1].0 + 3 + standalone_pad(reg[k - 1].1);
            assert_eq!(
                norm_offset(&cached, b),
                Some(expected),
                "node {} boundary {b} in {src:?}",
                k - 1,
            );
            checked += 1;
        }
        assert!(
            nodes.is_empty() || checked > 0,
            "no clean boundary checked for {src:?}",
        );
    }

    /// Structurally diverse documents that exercise every padding case: plain
    /// (no node), inline ruby (pad 0, long base collapses), inline forward
    /// format, a standalone block leaf (改ページ, pad 2), an inline gaiji
    /// reference, and a block container open/close (字下げ, pad 2 each).
    const GROUND_TRUTH_DOCS: &[&str] = &[
        "あいうえお\n\nかきくけこ\n",
        "前｜漢字《かんじ》後\n",
        "前\n\n｜山《やま》\n\n後\n",
        "あ［＃「あ」は太字］い\n",
        "前\n\n［＃改ページ］\n\n後\n",
        "海※［＃感嘆符二つ、1-8-75］辺\n",
        "前\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］\n\n後\n",
    ];

    #[test]
    fn norm_offset_endpoints_account_for_all_drift() {
        for src in GROUND_TRUTH_DOCS {
            assert_endpoints(src);
        }
        // Empty document: 0 maps to 0, both lengths zero.
        let empty = owned("");
        assert_eq!(norm_offset(&empty, 0), Some(0));
    }

    #[test]
    fn norm_offset_matches_registry_ground_truth() {
        for src in GROUND_TRUTH_DOCS {
            assert_registry_ground_truth(src);
        }
    }

    #[test]
    fn norm_offset_standalone_block_includes_lead_and_trail_padding() {
        // 改ページ is a standalone block leaf: pad 2 (lead) + 3 sentinel + 2
        // (trail). The boundary after it must skip the trailing pad too.
        let src = "前\n\n［＃改ページ］\n\n後\n";
        let cached = owned(src);
        let idx = cached
            .source_nodes
            .iter()
            .position(|sn| matches!(sn.node, NodeRefOwned::BlockLeaf(_)))
            .expect("改ページ is a block leaf");
        assert_eq!(
            standalone_pad(cached.source_nodes[idx].node),
            2,
            "standalone block leaf pads 2",
        );
        let reg = reg_entries(&cached);
        let b = cached.source_nodes[idx].source_span.end;
        assert_eq!(
            norm_offset(&cached, b),
            Some(reg[idx].0 + 3 + 2),
            "cursor sits after sentinel + trailing pad",
        );
    }

    #[test]
    fn norm_offset_block_container_pads_open_and_close() {
        // 字下げ container: BlockOpen + BlockClose, each block (pad 2).
        let src = "前\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］\n\n後\n";
        let cached = owned(src);
        let opens: Vec<_> = cached
            .source_nodes
            .iter()
            .filter(|sn| matches!(sn.node, NodeRefOwned::BlockOpen(_)))
            .collect();
        let closes: Vec<_> = cached
            .source_nodes
            .iter()
            .filter(|sn| matches!(sn.node, NodeRefOwned::BlockClose(_)))
            .collect();
        assert_eq!(opens.len(), 1, "one container open");
        assert_eq!(closes.len(), 1, "one container close");
        assert_eq!(standalone_pad(opens[0].node), 2, "block open pads 2");
        assert_eq!(standalone_pad(closes[0].node), 2, "block close pads 2");
        assert_registry_ground_truth(src);
    }

    #[test]
    fn standalone_pad_table_inline_vs_block() {
        // Inline open/close (傍点 range, is_inline) get no padding; block
        // open/close (罫囲み) pad 2. Constructed directly to pin the table
        // independent of which directives the parser happens to emit inline.
        let inline_open = NodeRefOwned::BlockOpen(RegionFormat::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Right,
        });
        let inline_close = NodeRefOwned::BlockClose(RegionClose::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Right,
        });
        let block_open = NodeRefOwned::BlockOpen(RegionFormat::Framed);
        let block_close = NodeRefOwned::BlockClose(RegionClose::Framed);
        assert_eq!(standalone_pad(inline_open), 0, "inline open: no pad");
        assert_eq!(standalone_pad(inline_close), 0, "inline close: no pad");
        assert_eq!(standalone_pad(block_open), 2, "block open: pad 2");
        assert_eq!(standalone_pad(block_close), 2, "block close: pad 2");
    }

    #[test]
    fn norm_offset_crlf_source_is_in_sanitized_coordinates() {
        // Sanitize strips \r first, so source_nodes / normalized already live
        // in sanitized space; norm_offset operates entirely there.
        let src = "前\r\n\r\n［＃改ページ］\r\n\r\n後\r\n";
        let cached = owned(src);
        assert!(!cached.sanitized.contains('\r'), "sanitized drops CR");
        assert_endpoints(src);
        assert_registry_ground_truth(src);
    }

    #[test]
    fn norm_offset_interior_gap_matches_bracketing_form() {
        // An interior boundary in the MIDDLE of a plain gap (not exactly at a
        // node end): the cumulative form must equal the bracketing form
        // `reg[k-1].0 + 3 + pad + (b - source_span.end)` — plain text is 1:1.
        let src = "前\n\n［＃改ページ］\n\n後の段落です\n";
        let cached = owned(src);
        let reg = reg_entries(&cached);
        // The single node is the page break; pick a boundary a few bytes into
        // the trailing "後の段落です" plain run, well past its span end.
        let node_end = cached.source_nodes[0].source_span.end;
        let after = cached.sanitized.find("後の段落").unwrap();
        let b = u32::try_from(after + "後".len()).unwrap();
        assert!(b > node_end, "boundary sits in the gap after the node");
        let pad = standalone_pad(cached.source_nodes[0].node);
        let bracketing = reg[0].0 + 3 + pad + (b - node_end);
        assert_eq!(
            norm_offset(&cached, b),
            Some(bracketing),
            "cumulative form equals bracketing form in a plain gap",
        );
    }

    #[test]
    fn norm_offset_out_of_bounds_yields_none() {
        let cached = owned("あいうえお\n");
        let san_len = u32::try_from(cached.sanitized.len()).unwrap();
        assert_eq!(norm_offset(&cached, san_len + 1), None, "past the end");
    }

    #[test]
    fn norm_offset_mid_codepoint_yields_none() {
        // The defensive char-boundary tripwire: a san_off that lands inside a
        // multi-byte codepoint maps to a non-char-boundary normalized offset
        // and must decline (→ caller falls back) rather than produce a
        // mid-codepoint splice point. "あ" is 3 bytes, so byte 1 is interior.
        let cached = owned("あ\n");
        assert!(
            !cached.sanitized.is_char_boundary(1),
            "byte 1 is mid-codepoint in the sanitized buffer",
        );
        assert_eq!(
            norm_offset(&cached, 1),
            None,
            "mid-codepoint offset declines"
        );
    }

    #[test]
    fn norm_offset_no_node_interior_is_identity() {
        // A document with no classified nodes has zero drift, so norm_offset is
        // the identity at every char boundary (normalized == sanitized).
        let cached = owned("あいうえお\n");
        assert!(cached.source_nodes.is_empty(), "plain text has no nodes");
        assert_eq!(cached.normalized, cached.sanitized, "no sentinels inserted");
        for b in 0..=cached.sanitized.len() {
            if cached.sanitized.is_char_boundary(b) {
                let off = u32::try_from(b).unwrap();
                assert_eq!(norm_offset(&cached, off), Some(off), "identity at {b}");
            }
        }
    }
}
