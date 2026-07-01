//! `textDocument/codeAction` handler — wrap selection in a delimiter
//! pair.
//!
//! When the user has a non-empty selection in an aozora document, the
//! editor (right-click → Refactor, or Ctrl+. lightbulb) shows a menu
//! of wrap actions:
//!
//! - `｜SEL《》` ルビをふる (selection becomes the kanji base; reading slot empty)
//! - `｜SEL《《》》` 二重ルビをふる
//! - `SEL［＃「SEL」に傍点］` 傍点をつける
//! - `「SEL」` 鉤括弧で囲む
//! - `〔SEL〕` アクセント分解で囲む
//! - `［＃SEL］` 注記化
//!
//! Each action is a [`CodeAction`] carrying a [`WorkspaceEdit`] that
//! splices the open/close around `selection`. The 縦中横 / 傍点
//! forward-reference variants additionally insert the
//! `［＃「TARGET」…］` directive after the selection, with `TARGET`
//! pre-filled to the selected text.

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Diagnostic, Range, TextEdit, Url,
    WorkspaceEdit,
};

use crate::diagnostics::{DiagnosticPayload, SerializablePairKind};
use crate::line_index::LineIndex;
use std::collections::HashMap;

/// Compute every wrap-selection [`CodeAction`] applicable to
/// `selection` in `source`. Returns an empty vec when the selection
/// is empty or unresolvable.
#[must_use]
pub fn wrap_selection_actions(
    source: &str,
    line_index: &LineIndex,
    uri: &Url,
    selection: Range,
) -> Vec<CodeActionOrCommand> {
    let Some(start) = line_index.byte_offset(source, selection.start) else {
        return Vec::new();
    };
    let Some(end) = line_index.byte_offset(source, selection.end) else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    let selected = &source[start..end];

    let mut actions: Vec<CodeActionOrCommand> = Vec::new();
    actions.extend([
        // ルビ系: ｜ を必ず先頭に挿入する。aozora 記法のスタイルガイドが
        // 「ベースの開始位置を曖昧にしないため常に ｜ を付ける」を推奨
        // しているため、ルビ wrap の唯一の形をこれに統一する。
        ruby_wrap(uri, selection, "《》", "ルビをふる ｜SEL《》"),
        ruby_wrap(uri, selection, "《《》》", "二重ルビをふる ｜SEL《《》》"),
        // 文字列を囲むだけの 3 種。
        wrap_pair(
            uri,
            selection,
            &WrapDecoration {
                open: "「",
                close: "」",
                title: "「」 で囲む",
            },
        ),
        wrap_pair(
            uri,
            selection,
            &WrapDecoration {
                open: "〔",
                close: "〕",
                title: "〔〕 で囲む (アクセント分解)",
            },
        ),
        wrap_pair(
            uri,
            selection,
            &WrapDecoration {
                open: "［＃",
                close: "］",
                title: "［＃...］ 注記にする",
            },
        ),
        // 傍点: 選択文字はそのまま、注記が後ろに付く。
        forward_bouten_action(uri, selection, selected),
    ]);
    actions
}

/// Open / close decoration shipped to [`wrap_pair`].
///
/// Bundling these three together keeps `wrap_pair` under the
/// workspace `too-many-arguments-threshold`; callers also document
/// themselves better with named fields than with a row of bare
/// `&str` arguments.
struct WrapDecoration<'a> {
    open: &'a str,
    close: &'a str,
    title: &'a str,
}

/// Build a single open/close wrap [`CodeAction`]. Selection ends
/// up *inside* the open / close pair.
fn wrap_pair(uri: &Url, selection: Range, deco: &WrapDecoration<'_>) -> CodeActionOrCommand {
    let edits = vec![
        TextEdit {
            range: Range::new(selection.start, selection.start),
            new_text: deco.open.to_owned(),
        },
        TextEdit {
            range: Range::new(selection.end, selection.end),
            new_text: deco.close.to_owned(),
        },
    ];
    build_action(uri, edits, deco.title)
}

