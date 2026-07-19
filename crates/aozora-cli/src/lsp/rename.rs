//! `textDocument/{prepareRename,rename}` — the LSP face of the #202
//! splice engine.
//!
//! Renaming one site of an aozora *coupling* edits its partner
//! coherently: a container open marker and its matching close, or a
//! forward-reference / heading-hint / margin-note directive and the
//! upstream literal it points at. This module is a thin surface over the
//! existing splice engine ([`aozora::Snapshot::splice`] /
//! [`aozora::Snapshot::coupling`]),
//! which is the single source of truth for "what couples to what". No
//! partner-derivation is reimplemented here — we *call* the splice and
//! *recover* its two-region edit from its own output.
//!
//! ## What the user renames
//!
//! prepareRename returns the span of the **whole** primary marker / bracket
//! (the `［＃…］` directive or the `［＃ここから…］` open marker), and the
//! `new_name` the client sends back maps 1:1 to the splice's `replacement`.
//! Exposing a narrower inner token would force us to duplicate splice's
//! private `first_quoted` partner-derivation — exactly what this layer must
//! not do.
//!
//! ## The coordinate gate
//!
//! The tree and the splice work in **sanitized** source coordinates; the
//! LSP document text and [`LineIndex`] are **raw**. Every operation is gated
//! on `tree.sanitized() == text`: when it holds, raw == sanitized, the two
//! coordinate spaces coincide, and every [`aozora::Span`] plus the spliced
//! `new_source` are in document coordinates. When the gate fails (CRLF, BOM,
//! PUA, or a stale cache) we return `None` rather than risk an incorrectly
//! mapped edit — the same invariant the incremental-diagnostics path uses.

use std::collections::HashMap;

use aozora::{Coupling, Region, Snapshot, SourceOffset, Span, SpliceError, SpliceSafety};
use tower_lsp::lsp_types::{Position, PrepareRenameResponse, Range, TextEdit, Url, WorkspaceEdit};

use crate::lsp::line_index::LineIndex;

/// `prepareRename`: `Some(range + placeholder)` when `position` sits on a
/// coupled region, else `None`.
///
/// Returns `None` for a `Direct` or `Opaque` region (nothing to couple), for
/// a position past the end of the document, or when the coordinate gate
/// (`tree.sanitized() == text`) does not hold.
#[must_use]
pub(super) fn prepare_rename_at(
    tree: &Snapshot,
    text: &str,
    line_index: &LineIndex,
    position: Position,
) -> Option<PrepareRenameResponse> {
    if tree.sanitized() != text {
        return None;
    }
    let off = line_index.byte_offset(text, position)?;
    let region = tree.region_at(SourceOffset::new(u32::try_from(off).ok()?))?;
    if !matches!(region.safety, SpliceSafety::Coupled(_)) {
        return None;
    }
    let placeholder = text
        .get(region.span.start as usize..region.span.end as usize)?
        .to_owned();
    Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: span_to_range(line_index, text, region.span),
        placeholder,
    })
}

