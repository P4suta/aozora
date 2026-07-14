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

use aozora_i18n::{self as i18n, FluentArgs, LanguageIdentifier};
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
#[allow(
    clippy::too_many_arguments,
    reason = "the wrap request (source, line index, uri, selection) plus the \
              resolved UI language for the action titles; lang is a \
              cross-cutting locale, not a data param to bundle"
)]
pub fn wrap_selection_actions(
    source: &str,
    line_index: &LineIndex,
    uri: &Url,
    selection: Range,
    lang: &LanguageIdentifier,
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

    // Titles come from the shared i18n catalog; the woven notation glyphs
    // (｜, 《》, 「」, 〔〕, ［＃…］) are locale-neutral aozora syntax.
    let mut actions: Vec<CodeActionOrCommand> = Vec::new();
    actions.extend([
        // Ruby: always prepend ｜ so the base's start is unambiguous (aozora
        // style-guide recommended) — the single canonical ruby-wrap shape.
        ruby_wrap(uri, selection, "《》", &i18n::t(lang, "lsp-action-ruby")),
        ruby_wrap(
            uri,
            selection,
            "《《》》",
            &i18n::t(lang, "lsp-action-ruby-double"),
        ),
        // The three plain surround-only wraps.
        wrap_pair(
            uri,
            selection,
            &WrapDecoration {
                open: "「",
                close: "」",
                title: &i18n::t(lang, "lsp-action-wrap-quote"),
            },
        ),
        wrap_pair(
            uri,
            selection,
            &WrapDecoration {
                open: "〔",
                close: "〕",
                title: &i18n::t(lang, "lsp-action-wrap-accent"),
            },
        ),
        wrap_pair(
            uri,
            selection,
            &WrapDecoration {
                open: "［＃",
                close: "］",
                title: &i18n::t(lang, "lsp-action-wrap-annotation"),
            },
        ),
        // Bouten: the selection is left as-is; the directive follows it.
        forward_bouten_action(uri, selection, selected, lang),
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
pub(crate) fn quick_fix_actions(
    uri: &Url,
    diagnostics: &[Diagnostic],
    lang: &LanguageIdentifier,
) -> Vec<CodeActionOrCommand> {
    diagnostics
        .iter()
        .filter_map(|diag| {
            let payload = diag
                .data
                .as_ref()
                .and_then(|v| serde_json::from_value::<DiagnosticPayload>(v.clone()).ok())?;
            build_quick_fix(uri, diag, payload, lang)
        })
        .collect()
}

fn build_quick_fix(
    uri: &Url,
    diag: &Diagnostic,
    payload: DiagnosticPayload,
    lang: &LanguageIdentifier,
) -> Option<CodeActionOrCommand> {
    match payload {
        DiagnosticPayload::UnclosedBracket {
            pair_kind,
            expected_close,
        } => Some(insert_close_action(
            uri,
            diag,
            pair_kind,
            &expected_close,
            lang,
        )),
        DiagnosticPayload::UnmatchedClose { pair_kind } => {
            Some(delete_unmatched_close_action(uri, diag, pair_kind, lang))
        }
        DiagnosticPayload::SourceContainsPua { codepoint } => {
            Some(delete_pua_action(uri, diag, codepoint, lang))
        }
        // ResidualAnnotationMarker → no automatic fix (the user must
        // choose which keyword they meant); the diagnostic's verbose
        // message lists the manual recovery steps.
        DiagnosticPayload::ResidualAnnotationMarker => None,
        DiagnosticPayload::NonCanonicalDirective { canonical } => {
            Some(rewrite_directive_action(uri, diag, &canonical, lang))
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the quick-fix context (uri, diagnostic, pair kind, close glyph) \
              plus the resolved UI language for the title; lang is a \
              cross-cutting locale, not a data param to bundle"
)]
fn insert_close_action(
    uri: &Url,
    diag: &Diagnostic,
    pair_kind: SerializablePairKind,
    close: &str,
    lang: &LanguageIdentifier,
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
    let mut args = FluentArgs::new();
    args.set("close", close.to_owned());
    args.set("open", pair_kind.open_str().to_owned());
    CodeActionOrCommand::CodeAction(CodeAction {
        title: i18n::tf(lang, "lsp-action-close-bracket", &args),
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
    lang: &LanguageIdentifier,
) -> CodeActionOrCommand {
    let close = pair_kind.close_str();
    // Replace the diagnostic span (the stray close) with empty text.
    let edits = vec![TextEdit {
        range: diag.range,
        new_text: String::new(),
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    let mut args = FluentArgs::new();
    args.set("close", close.to_owned());
    CodeActionOrCommand::CodeAction(CodeAction {
        title: i18n::tf(lang, "lsp-action-delete-unmatched", &args),
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

fn rewrite_directive_action(
    uri: &Url,
    diag: &Diagnostic,
    canonical: &str,
    lang: &LanguageIdentifier,
) -> CodeActionOrCommand {
    // Replace the whole ［＃…］ span (the diagnostic range) with the canonical
    // directive. The lint's `span` is the full bracket extent, so a single
    // range replace swaps the near-miss body without disturbing the delimiters.
    let new_text = format!("［＃{canonical}］");
    let mut args = FluentArgs::new();
    args.set("directive", new_text.clone());
    let title = i18n::tf(lang, "lsp-action-rewrite", &args);
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

fn delete_pua_action(
    uri: &Url,
    diag: &Diagnostic,
    codepoint: u32,
    lang: &LanguageIdentifier,
) -> CodeActionOrCommand {
    let edits = vec![TextEdit {
        range: diag.range,
        new_text: String::new(),
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    let mut args = FluentArgs::new();
    args.set("codepoint", format!("{codepoint:04X}"));
    CodeActionOrCommand::CodeAction(CodeAction {
        title: i18n::tf(lang, "lsp-action-delete-pua", &args),
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
fn forward_bouten_action(
    uri: &Url,
    selection: Range,
    selected: &str,
    lang: &LanguageIdentifier,
) -> CodeActionOrCommand {
    let new_text = format!("［＃「{selected}」に傍点］");
    let edits = vec![TextEdit {
        range: Range::new(selection.end, selection.end),
        new_text,
    }];
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: i18n::t(lang, "lsp-action-bouten"),
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

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    /// Shims mirroring the pre-i18n arities, pinned to the canonical English
    /// catalog so the title assertions below stay locale-stable.
    fn wrap_selection_actions(
        source: &str,
        line_index: &LineIndex,
        uri: &Url,
        selection: Range,
    ) -> Vec<CodeActionOrCommand> {
        super::wrap_selection_actions(source, line_index, uri, selection, &en())
    }

    fn quick_fix_actions(uri: &Url, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
        super::quick_fix_actions(uri, diagnostics, &en())
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
    fn action_titles_localize_by_lang() {
        // Titles come from the shared catalog: `ja` keeps the migrated
        // Japanese, `zh` the new Chinese; `en` is asserted via the shim above.
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let idx = LineIndex::new(src);
        let uri = fake_uri();
        let ruby_title = |tag: &str| {
            let lang: LanguageIdentifier = tag.parse().expect("locale parses");
            let acts = super::wrap_selection_actions(src, &idx, &uri, sel, &lang);
            let CodeActionOrCommand::CodeAction(ca) = &acts[0] else {
                unreachable!("first action is the ruby wrap")
            };
            ca.title.clone()
        };
        assert_eq!(ruby_title("ja"), "ルビをふる ｜SEL《》");
        assert_eq!(ruby_title("zh"), "添加注音 ｜SEL《》");
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
        assert!(ca.title.contains("Delete"), "title: {}", ca.title);
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

    // -----------------------------------------------------------------
    // Field-presence pins: every builder stamps the LSP metadata
    // (`title` / `kind` / `diagnostics` / `is_preferred`) explicitly.
    // Dropping any of these would silently fall back to
    // `CodeAction::default()` (`""` / `None`), so each assertion pins
    // the exact non-default value the builder must set.
    // -----------------------------------------------------------------

    fn as_code_action(action: &CodeActionOrCommand) -> &CodeAction {
        let CodeActionOrCommand::CodeAction(ca) = action else {
            panic!("expected CodeAction");
        };
        ca
    }

    #[test]
    fn build_action_sets_title_and_refactor_kind() {
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        let ca = as_code_action(&actions[0]);
        // build_action forwards the caller's title verbatim (default: "").
        assert_eq!(ca.title, "Add ruby ｜SEL《》");
        // and stamps REFACTOR_REWRITE (default: None).
        assert_eq!(ca.kind, Some(CodeActionKind::REFACTOR_REWRITE));
    }

    #[test]
    fn forward_bouten_action_sets_title_and_refactor_kind() {
        let src = "青空";
        let sel = Range::new(Position::new(0, 0), Position::new(0, 2));
        let actions = wrap_selection_actions(src, &LineIndex::new(src), &fake_uri(), sel);
        let ca = as_code_action(actions.last().expect("bouten last"));
        // Localized title (default: "") and REFACTOR_REWRITE kind (default: None).
        assert_eq!(ca.title, "Add emphasis dots ［＃「SEL」に傍点］");
        assert_eq!(ca.kind, Some(CodeActionKind::REFACTOR_REWRITE));
    }

    #[test]
    fn insert_close_action_carries_originating_diagnostic() {
        let diag = diag_with_payload(DiagnosticPayload::UnclosedBracket {
            pair_kind: SerializablePairKind::Bracket,
            expected_close: "］".to_owned(),
        });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        let ca = as_code_action(&actions[0]);
        // The source diagnostic is attached to the quick fix (default: None).
        assert_eq!(ca.diagnostics, Some(vec![diag]));
    }

    #[test]
    fn delete_unmatched_close_action_stamps_quickfix_diag_and_preferred() {
        let diag = diag_with_payload(DiagnosticPayload::UnmatchedClose {
            pair_kind: SerializablePairKind::Ruby,
        });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        let ca = as_code_action(&actions[0]);
        assert_eq!(ca.kind, Some(CodeActionKind::QUICKFIX)); // default: None
        assert_eq!(ca.diagnostics, Some(vec![diag])); // default: None
        assert_eq!(ca.is_preferred, Some(true)); // default: None
    }

    #[test]
    fn rewrite_directive_action_sets_title_and_carries_diagnostic() {
        let diag = diag_with_payload(DiagnosticPayload::NonCanonicalDirective {
            canonical: "ここで字下げ終わり".to_owned(),
        });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        let ca = as_code_action(&actions[0]);
        // Title interpolates the canonical directive (default: "").
        assert_eq!(ca.title, "Rewrite to `［＃ここで字下げ終わり］`");
        // The source diagnostic is attached (default: None).
        assert_eq!(ca.diagnostics, Some(vec![diag]));
    }

    #[test]
    fn delete_pua_action_stamps_quickfix_diag_and_preferred() {
        let diag = diag_with_payload(DiagnosticPayload::SourceContainsPua { codepoint: 0xE001 });
        let actions = quick_fix_actions(&fake_uri(), slice::from_ref(&diag));
        let ca = as_code_action(&actions[0]);
        assert_eq!(ca.kind, Some(CodeActionKind::QUICKFIX)); // default: None
        assert_eq!(ca.diagnostics, Some(vec![diag])); // default: None
        assert_eq!(ca.is_preferred, Some(true)); // default: None
    }
}
