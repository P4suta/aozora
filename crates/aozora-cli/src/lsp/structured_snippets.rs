//! Structured-form snippet completions — IDE-style Tab-stop flow
//! for the four aozora multi-character forms (`［＃...］`, ruby
//! `｜base《reading》`, gaiji `※［＃「desc」、men］`).
//!
//! ## Why a snippet path on top of `onTypeFormatting`
//!
//! [`crate::lsp::on_type_formatting`] handles single-char swaps (`[` →
//! `［`, `*` → `※`, `|` → `｜`, `<` → `《`). That's the snappy
//! path for typing aozora content directly — no popup, no accept.
//!
//! But the multi-char forms benefit from the editor's snippet engine:
//! a typed `｜` could naturally expand to `｜${1:base}《${2:reading}》`,
//! letting the user fill in `太郎` → Tab → `たろう` → Tab. That's
//! the IDE-style Tab-stop flow the user asked for on 2026-04-29
//! ("｜太郎《たろう》でいえばIDEみたいに太郎の入力が終わったら
//! tabでルビ入力に直接飛べるとかさ"). It only works while a snippet
//! is **active** — i.e. the user accepted a snippet completion item
//! that emitted `${1}…${2}…${0}` placeholders. There is no LSP
//! mechanism to retroactively activate Tab-stops over already-typed
//! text.
//!
//! ## Per-trigger snippet table
//!
//! The completion popup fires when the user has just typed one of the
//! trigger chars (the on-type swap has already converted `*` → `※`,
//! etc — by the time this handler runs, the document holds the
//! full-width form). For each trigger we offer one or more
//! `preselect: true` snippet items so a single Enter accepts.
//!
//! | Just typed (after on-type) | Snippet body                                 | Notes                                   |
//! |----------------------------|----------------------------------------------|------------------------------------------|
//! | `#` (not inside any opener)| `［＃${0}］`                                  | + every catalogue entry as `［＃X］`     |
//! | `｜`                       | `${1:base}《${2:reading}》`                   | inserted AFTER the typed `｜`            |
//! | `《` (not preceded by 漢字 run) | `${1:reading}》`                          | Tab-stop on reading                      |
//! | `※`                        | `［＃「${1:description}」、${2:mencode}］${0}` | full gaiji skeleton                      |
//!
//! Each item carries `filter_text` matching the just-typed char so
//! VS Code's filter accepts the typed prefix verbatim — same trick
//! [`crate::lsp::completion`] and [`crate::lsp::half_width_emmet`] use.

#![allow(
    clippy::literal_string_with_formatting_args,
    reason = "snippet placeholders like `${1:base}` are shipped to the editor, not Rust format args"
)]

use crate::i18n::{self as i18n, LanguageIdentifier};
use aozora::{Catalogue, CatalogueFamily as SlugFamily};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::lsp::completion::{canonical_to_snippet, family_to_kind};
use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

const HASH: char = '#';
const PIPE_FW: char = '｜';
const OPEN_DOUBLE_ANGLE: char = '《';
const KOME: char = '※';

/// Compute every snippet completion item for the position. Returns
/// an empty vec when none of the trigger chars sits immediately
/// before the cursor.
#[must_use]
pub(super) fn snippet_completions(
    source: &str,
    position: Position,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    let Some(cursor) = position_to_byte_offset(source, position) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    items.extend(hash_wrap_completions(source, cursor, lang));
    items.extend(pipe_ruby_completion(source, cursor, lang));
    items.extend(open_angle_reading_completion(source, cursor, lang));
    items.extend(kome_gaiji_completion(source, cursor, lang));
    items
}