/// `rename`: the coherent coupled edit for renaming the region at `position`
/// to `new_name`.
///
/// - `Ok(Some(edit))` — a coherent coupled edit.
/// - `Ok(None)` — the position is not on a coupled region, the gate failed,
///   or the splice was a no-op (the new text equals the old).
/// - `Err(e)` — the splice engine *declined* the edit (an ambiguous referent,
///   a ruby-base target literal, a 、-joined multi-target, or an opaque node).
///
/// `new_name` is passed **verbatim** to [`Snapshot::splice`] as the full new region
/// text; splice is the authority and derives the partner change itself. The
/// emitted [`WorkspaceEdit`] is then *recovered* from splice's own output — we
/// never re-derive the partner.
///
/// # Errors
///
/// Propagates [`SpliceError`] when the underlying [`Snapshot::splice`] declines the
/// coupled edit.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the LSP rename request shape (document, position, new name) \
              over the cached tree + line index; bundling would obscure the surface"
)]
pub(super) fn rename_edit(
    tree: &Snapshot,
    text: &str,
    line_index: &LineIndex,
    uri: &Url,
    position: Position,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>, SpliceError> {
    if tree.sanitized() != text {
        return Ok(None);
    }
    let Some(off) = line_index.byte_offset(text, position) else {
        return Ok(None);
    };
    let Ok(raw_off) = u32::try_from(off) else {
        return Ok(None);
    };
    let Some(region) = tree.region_at(SourceOffset::new(raw_off)) else {
        return Ok(None);
    };
    if !matches!(region.safety, SpliceSafety::Coupled(_)) {
        return Ok(None);
    }

    // The splice engine is the single source of truth: it derives the partner,
    // verifies by re-parse, and returns Err when it cannot prove the edit
    // coherent. `new_name` is the full new region text.
    let new_source = tree.splice(region, new_name)?;
    if new_source == text {
        return Ok(None);
    }

    let edits = edits_for_splice(tree, line_index, text, &new_source, region, new_name);
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

/// Recover the minimal [`TextEdit`]s for a successful splice, *without*
/// re-deriving the partner: the primary's new bytes are exactly `new_name`,
/// and the partner's new bytes are read out of `new_source` by arithmetic
/// (splice changed only the two disjoint regions).
#[expect(
    clippy::too_many_arguments,
    reason = "passes the splice inputs and outputs through verbatim; a wrapper \
              struct would add a type without reducing the data threaded"
)]
fn edits_for_splice(
    tree: &Snapshot,
    line_index: &LineIndex,
    text: &str,
    new_source: &str,
    region: Region,
    new_name: &str,
) -> Vec<TextEdit> {
    match tree.coupling(region) {
        // Splice succeeded as a single-region change (an attribute-only edit
        // whose partner is unchanged, or a marker with no locatable partner):
        // the primary edit alone reproduces it.
        None => vec![text_edit(line_index, text, region.span, new_name)],
        Some(Coupling {
            primary: p,
            partner: q,
            ..
        }) => {
            recover_two_edits(line_index, text, new_source, p, q, new_name).unwrap_or_else(|| {
                // Robustness: if the arithmetic recovery is out of bounds, fall
                // back to a single minimal-diff edit. Correctness over minimality;
                // the round-trip test guards both paths.
                vec![minimal_diff_edit(line_index, text, new_source)]
            })
        }
    }
}

/// Recover the primary + partner edits arithmetically. Splice rewrote only the
/// disjoint regions `p` (the queried marker, now exactly `new_name`) and `q`
/// (its partner); every other byte is identical, so the partner's new text is
/// the slice of `new_source` at the partner's shifted offset.
///
/// Returns `None` if the computed partner slice falls out of bounds — the
/// caller then uses the minimal-diff fallback.
#[expect(
    clippy::too_many_arguments,
    reason = "the two spans plus the before/after texts are all load-bearing \
              inputs to the disjoint-region arithmetic"
)]
fn recover_two_edits(
    line_index: &LineIndex,
    text: &str,
    new_source: &str,
    p: Span,
    q: Span,
    new_name: &str,
) -> Option<Vec<TextEdit>> {
    let total_delta = i64::try_from(new_source.len()).ok()? - i64::try_from(text.len()).ok()?;
    let primary_delta = i64::try_from(new_name.len()).ok()? - i64::from(p.end - p.start);
    let partner_delta = total_delta - primary_delta;

    let q_new_len = i64::from(q.end - q.start) + partner_delta;
    // The partner shifts by the primary's length change only when it sits
    // *after* the primary; a partner before the primary keeps its start.
    let q_new_start = if q.start > p.start {
        i64::from(q.start) + primary_delta
    } else {
        i64::from(q.start)
    };
    if q_new_start < 0 || q_new_len < 0 {
        return None;
    }
    let start = usize::try_from(q_new_start).ok()?;
    let len = usize::try_from(q_new_len).ok()?;
    let q_new = new_source.get(start..start.checked_add(len)?)?;

    let mut edits = vec![text_edit(line_index, text, p, new_name)];
    // Emit the partner edit only when its bytes actually changed (an
    // attribute-only / amount-only splice leaves the partner verbatim).
    let q_old = text.get(q.start as usize..q.end as usize)?;
    if q_new != q_old {
        edits.push(text_edit(line_index, text, q, q_new));
    }
    Some(edits)
}