/// Build a "ルビをふる" wrap [`CodeAction`]. Selection becomes the
/// **kanji base**; `｜` is prepended so the base's start is pinned
/// (aozora style-guide recommended). The `reading_brackets`
/// argument is the closer pair (`《》` for normal, `《《》》` for
/// double); inserting them empty puts the cursor inside the
/// reading slot when the editor expands the snippet.
fn ruby_wrap(
    uri: &Url,
    selection: Range,
    reading_brackets: &str,
    title: &str,
) -> CodeActionOrCommand {
    let edits = vec![
        TextEdit {
            range: Range::new(selection.start, selection.start),
            new_text: "｜".to_owned(),
        },
        TextEdit {
            range: Range::new(selection.end, selection.end),
            new_text: reading_brackets.to_owned(),
        },
    ];
    build_action(uri, edits, title)
}

fn build_action(uri: &Url, edits: Vec<TextEdit>, title: &str) -> CodeActionOrCommand {
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: title.to_owned(),
        kind: Some(CodeActionKind::REFACTOR_REWRITE),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        ..CodeAction::default()
    })
}

/// Convert the LSP-supplied `params.context.diagnostics` into a
/// quick-fix [`CodeAction`] list. Each diagnostic carries a JSON
/// `data` payload (set by `crate::diagnostics::describe`, private) describing
/// what kind of fix is appropriate; this function decodes the
/// payload and emits a concrete [`WorkspaceEdit`].
///
/// Returns an empty `Vec` when no diagnostic in the request range
/// has a known fix shape.
#[must_use]
pub(crate) fn quick_fix_actions(uri: &Url, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    diagnostics
        .iter()
        .filter_map(|diag| {
            let payload = diag
                .data
                .as_ref()
                .and_then(|v| serde_json::from_value::<DiagnosticPayload>(v.clone()).ok())?;
            build_quick_fix(uri, diag, payload)
        })
        .collect()
}

fn build_quick_fix(
    uri: &Url,
    diag: &Diagnostic,
    payload: DiagnosticPayload,
) -> Option<CodeActionOrCommand> {
    match payload {
        DiagnosticPayload::UnclosedBracket {
            pair_kind,
            expected_close,
        } => Some(insert_close_action(uri, diag, pair_kind, &expected_close)),
        DiagnosticPayload::UnmatchedClose { pair_kind } => {
            Some(delete_unmatched_close_action(uri, diag, pair_kind))
        }
        DiagnosticPayload::SourceContainsPua { codepoint } => {
            Some(delete_pua_action(uri, diag, codepoint))
        }
        // ResidualAnnotationMarker → no automatic fix (the user must
        // choose which keyword they meant); the diagnostic's verbose
        // message lists the manual recovery steps.
        DiagnosticPayload::ResidualAnnotationMarker => None,
        DiagnosticPayload::NonCanonicalDirective { canonical } => {
            Some(rewrite_directive_action(uri, diag, &canonical))
        }
    }
}

fn insert_close_action(
    uri: &Url,
    diag: &Diagnostic,
    pair_kind: SerializablePairKind,
    close: &str,
) -> CodeActionOrCommand {
    // Insert the close at the end of the diagnostic's range — that
    // sits just past the unclosed open delimiter, which is the most
    // ergonomic landing spot for the auto-fix. The user can move it
    // afterward if they meant for the body to extend further.
    let edits = vec![TextEdit {
        range: Range::new(diag.range.end, diag.range.end),
        new_text: close.to_owned(),
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("`{close}` を補って閉じる ({} ペア)", pair_kind.open_str()),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        is_preferred: Some(true),
        ..CodeAction::default()
    })
}