/// `#` typed alone (NOT immediately after `[` or `［`) → wrap into
/// `［＃${0}］` plus every catalogue entry pre-wrapped.
///
/// Existing `[#` / `［＃` flow goes through [`crate::lsp::completion`]
/// which already shows the catalogue. This path covers the case
/// where the user just types `#` first (no opener) and gets the
/// brackets + catalogue in one popup.
fn hash_wrap_completions(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    let Some(typed_start) = char_typed_just_before(source, cursor, HASH) else {
        return Vec::new();
    };
    // Skip when an opener is forming — the slug-catalogue path in
    // `crate::lsp::completion` owns that case.
    if matches!(char_before(source, typed_start), Some('[' | '［')) {
        return Vec::new();
    }

    let range = Range::new(
        byte_offset_to_position(source, typed_start),
        byte_offset_to_position(source, cursor),
    );

    let mut items = Vec::new();

    // Empty wrap, preselected so a single Enter expands.
    items.push(CompletionItem {
        label: "［＃］".to_owned(),
        filter_text: Some("#".to_owned()),
        // `sort_text` "00" makes this float to the top above the
        // catalogue entries (which sort by their canonical body
        // alphabetically).
        sort_text: Some("00".to_owned()),
        detail: Some(i18n::t(lang, "lsp-snippet-empty-wrap-detail")),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: i18n::t(lang, "lsp-snippet-empty-wrap-doc"),
        })),
        kind: Some(CompletionItemKind::SNIPPET),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: "［＃${0}］".to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    });

    // Each catalogue entry as a pre-wrapped option.
    for entry in Catalogue::directives() {
        let (body, format) = if entry.accepts_param {
            (
                canonical_to_snippet(entry.canonical),
                InsertTextFormat::SNIPPET,
            )
        } else {
            (entry.canonical.to_owned(), InsertTextFormat::PLAIN_TEXT)
        };
        items.push(CompletionItem {
            label: format!("［＃{}］", entry.canonical),
            filter_text: Some(format!("#{}", entry.canonical)),
            detail: Some(entry.doc.to_owned()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "**{family:?}** {accepts}\n\n{doc}",
                    family = entry.family,
                    accepts = if entry.accepts_param {
                        i18n::t(lang, "lsp-completion-takes-param")
                    } else {
                        String::new()
                    },
                    doc = entry.doc,
                ),
            })),
            kind: Some(family_to_kind(entry.family)),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: format!("［＃{body}］"),
            })),
            insert_text_format: Some(format),
            ..CompletionItem::default()
        });
    }

    // Block-container open entries get their close partner appended.
    // We don't try to attach `additional_text_edits` here because the
    // user is starting from `#` (no existing close to align with);
    // the partner pair lands inline in the snippet body for the
    // entries that need it.
    let _ = SlugFamily::BlockContainerOpen;

    items
}

/// `｜` typed alone → suggest `${1:base}《${2:reading}》` snippet
/// inserted AFTER the typed pipe. Activates IDE-style Tab navigation:
/// type base, Tab, type reading, Tab to escape.
fn pipe_ruby_completion(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    if char_typed_just_before(source, cursor, PIPE_FW).is_none() {
        return Vec::new();
    }
    // Don't fire when the user is mid-edit inside an existing ruby
    // (the `｜` is the start of one already, with content after).
    // Cheap heuristic: if any of `《》` appears within the next 32
    // chars on the same line, the user already has the structure.
    if has_ruby_struct_ahead(source, cursor) {
        return Vec::new();
    }
    let pos = byte_offset_to_position(source, cursor);
    vec![CompletionItem {
        label: "${base}《${reading}》".to_owned(),
        filter_text: Some("｜".to_owned()),
        sort_text: Some("00".to_owned()),
        detail: Some(i18n::t(lang, "lsp-snippet-ruby-detail")),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: i18n::t(lang, "lsp-snippet-ruby-doc"),
        })),
        kind: Some(CompletionItemKind::SNIPPET),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(pos, pos),
            new_text: "${1:base}《${2:reading}》".to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    }]
}

/// `《` typed alone → suggest `${1:reading}》` snippet. Useful when
/// the user types `《` directly (without going via `｜`); the
/// reading slot is selected for direct typing.
fn open_angle_reading_completion(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    if char_typed_just_before(source, cursor, OPEN_DOUBLE_ANGLE).is_none() {
        return Vec::new();
    }
    // Bail when there's already a `》` close ahead — the user is
    // editing inside an existing ruby pair.
    if source[cursor..].chars().take(32).any(|c| c == '》') {
        return Vec::new();
    }
    let pos = byte_offset_to_position(source, cursor);
    vec![CompletionItem {
        label: "${reading}》".to_owned(),
        filter_text: Some("《".to_owned()),
        sort_text: Some("00".to_owned()),
        detail: Some(i18n::t(lang, "lsp-snippet-reading-detail")),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: i18n::t(lang, "lsp-snippet-reading-doc"),
        })),
        kind: Some(CompletionItemKind::SNIPPET),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(pos, pos),
            new_text: "${1:reading}》".to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    }]
}