/// A single whole-region minimal-diff edit: trim the common prefix and suffix
/// (snapped to UTF-8 boundaries) of `text` vs `new_source` and replace the
/// differing middle. Correct for any splice output, used as the robustness
/// fallback when the two-region arithmetic cannot be applied.
fn minimal_diff_edit(line_index: &LineIndex, text: &str, new_source: &str) -> TextEdit {
    let old = text.as_bytes();
    let new = new_source.as_bytes();

    // Common prefix, snapped down to a char boundary in `text` (equal bytes
    // mean the same boundary holds in `new_source`).
    let mut pre = 0;
    let max_pre = old.len().min(new.len());
    while pre < max_pre && old[pre] == new[pre] {
        pre += 1;
    }
    while pre > 0 && !text.is_char_boundary(pre) {
        pre -= 1;
    }

    // Common suffix of the post-prefix tails (bounded by the shorter tail),
    // snapped so `text.len() - suf` is a char boundary in `text`.
    let mut suf = old[pre..]
        .iter()
        .rev()
        .zip(new[pre..].iter().rev())
        .take_while(|(o, n)| o == n)
        .count();
    while suf > 0 && !text.is_char_boundary(text.len() - suf) {
        suf -= 1;
    }

    let old_end = text.len() - suf;
    let new_end = new_source.len() - suf;
    let new_text = new_source.get(pre..new_end).unwrap_or(new_source);
    let span = Span::new(
        u32::try_from(pre).unwrap_or(u32::MAX),
        u32::try_from(old_end).unwrap_or(u32::MAX),
    );
    text_edit(line_index, text, span, new_text)
}

/// Convert a sanitized-coordinate [`Span`] (== document coordinate under the
/// gate) into an LSP [`Range`].
fn span_to_range(line_index: &LineIndex, text: &str, span: Span) -> Range {
    Range::new(
        line_index.position(text, span.start as usize),
        line_index.position(text, span.end as usize),
    )
}