fn delete_unmatched_close_action(
    uri: &Url,
    diag: &Diagnostic,
    pair_kind: SerializablePairKind,
) -> CodeActionOrCommand {
    let close = pair_kind.close_str();
    // Replace the diagnostic span (the stray close) with empty text.
    let edits = vec![TextEdit {
        range: diag.range,
        new_text: String::new(),
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("対応のない `{close}` を削除する"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        is_preferred: Some(true),
        ..CodeAction::default()
    })
}

fn rewrite_directive_action(uri: &Url, diag: &Diagnostic, canonical: &str) -> CodeActionOrCommand {
    // Replace the whole ［＃…］ span (the diagnostic range) with the canonical
    // directive. The lint's `span` is the full bracket extent, so a single
    // range replace swaps the near-miss body without disturbing the delimiters.
    let new_text = format!("［＃{canonical}］");
    let title = format!("`{new_text}` に書き換える");
    let edits = vec![TextEdit {
        range: diag.range,
        new_text,
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        is_preferred: Some(true),
        ..CodeAction::default()
    })
}

fn delete_pua_action(uri: &Url, diag: &Diagnostic, codepoint: u32) -> CodeActionOrCommand {
    let edits = vec![TextEdit {
        range: diag.range,
        new_text: String::new(),
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("私用領域文字 U+{codepoint:04X} を削除する"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        is_preferred: Some(true),
        ..CodeAction::default()
    })
}

/// Append a forward-reference `［＃「SEL」に傍点］` immediately after
/// the selection. The selection itself is not modified — bouten
/// targets the prior run.
fn forward_bouten_action(uri: &Url, selection: Range, selected: &str) -> CodeActionOrCommand {
    let new_text = format!("［＃「{selected}」に傍点］");
    let edits = vec![TextEdit {
        range: Range::new(selection.end, selection.end),
        new_text,
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: "傍点を付ける ［＃「SEL」に傍点］".to_owned(),
        kind: Some(CodeActionKind::REFACTOR_REWRITE),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        ..CodeAction::default()
    })
}

#[cfg(test)]
mod tests {
    use std::slice;

    use tower_lsp::lsp_types::Position;

    use super::*;

    fn fake_uri() -> Url {
        Url::parse("file:///fake.afm").expect("valid URL")
    }

    /// A diagnostic carrying a serialised quick-fix payload, the shape the
    /// `code_action` handler receives from the editor.
    fn diag_with_payload(payload: DiagnosticPayload) -> Diagnostic {
        Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 2)),
            data: Some(serde_json::to_value(payload).expect("serialise payload")),
            ..Diagnostic::default()
        }
    }

    /// The `new_text` of an action's single edit.
    fn single_edit_text(action: &CodeActionOrCommand) -> String {
        let CodeActionOrCommand::CodeAction(ca) = action else {
            panic!("expected CodeAction");
        };
        ca.edit
            .as_ref()
            .and_then(|e| e.changes.as_ref())
            .and_then(|c| c.values().next())
            .and_then(|edits| edits.first())
            .map(|edit| edit.new_text.clone())
            .expect("one edit")
    }

    fn extract_change_count(action: &CodeActionOrCommand) -> usize {
        let CodeActionOrCommand::CodeAction(ca) = action else {
            panic!("expected CodeAction");
        };
        ca.edit
            .as_ref()
            .and_then(|e| e.changes.as_ref())
            .and_then(|c| c.values().next())
            .map_or(0, Vec::len)
    }

    #[test]
    fn empty_selection_yields_no_actions() {
        let zero = Range::new(Position::new(0, 0), Position::new(0, 0));
        assert!(
            wrap_selection_actions("hello", &LineIndex::new("hello"), &fake_uri(), zero).is_empty()
        );
    }

    #[test]
    fn nonempty_selection_returns_full_menu() {
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        // ルビ + 二重ルビ + 「」 + 〔〕 + ［＃］ + 傍点 = 6 actions.
        assert_eq!(actions.len(), 6, "expected 6 wrap actions, got {actions:?}");
    }

    #[test]
    fn every_wrap_action_inserts_at_least_one_edit() {
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        for action in &actions {
            assert!(extract_change_count(action) >= 1);
        }
    }

    #[test]
    fn ruby_wrap_inserts_pipe_before_and_brackets_after() {
        // Pin the selection-as-base + always-pipe-prefix shape: the
        // first action in the menu is the bare ruby, which must
        // produce ｜SEL《》.
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        let CodeActionOrCommand::CodeAction(ca) = &actions[0] else {
            unreachable!()
        };
        let edits: Vec<&str> = ca
            .edit
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .iter()
            .map(|e| e.new_text.as_str())
            .collect();
        assert_eq!(edits, vec!["｜", "《》"]);
    }

    #[test]
    fn double_ruby_wrap_uses_double_brackets() {
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        let CodeActionOrCommand::CodeAction(ca) = &actions[1] else {
            unreachable!()
        };
        let edits: Vec<&str> = ca
            .edit
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .iter()
            .map(|e| e.new_text.as_str())
            .collect();
        assert_eq!(edits, vec!["｜", "《《》》"]);
    }

    // -----------------------------------------------------------------
    // quick_fix_actions — the diagnostic-driven fix path.
    // -----------------------------------------------------------------

    #[test]
    fn quick_fix_unclosed_bracket_inserts_close_delimiter() {
        let diag = diag_with_payload(DiagnosticPayload::UnclosedBracket {
            pair_kind: SerializablePairKind::Bracket,
            expected_close: "］".to_owned(),
        });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(ca) = &actions[0] else {
            unreachable!()
        };
        assert_eq!(ca.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(ca.is_preferred, Some(true));
        assert!(ca.title.contains('］'), "title: {}", ca.title);
        assert_eq!(single_edit_text(&actions[0]), "］");
    }

    #[test]
    fn quick_fix_unmatched_close_deletes_the_stray_delimiter() {
        let diag = diag_with_payload(DiagnosticPayload::UnmatchedClose {
            pair_kind: SerializablePairKind::Ruby,
        });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(ca) = &actions[0] else {
            unreachable!()
        };
        assert!(ca.title.contains("削除"), "title: {}", ca.title);
        // A deletion edit replaces the span with the empty string.
        assert_eq!(single_edit_text(&actions[0]), "");
    }

    #[test]
    fn quick_fix_pua_deletes_the_codepoint() {
        let diag = diag_with_payload(DiagnosticPayload::SourceContainsPua { codepoint: 0xE001 });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(ca) = &actions[0] else {
            unreachable!()
        };
        assert!(ca.title.contains("U+E001"), "title: {}", ca.title);
        assert_eq!(single_edit_text(&actions[0]), "");
    }

    #[test]
    fn residual_annotation_marker_offers_no_quick_fix() {
        let diag = diag_with_payload(DiagnosticPayload::ResidualAnnotationMarker);
        assert!(quick_fix_actions(&fake_uri(), slice::from_ref(&diag)).is_empty());
    }

    #[test]
    fn quick_fix_non_canonical_directive_replaces_span_with_canonical() {
        let diag = diag_with_payload(DiagnosticPayload::NonCanonicalDirective {
            canonical: "ここで字下げ終わり".to_owned(),
        });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(ca) = &actions[0] else {
            unreachable!()
        };
        assert_eq!(ca.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(ca.is_preferred, Some(true));
        // The whole ［＃…］ span is replaced with the canonical directive.
        assert_eq!(single_edit_text(&actions[0]), "［＃ここで字下げ終わり］");
    }

    #[test]
    fn diagnostic_without_payload_is_skipped() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
            ..Diagnostic::default()
        };
        assert!(quick_fix_actions(&fake_uri(), slice::from_ref(&diag)).is_empty());
    }

    #[test]
    fn diagnostic_with_unparsable_payload_is_skipped() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
            data: Some(serde_json::json!({ "kind": "not-a-known-payload" })),
            ..Diagnostic::default()
        };
        assert!(quick_fix_actions(&fake_uri(), slice::from_ref(&diag)).is_empty());
    }

    #[test]
    fn multiple_diagnostics_produce_multiple_fixes() {
        let diags = vec![
            diag_with_payload(DiagnosticPayload::SourceContainsPua { codepoint: 0xE001 }),
            diag_with_payload(DiagnosticPayload::UnmatchedClose {
                pair_kind: SerializablePairKind::Bracket,
            }),
        ];
        assert_eq!(quick_fix_actions(&fake_uri(), &diags).len(), 2);
    }

    #[test]
    fn forward_bouten_carries_selected_text() {
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        // Bouten is the LAST action in the menu.
        let bouten = actions.last().expect("bouten last");
        let CodeActionOrCommand::CodeAction(ca) = bouten else {
            unreachable!()
        };
        let change_text = ca
            .edit
            .as_ref()
            .unwrap()
            .changes
            .as_ref()
            .unwrap()
            .values()
            .next()
            .unwrap()[0]
            .new_text
            .clone();
        assert_eq!(change_text, "［＃「青空」に傍点］");
    }
}