/// `※` typed alone → suggest the full gaiji skeleton
/// `［＃「${1:description}」、${2:mencode}］${0}`. The `※` itself is
/// already typed (or auto-converted from `*`); the snippet body
/// fills in the bracket structure with Tab-stops.
fn kome_gaiji_completion(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    if char_typed_just_before(source, cursor, KOME).is_none() {
        return Vec::new();
    }
    // Don't fire when the gaiji structure already exists ahead.
    if source[cursor..].chars().take(64).any(|c| c == '］') {
        return Vec::new();
    }
    let pos = byte_offset_to_position(source, cursor);
    vec![CompletionItem {
        label: "［＃「${desc}」、${men}］".to_owned(),
        filter_text: Some("※".to_owned()),
        sort_text: Some("00".to_owned()),
        detail: Some(i18n::t(lang, "lsp-snippet-gaiji-detail")),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: i18n::t(lang, "lsp-snippet-gaiji-doc"),
        })),
        kind: Some(CompletionItemKind::SNIPPET),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(pos, pos),
            new_text: "［＃「${1:description}」、${2:mencode}］${0}".to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    }]
}

// ---------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------

/// Returns `Some(typed_start)` when the char immediately before
/// `cursor` is `expected`. Honours UTF-8 boundaries so multi-byte
/// chars like `｜` (3 bytes) and `※` (3 bytes) work correctly.
fn char_typed_just_before(source: &str, cursor: usize, expected: char) -> Option<usize> {
    let len = expected.len_utf8();
    let typed_start = cursor.checked_sub(len)?;
    if !source.is_char_boundary(typed_start) {
        return None;
    }
    if !source[typed_start..].starts_with(expected) {
        return None;
    }
    Some(typed_start)
}

/// Returns the char ending exactly at byte offset `byte_end`, or
/// `None` if `byte_end` is `0` or sits mid-codepoint.
fn char_before(source: &str, byte_end: usize) -> Option<char> {
    if byte_end == 0 || byte_end > source.len() {
        return None;
    }
    source[..byte_end].chars().next_back()
}