/// Build a [`TextEdit`] replacing `span` with `new`.
fn text_edit(line_index: &LineIndex, text: &str, span: Span, new: &str) -> TextEdit {
    TextEdit {
        range: span_to_range(line_index, text, span),
        new_text: new.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Reverse;

    use super::*;

    fn fake_uri() -> Url {
        Url::parse("file:///fake.aozora").expect("valid URL")
    }

    /// Position of the byte offset `off` in `src`.
    fn pos(src: &str, off: usize) -> Position {
        LineIndex::new(src).position(src, off)
    }

    /// Apply `edits` (disjoint, against the original document) to `text`,
    /// returning the result. Edits are applied back-to-front so earlier byte
    /// offsets stay valid.
    fn apply_text_edits(text: &str, line_index: &LineIndex, edits: &[TextEdit]) -> String {
        let mut spans: Vec<(usize, usize, String)> = edits
            .iter()
            .map(|e| {
                let s = line_index
                    .byte_offset(text, e.range.start)
                    .expect("start in range");
                let en = line_index
                    .byte_offset(text, e.range.end)
                    .expect("end in range");
                (s, en, e.new_text.clone())
            })
            .collect();
        spans.sort_by_key(|s| Reverse(s.0));
        let mut out = text.to_owned();
        for (s, en, t) in spans {
            out.replace_range(s..en, &t);
        }
        out
    }

    /// Convenience: parse `src`, run `prepare_rename_at` at byte offset `off`.
    fn prepare_at(src: &str, off: usize) -> Option<PrepareRenameResponse> {
        let tree = aozora::parse(src).snapshot();
        prepare_rename_at(&tree, src, &LineIndex::new(src), pos(src, off))
    }

    /// Convenience: parse `src`, run `rename_edit` at byte offset `off`.
    fn rename_at(
        src: &str,
        off: usize,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, SpliceError> {
        let tree = aozora::parse(src).snapshot();
        rename_edit(
            &tree,
            src,
            &LineIndex::new(src),
            &fake_uri(),
            pos(src, off),
            new_name,
        )
    }

    /// The single change-list of a workspace edit.
    fn changes(edit: &WorkspaceEdit) -> &Vec<TextEdit> {
        edit.changes
            .as_ref()
            .expect("changes present")
            .values()
            .next()
            .expect("one uri")
    }

    // ---- prepareRename -------------------------------------------------

    #[test]
    fn prepare_on_container_open_is_renameable() {
        let src = "［＃ここから2字下げ］本文\n";
        let off = src.find('［').expect("open marker");
        let resp = prepare_at(src, off).expect("container open is coupled");
        let PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = resp else {
            panic!("expected RangeWithPlaceholder, got {resp:?}");
        };
        assert_eq!(placeholder, "［＃ここから2字下げ］");
    }

    #[test]
    fn prepare_on_forward_reference_is_renameable() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let off = src.rfind('［').expect("directive bracket");
        let resp = prepare_at(src, off).expect("forward reference is coupled");
        let PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = resp else {
            panic!("expected RangeWithPlaceholder");
        };
        assert_eq!(placeholder, "［＃「青空」に傍点］");
    }

    #[test]
    fn prepare_on_plain_prose_is_not_renameable() {
        let src = "ただの本文です。";
        assert!(prepare_at(src, 3).is_none());
    }

    #[test]
    fn prepare_on_ruby_is_not_renameable() {
        // Ruby is Direct (self-contained), so it is not a coupled rename site.
        let src = "日本《にほん》";
        let off = src.find('《').expect("ruby reading");
        assert!(prepare_at(src, off).is_none());
    }

    #[test]
    fn prepare_past_eof_is_none() {
        let src = "本文";
        // A position on a line past the buffer maps to no byte offset.
        let tree = aozora::parse(src).snapshot();
        assert!(prepare_rename_at(&tree, src, &LineIndex::new(src), Position::new(9, 0)).is_none());
    }

    #[test]
    fn prepare_crlf_doc_fails_the_gate() {
        // sanitize folds CRLF→LF, so tree.sanitized() != text — the gate
        // returns None rather than risk an incorrectly mapped edit.
        let src = "前\r\n［＃ここから2字下げ］\r\n本文\r\n［＃ここで字下げ終わり］\r\n後";
        let off = src.find('［').expect("open marker");
        assert!(prepare_at(src, off).is_none());
    }

    // ---- rename: container ---------------------------------------------

    const CONTAINER_DOC: &str = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";

    #[test]
    fn rename_container_family_change_rewrites_both_markers() {
        let off = CONTAINER_DOC.find('［').expect("open marker");
        let edit = rename_at(CONTAINER_DOC, off, "［＃ここから罫囲み］")
            .expect("kind change is verifiable")
            .expect("edit emitted");
        let applied = apply_text_edits(
            CONTAINER_DOC,
            &LineIndex::new(CONTAINER_DOC),
            changes(&edit),
        );
        assert!(applied.contains("［＃ここから罫囲み］"));
        assert!(applied.contains("罫囲み終わり"));
        assert!(!applied.contains("字下げ"));
        // Two markers changed → two edits.
        assert_eq!(changes(&edit).len(), 2);
    }

    #[test]
    fn rename_container_amount_change_touches_only_the_open() {
        let off = CONTAINER_DOC.find('［').expect("open marker");
        let edit = rename_at(CONTAINER_DOC, off, "［＃ここから4字下げ］")
            .expect("amount change is verifiable")
            .expect("edit emitted");
        // The close keyword (字下げ終わり) carries no amount, so it is verbatim:
        // exactly one edit on the open.
        assert_eq!(changes(&edit).len(), 1);
        let applied = apply_text_edits(
            CONTAINER_DOC,
            &LineIndex::new(CONTAINER_DOC),
            changes(&edit),
        );
        assert!(applied.contains("［＃ここから4字下げ］"));
        assert!(applied.contains("［＃ここで字下げ終わり］"));
    }

    // ---- rename: forward reference -------------------------------------

    #[test]
    fn rename_forward_target_change_edits_literal_and_bracket() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let off = src.rfind('［').expect("directive bracket");
        let edit = rename_at(src, off, "［＃「海」に傍点］")
            .expect("target change is coupled")
            .expect("edit emitted");
        // Two regions: the upstream literal + the bracket.
        assert_eq!(changes(&edit).len(), 2);
        let applied = apply_text_edits(src, &LineIndex::new(src), changes(&edit));
        assert_eq!(applied, "海がひろがる、その［＃「海」に傍点］");
    }

    #[test]
    fn rename_forward_attribute_only_change_edits_one_region() {
        // 傍点 → 傍線 keeps the same target, so the literal is unchanged: one edit.
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let off = src.rfind('［').expect("directive bracket");
        let edit = rename_at(src, off, "［＃「青空」に傍線］")
            .expect("attribute-only change keeps the forward")
            .expect("edit emitted");
        assert_eq!(changes(&edit).len(), 1);
        let applied = apply_text_edits(src, &LineIndex::new(src), changes(&edit));
        assert_eq!(applied, "青空がひろがる、その［＃「青空」に傍線］");
    }

    #[test]
    fn rename_ambiguous_referent_declines() {
        // 青空 appears twice upstream → no unique referent → splice declines.
        let src = "青空と青空、その［＃「青空」に傍点］";
        let off = src.rfind('［').expect("directive bracket");
        let err =
            rename_at(src, off, "［＃「海」に傍点］").expect_err("ambiguous referent declines");
        assert!(matches!(err, SpliceError::Unverifiable { .. }));
    }

    // ---- gate + round-trip ---------------------------------------------

    #[test]
    fn rename_crlf_doc_fails_the_gate() {
        let src = "前\r\n［＃ここから2字下げ］\r\n本文\r\n［＃ここで字下げ終わり］\r\n後";
        let off = src.find('［').expect("open marker");
        assert!(
            rename_at(src, off, "［＃ここから罫囲み］")
                .expect("gate returns Ok(None)")
                .is_none()
        );
    }

    /// The emitted `TextEdits`, applied to `text`, must reproduce
    /// `tree.splice(region, new_name)` byte-for-byte. This guards the
    /// two-region arithmetic recovery against `new_source`.
    fn assert_round_trip(src: &str, off: usize, new_name: &str) {
        let tree = aozora::parse(src).snapshot();
        let line_index = LineIndex::new(src);
        let region = tree
            .region_at(SourceOffset::new(u32::try_from(off).unwrap()))
            .expect("region at offset");
        let spliced = tree.splice(region, new_name).expect("splice succeeds");
        let edit = rename_edit(
            &tree,
            src,
            &line_index,
            &fake_uri(),
            pos(src, off),
            new_name,
        )
        .expect("rename ok")
        .expect("edit emitted");
        let applied = apply_text_edits(src, &line_index, changes(&edit));
        assert_eq!(applied, spliced, "edits must reproduce the splice output");
    }

    #[test]
    fn round_trip_container_family_change() {
        let off = CONTAINER_DOC.find('［').unwrap();
        assert_round_trip(CONTAINER_DOC, off, "［＃ここから罫囲み］");
    }

    #[test]
    fn round_trip_container_amount_change() {
        let off = CONTAINER_DOC.find('［').unwrap();
        assert_round_trip(CONTAINER_DOC, off, "［＃ここから4字下げ］");
    }

    #[test]
    fn round_trip_forward_target_change() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let off = src.rfind('［').unwrap();
        assert_round_trip(src, off, "［＃「海」に傍点］");
    }

    #[test]
    fn round_trip_forward_attribute_change() {
        let src = "青空がひろがる、その［＃「青空」に傍点］";
        let off = src.rfind('［').unwrap();
        assert_round_trip(src, off, "［＃「青空」に傍線］");
    }

    // ---- minimal_diff_edit (direct) ------------------------------------
    //
    // The minimal-diff fallback only fires when the two-region arithmetic
    // is out of bounds, so these exercise it directly. Each pins the exact
    // prefix/suffix trim offsets and the differing-middle replacement.

    /// Byte-offset span + replacement text of a `TextEdit`, reconstructed
    /// against `text` via the same `LineIndex` the edit was built from.
    fn edit_bytes(text: &str, li: &LineIndex, e: &TextEdit) -> (usize, usize, String) {
        let start = li.byte_offset(text, e.range.start).expect("start in range");
        let end = li.byte_offset(text, e.range.end).expect("end in range");
        (start, end, e.new_text.clone())
    }

    #[test]
    fn minimal_diff_ascii_middle_replacement() {
        // Common prefix "abc" (3 bytes), common suffix "fgh" (3 bytes): the
        // minimal edit replaces the differing middle bytes 3..6 with "QW".
        let text = "abcXYZfgh";
        let new_source = "abcQWfgh";
        let li = LineIndex::new(text);
        let edit = minimal_diff_edit(&li, text, new_source);
        let (start, end, new_text) = edit_bytes(text, &li, &edit);
        assert_eq!(start, 3, "prefix trimmed to byte 3");
        assert_eq!(end, 6, "old suffix trimmed to byte 6");
        assert_eq!(new_text, "QW", "only the differing middle is emitted");
    }

    #[test]
    fn minimal_diff_pure_insertion_at_end() {
        // `text` is a proper prefix of `new_source`: the prefix walk must
        // stop *at* max_pre without reading past the shorter buffer, and the
        // edit is a pure insertion of "XY" at byte 3 (empty replaced span).
        let text = "abc";
        let new_source = "abcXY";
        let li = LineIndex::new(text);
        let edit = minimal_diff_edit(&li, text, new_source);
        let (start, end, new_text) = edit_bytes(text, &li, &edit);
        assert_eq!(start, 3, "insertion point after the shared prefix");
        assert_eq!(end, 3, "nothing is deleted");
        assert_eq!(new_text, "XY");
    }

    #[test]
    fn minimal_diff_snaps_across_multibyte_boundaries() {
        // "あ" = E3 81 82 and "も" = E3 82 82 share their lead byte (after the
        // ASCII 'X') and their trailing byte, so a naive byte diff cuts the
        // prefix inside the kana (byte 2) and the suffix inside it (byte 1).
        // Both must snap to char boundaries: replace the whole "あ" (bytes
        // 1..4) with "も". Catches byte-vs-char-boundary confusion.
        let text = "Xあ";
        let new_source = "Xも";
        let li = LineIndex::new(text);
        let edit = minimal_diff_edit(&li, text, new_source);
        let (start, end, new_text) = edit_bytes(text, &li, &edit);
        assert_eq!(start, 1, "prefix snapped back to the 'あ' boundary");
        assert_eq!(end, 4, "suffix snapped forward to the end");
        assert_eq!(new_text, "も");
    }

    // ---- recover_two_edits (direct) ------------------------------------

    #[test]
    fn recover_partner_at_same_start_does_not_shift() {
        // q.start == p.start: the partner sits *at* the primary, not strictly
        // after it, so it must keep its start (the `>` branch is false). Under
        // `>=` the partner would shift by the primary delta (-1) to a negative
        // start and the recovery would bail to None.
        let text = "0123456789";
        let new_source = "abcdefghij"; // same length → total_delta 0
        let li = LineIndex::new(text);
        let p = Span::new(0, 2);
        let q = Span::new(0, 4);
        // primary_delta = 1 - 2 = -1; `>` → q_new_start = 0 (valid) → Some.
        // `>=` → q_new_start = 0 + (-1) = -1 → guard returns None.
        let edits = recover_two_edits(&li, text, new_source, p, q, "X");
        assert!(
            edits.is_some(),
            "partner at the primary's start must not shift"
        );
    }

    #[test]
    fn recover_keeps_a_fully_deleted_partner() {
        // A partner whose new length is exactly 0 (deleted entirely) is a
        // valid recovery: the lower bound on `q_new_len` must be strict `< 0`,
        // not `== 0` or `<= 0`. Under those mutants the zero-length partner is
        // wrongly rejected and the whole recovery returns None.
        let text = "abcdefghijkl"; // len 12
        let new_source = "abABCDEfjkl"; // len 11 → total_delta -1
        let li = LineIndex::new(text);
        let p = Span::new(2, 5); // len 3
        let q = Span::new(6, 9); // len 3, starts strictly after p
        // primary_delta = 5 - 3 = 2; partner_delta = -1 - 2 = -3;
        // q_new_len = 3 + (-3) = 0; q_new_start = 6 + 2 = 8 (>= 0).
        let edits = recover_two_edits(&li, text, new_source, p, q, "ABCDE")
            .expect("a zero-length partner is a valid recovery, not a bail-out");
        assert_eq!(edits.len(), 2, "primary edit plus the partner deletion");
        assert_eq!(edits[1].new_text, "", "the partner region is deleted");
    }
}
