//! `textDocument/{prepareRename,rename}` — the LSP face of the #202
//! splice engine.
//!
//! Renaming one site of an aozora *coupling* edits its partner
//! coherently: a container open marker and its matching close, or a
//! forward-reference / heading-hint / margin-note directive and the
//! upstream literal it points at. This module is a thin surface over the
//! core replacement engine, which is the single source of truth for what
//! couples to what. Partner derivation is not reimplemented here.
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
use std::collections::HashMap;

use aozora::{EditError, Snapshot, Span};
use tower_lsp::lsp_types::{Position, PrepareRenameResponse, Range, TextEdit, Url, WorkspaceEdit};

use crate::lsp::line_index::LineIndex;

/// `prepareRename`: `Some(range + placeholder)` when `position` sits on a
/// coupled region, else `None`.
///
/// Returns `None` for a `Direct` or `Opaque` region (nothing to couple), for
/// a position past the end of the document.
#[must_use]
pub(super) fn prepare_rename_at(
    tree: &Snapshot,
    text: &str,
    line_index: &LineIndex,
    position: Position,
) -> Option<PrepareRenameResponse> {
    let off = line_index.byte_offset(text, position)?;
    let span = tree.coupled_span(off)?;
    let placeholder = text.get(span.start as usize..span.end as usize)?.to_owned();
    Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: span_to_range(line_index, text, span),
        placeholder,
    })
}

/// `rename`: the coherent coupled edit for renaming the region at `position`
/// to `new_name`.
///
/// - `Ok(Some(edit))` — a coherent coupled edit.
/// - `Ok(None)` — the position is not on a coupled region or the splice was a
///   no-op.
/// - `Err(e)` — the splice engine *declined* the edit (an ambiguous referent,
///   a ruby-base target literal, a 、-joined multi-target, or an opaque node).
///
/// `new_name` is passed verbatim to [`Snapshot::replacement_edits`] as the full
/// new region text. The core derives the partner change and returns edits in
/// original-source coordinates.
///
/// # Errors
///
/// Returns [`EditError::Unverifiable`] when the core declines the coupled edit.
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
) -> Result<Option<WorkspaceEdit>, EditError> {
    let Some(off) = line_index.byte_offset(text, position) else {
        return Ok(None);
    };
    let edits = tree.replacement_edits(off, new_name)?;
    if edits.is_empty() {
        return Ok(None);
    }
    let edits = edits
        .into_iter()
        .map(|edit| TextEdit {
            range: Range::new(
                line_index.position(text, edit.range().start),
                line_index.position(text, edit.range().end),
            ),
            new_text: edit.replacement().to_owned(),
        })
        .collect();
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

/// Convert an original-source [`Span`] into an LSP [`Range`].
fn span_to_range(line_index: &LineIndex, text: &str, span: Span) -> Range {
    Range::new(
        line_index.position(text, span.start as usize),
        line_index.position(text, span.end as usize),
    )
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
        let tree = aozora::parse(src)
            .expect("source fits parser span limit")
            .snapshot();
        prepare_rename_at(&tree, src, &LineIndex::new(src), pos(src, off))
    }

    /// Convenience: parse `src`, run `rename_edit` at byte offset `off`.
    fn rename_at(
        src: &str,
        off: usize,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, EditError> {
        let tree = aozora::parse(src)
            .expect("source fits parser span limit")
            .snapshot();
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
        let tree = aozora::parse(src)
            .expect("source fits parser span limit")
            .snapshot();
        assert!(prepare_rename_at(&tree, src, &LineIndex::new(src), Position::new(9, 0)).is_none());
    }

    #[test]
    fn prepare_crlf_doc_uses_original_coordinates() {
        let src = "前\r\n［＃ここから2字下げ］\r\n本文\r\n［＃ここで字下げ終わり］\r\n後";
        let off = src.find('［').expect("open marker");
        let response = prepare_at(src, off).expect("container open is coupled");
        let PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = response else {
            panic!("expected RangeWithPlaceholder");
        };
        assert_eq!(placeholder, "［＃ここから2字下げ］");
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
        assert!(!changes(&edit).is_empty());
    }

    #[test]
    fn rename_container_amount_change_touches_only_the_open() {
        let off = CONTAINER_DOC.find('［').expect("open marker");
        let edit = rename_at(CONTAINER_DOC, off, "［＃ここから4字下げ］")
            .expect("amount change is verifiable")
            .expect("edit emitted");
        // The close keyword (字下げ終わり) carries no amount, so it is verbatim:
        assert!(!changes(&edit).is_empty());
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
        assert!(!changes(&edit).is_empty());
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
        assert!(!changes(&edit).is_empty());
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
        assert_eq!(err, EditError::Unverifiable);
    }

    #[test]
    fn rename_crlf_doc_preserves_original_line_endings() {
        let src = "前\r\n［＃ここから2字下げ］\r\n本文\r\n［＃ここで字下げ終わり］\r\n後";
        let off = src.find('［').expect("open marker");
        let edit = rename_at(src, off, "［＃ここから罫囲み］")
            .expect("rename succeeds")
            .expect("edit emitted");
        let applied = apply_text_edits(src, &LineIndex::new(src), changes(&edit));
        assert_eq!(
            applied,
            "前\r\n［＃ここから罫囲み］\r\n本文\r\n［＃罫囲み終わり］\r\n後"
        );
    }

    /// The emitted LSP edits must reproduce the core edit plan byte-for-byte.
    fn assert_round_trip(src: &str, off: usize, new_name: &str) {
        let tree = aozora::parse(src)
            .expect("source fits parser span limit")
            .snapshot();
        let line_index = LineIndex::new(src);
        let core_edits = tree
            .replacement_edits(off, new_name)
            .expect("core edit succeeds");
        let mut spliced = src.to_owned();
        for edit in core_edits.iter().rev() {
            spliced.replace_range(edit.range(), edit.replacement());
        }
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
}