/// Cheap "is the user already inside a ruby structure" check —
/// looks for `《` or `》` within the next 32 chars on the same line.
/// Used to suppress the `｜` snippet when the surrounding text
/// already has the structure.
fn has_ruby_struct_ahead(source: &str, cursor: usize) -> bool {
    source[cursor..]
        .chars()
        .take(32)
        .take_while(|&c| c != '\n')
        .any(|c| c == '《' || c == '》')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos_at(source: &str, byte: usize) -> Position {
        byte_offset_to_position(source, byte)
    }

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    /// Shim mirroring the pre-i18n `snippet_completions(source, position)`
    /// arity, pinned to English so the detail/doc assertions stay locale-stable.
    fn snippet_completions(source: &str, position: Position) -> Vec<CompletionItem> {
        super::snippet_completions(source, position, &en())
    }

    #[test]
    fn hash_typed_alone_emits_empty_wrap_first() {
        let src = "#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert!(!items.is_empty(), "must emit at least the empty wrap");
        // The empty wrap sorts above catalogue entries via sort_text "00".
        let first = &items[0];
        assert_eq!(first.label, "［＃］");
        assert_eq!(first.filter_text.as_deref(), Some("#"));
        assert_eq!(first.sort_text.as_deref(), Some("00"));
        let CompletionTextEdit::Edit(edit) = first.text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, "［＃${0}］");
        assert_eq!(first.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert_eq!(first.preselect, Some(true));
    }

    #[test]
    fn hash_typed_alone_also_emits_catalogue_entries() {
        let src = "#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        // 改ページ should be in there (one of the simplest catalogue
        // entries, no parameters).
        let kaipage = items
            .iter()
            .find(|i| i.label == "［＃改ページ］")
            .expect("改ページ in catalogue items");
        assert_eq!(kaipage.filter_text.as_deref(), Some("#改ページ"));
        let CompletionTextEdit::Edit(edit) = kaipage.text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, "［＃改ページ］");
    }

    #[test]
    fn hash_inside_existing_opener_yields_no_wrap() {
        // `[#` and `［#` go through the slug-catalogue path in
        // `crate::lsp::completion`. We must NOT also offer the bare-`#`
        // wrap, otherwise the popup shows duplicate items.
        for src in ["[#", "［#"] {
            let items = snippet_completions(src, pos_at(src, src.len()));
            assert!(
                items.is_empty(),
                "no wrap for `{src}` (slug catalogue owns this case): {items:?}",
            );
        }
    }

    #[test]
    fn pipe_typed_alone_emits_ruby_snippet() {
        let src = "本文｜";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.filter_text.as_deref(), Some("｜"));
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, "${1:base}《${2:reading}》");
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    }

    #[test]
    fn pipe_inside_existing_ruby_yields_no_snippet() {
        // Cursor sits between `｜` and an already-existing `《》`.
        // No snippet — the user is editing, not creating.
        let src = "｜太郎《たろう》";
        let cursor = "｜".len(); // right after the leading ｜
        let items = snippet_completions(src, pos_at(src, cursor));
        assert!(items.is_empty(), "{items:?}");
    }

    #[test]
    fn open_angle_typed_alone_emits_reading_snippet() {
        let src = "太郎《";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert_eq!(items.len(), 1);
        let item = &items[0];
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, "${1:reading}》");
    }

    #[test]
    fn open_angle_with_close_ahead_yields_no_snippet() {
        let src = "太郎《たろう》";
        let cursor = "太郎《".len();
        let items = snippet_completions(src, pos_at(src, cursor));
        assert!(items.is_empty(), "{items:?}");
    }

    #[test]
    fn kome_typed_alone_emits_gaiji_skeleton() {
        let src = "本文※";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert_eq!(items.len(), 1);
        let item = &items[0];
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(
            edit.new_text,
            "［＃「${1:description}」、${2:mencode}］${0}"
        );
    }

    #[test]
    fn kome_with_close_bracket_ahead_yields_no_snippet() {
        let src = "※［＃「foo」、X］";
        let cursor = "※".len();
        let items = snippet_completions(src, pos_at(src, cursor));
        assert!(items.is_empty(), "{items:?}");
    }

    #[test]
    fn unrelated_text_emits_nothing() {
        let src = "ただの文章";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert!(items.is_empty(), "{items:?}");
    }

    #[test]
    fn empty_source_emits_nothing() {
        assert!(snippet_completions("", pos_at("", 0)).is_empty());
    }

    #[test]
    fn hash_after_japanese_text_still_fires_wrap() {
        // The look-back is byte-aware so multi-byte preceding chars
        // don't break the trigger detection.
        let src = "本文の途中で#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert!(!items.is_empty());
        assert_eq!(items[0].label, "［＃］");
    }

    #[test]
    fn empty_wrap_carries_detail_documentation_and_kind() {
        // Pins the three metadata fields on the preselected empty-wrap
        // item: dropping any of them (mutant "delete field …") flips it
        // to the `CompletionItem::default()` `None`.
        let src = "#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        let wrap = items
            .iter()
            .find(|i| i.label == "［＃］")
            .expect("empty wrap present");
        // `detail` field — the canonical English catalog value (via the shim).
        assert_eq!(
            wrap.detail.as_deref(),
            Some("Empty annotation-slug template (edit the body)")
        );
        // `documentation` field — pin the markdown payload.
        let Some(Documentation::MarkupContent(mc)) = &wrap.documentation else {
            panic!("empty wrap must carry markdown documentation")
        };
        assert_eq!(mc.kind, MarkupKind::Markdown);
        assert_eq!(
            mc.value,
            "Convert `#` to `［＃<cursor>］`. Press Enter to confirm."
        );
        // `kind` field.
        assert_eq!(wrap.kind, Some(CompletionItemKind::SNIPPET));
    }

    #[test]
    fn empty_wrap_prose_localizes_by_lang() {
        // The empty-wrap detail/doc come from the shared catalog, one per
        // locale (en is asserted above).
        let src = "#";
        let wrap = |tag: &str| {
            let lang: LanguageIdentifier = tag.parse().expect("locale parses");
            super::snippet_completions(src, pos_at(src, src.len()), &lang)
                .into_iter()
                .find(|i| i.label == "［＃］")
                .expect("empty wrap present")
        };
        assert_eq!(
            wrap("ja").detail.as_deref(),
            Some("注記スラグの空ひな型 (中身を編集)")
        );
        assert_eq!(
            wrap("zh").detail.as_deref(),
            Some("注记 slug 空模板（编辑内容）")
        );
    }

    #[test]
    fn catalogue_entry_carries_detail_documentation_kind_and_format() {
        // Pins the four metadata fields on a catalogue item. `改ページ`
        // takes no parameter, so its `insert_text_format` is PLAIN_TEXT.
        let src = "#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        let entry = Catalogue::directives()
            .iter()
            .find(|e| e.canonical == "改ページ")
            .expect("改ページ in SLUGS");
        let kaipage = items
            .iter()
            .find(|i| i.label == "［＃改ページ］")
            .expect("改ページ catalogue item present");
        // `detail` field is the entry's doc string.
        assert_eq!(kaipage.detail.as_deref(), Some(entry.doc));
        // `documentation` field is present.
        assert!(
            kaipage.documentation.is_some(),
            "catalogue item must carry documentation: {kaipage:?}",
        );
        // `kind` field is derived from the entry's family.
        assert_eq!(kaipage.kind, Some(family_to_kind(entry.family)));
        // `insert_text_format` field: no param → PLAIN_TEXT.
        let expected_format = if entry.accepts_param {
            InsertTextFormat::SNIPPET
        } else {
            InsertTextFormat::PLAIN_TEXT
        };
        assert_eq!(kaipage.insert_text_format, Some(expected_format));
    }

    #[test]
    fn open_angle_fires_when_only_non_close_text_follows() {
        // `《` just typed, a reading half-entered, but NO closing `》`
        // ahead → the `any(|c| c == '》')` guard is false → snippet
        // fires. Flipping `==` to `!=` sees the non-`》` chars and
        // wrongly bails (0 items).
        let src = "太郎《読み";
        let cursor = "太郎《".len();
        let items = snippet_completions(src, pos_at(src, cursor));
        assert_eq!(items.len(), 1, "reading snippet must fire: {items:?}");
        let CompletionTextEdit::Edit(edit) = items[0].text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, "${1:reading}》");
    }

    #[test]
    fn kome_fires_when_only_non_bracket_text_follows() {
        // `※` just typed, plain text after it, but NO closing `］`
        // ahead → the `any(|c| c == '］')` guard is false → skeleton
        // fires. Flipping `==` to `!=` sees the non-`］` char and
        // wrongly bails (0 items).
        let src = "※あ";
        let cursor = "※".len();
        let items = snippet_completions(src, pos_at(src, cursor));
        assert_eq!(items.len(), 1, "gaiji skeleton must fire: {items:?}");
        let CompletionTextEdit::Edit(edit) = items[0].text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(
            edit.new_text,
            "［＃「${1:description}」、${2:mencode}］${0}"
        );
    }

    #[test]
    fn char_before_returns_char_ending_exactly_at_offset() {
        // `byte_end == source.len()` must return the LAST char, not
        // `None`: `>` → `==` and `>` → `>=` both treat the end offset as
        // out of range and return `None` instead.
        assert_eq!(char_before("ab", 2), Some('b'));
        // Multi-byte: `あ` is 3 bytes; ending offset 3 == len → its char.
        assert_eq!(char_before("あ", 3), Some('あ'));
        // Interior boundary returns the preceding char.
        assert_eq!(char_before("ab", 1), Some('a'));
        // `byte_end == 0` → `None` (no char ends at the start).
        assert_eq!(char_before("ab", 0), None);
        // `byte_end` past the end → `None`, NOT a panic: `||` → `&&`
        // drops the out-of-range guard and slices out of bounds.
        assert_eq!(char_before("ab", 5), None);
    }

    #[test]
    fn has_ruby_struct_ahead_matches_both_angle_chars_only() {
        // Open angle alone → true; `c == '《'` → `!=` would miss it.
        assert!(has_ruby_struct_ahead("《", 0));
        // Close angle alone → true; `c == '》'` → `!=` would miss it.
        assert!(has_ruby_struct_ahead("》", 0));
        // A non-angle char → false; either `==` → `!=` flip flips it true.
        assert!(!has_ruby_struct_ahead("あ", 0));
    }
}
