//! Provides notation snippets with tab stops through [`snippet_completions`].

use crate::i18n::{self as i18n, LanguageIdentifier};
use aozora::{Catalogue, CatalogueFamily as SlugFamily};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::lsp::completion::{canonical_to_snippet, family_label, family_to_kind};
use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

const HASH: char = '#';
const PIPE_FW: char = '｜';
const OPEN_DOUBLE_ANGLE: char = '《';
const KOME: char = '※';
const EMPTY_WRAP: &str = "［＃${0}］";
const RUBY_LABEL: &str = "${base}《${reading}》";
const RUBY_BODY: &str = "${1:base}《${2:reading}》";
const READING_LABEL: &str = "${reading}》";
const READING_BODY: &str = "${1:reading}》";
const GAIJI_LABEL: &str = "［＃「${desc}」、${men}］";
const GAIJI_BODY: &str = "［＃「${1:description}」、${2:mencode}］${0}";
const RUBY_LOOKAHEAD_CHARS: usize = 32;

/// Returns snippet completions when a supported trigger precedes the cursor.
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

fn hash_wrap_completions(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    let Some(typed_start) = char_typed_just_before(source, cursor, HASH) else {
        return Vec::new();
    };
    if matches!(char_before(source, typed_start), Some('[' | '［')) {
        return Vec::new();
    }

    let range = Range::new(
        byte_offset_to_position(source, typed_start),
        byte_offset_to_position(source, cursor),
    );

    let mut items = Vec::new();

    items.push(CompletionItem {
        label: "［＃］".to_owned(),
        filter_text: Some("#".to_owned()),
        sort_text: Some("00".to_owned()),
        detail: Some(i18n::t(lang, "lsp-snippet-empty-wrap-detail")),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: i18n::t(lang, "lsp-snippet-empty-wrap-doc"),
        })),
        kind: Some(CompletionItemKind::SNIPPET),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: EMPTY_WRAP.to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    });

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
                    "**{family}** {accepts}\n\n{doc}",
                    family = family_label(lang, entry.family),
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

    let _ = SlugFamily::BlockContainerOpen;

    items
}

fn pipe_ruby_completion(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    if char_typed_just_before(source, cursor, PIPE_FW).is_none() {
        return Vec::new();
    }
    if has_ruby_struct_ahead(source, cursor) {
        return Vec::new();
    }
    let pos = byte_offset_to_position(source, cursor);
    vec![CompletionItem {
        label: RUBY_LABEL.to_owned(),
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
            new_text: RUBY_BODY.to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    }]
}

fn open_angle_reading_completion(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    if char_typed_just_before(source, cursor, OPEN_DOUBLE_ANGLE).is_none() {
        return Vec::new();
    }
    if source[cursor..].chars().take(32).any(|c| c == '》') {
        return Vec::new();
    }
    let pos = byte_offset_to_position(source, cursor);
    vec![CompletionItem {
        label: READING_LABEL.to_owned(),
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
            new_text: READING_BODY.to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    }]
}

fn kome_gaiji_completion(
    source: &str,
    cursor: usize,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    if char_typed_just_before(source, cursor, KOME).is_none() {
        return Vec::new();
    }
    if source[cursor..].chars().take(64).any(|c| c == '］') {
        return Vec::new();
    }
    let pos = byte_offset_to_position(source, cursor);
    vec![CompletionItem {
        label: GAIJI_LABEL.to_owned(),
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
            new_text: GAIJI_BODY.to_owned(),
        })),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        preselect: Some(true),
        ..CompletionItem::default()
    }]
}

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

fn char_before(source: &str, byte_end: usize) -> Option<char> {
    if byte_end == 0 || byte_end > source.len() {
        return None;
    }
    source[..byte_end].chars().next_back()
}

fn has_ruby_struct_ahead(source: &str, cursor: usize) -> bool {
    source[cursor..]
        .chars()
        .take(RUBY_LOOKAHEAD_CHARS)
        .take_while(|&c| c != '\n')
        .any(|c| c == '《' || c == '》')
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_RUBY_SNIPPET: &str = "${1:base}《${2:reading}》";
    const EXPECTED_READING_SNIPPET: &str = "${1:reading}》";
    const EXPECTED_GAIJI_SNIPPET: &str = "［＃「${1:description}」、${2:mencode}］${0}";

    fn pos_at(source: &str, byte: usize) -> Position {
        byte_offset_to_position(source, byte)
    }

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    fn snippet_completions(source: &str, position: Position) -> Vec<CompletionItem> {
        super::snippet_completions(source, position, &en())
    }

    #[test]
    fn hash_typed_alone_emits_empty_wrap_first() {
        let src = "#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert!(!items.is_empty(), "must emit at least the empty wrap");
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
        assert_eq!(edit.new_text, EXPECTED_RUBY_SNIPPET);
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    }

    #[test]
    fn pipe_inside_existing_ruby_yields_no_snippet() {
        let src = "｜太郎《たろう》";
        let cursor = "｜".len();
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
        assert_eq!(edit.new_text, EXPECTED_READING_SNIPPET);
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
        assert_eq!(edit.new_text, EXPECTED_GAIJI_SNIPPET);
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
        assert_eq!(
            snippet_completions("", pos_at("", 0)),
            Vec::<CompletionItem>::new()
        );
    }

    #[test]
    fn hash_after_japanese_text_still_fires_wrap() {
        let src = "本文の途中で#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        assert_ne!(items, Vec::<CompletionItem>::new());
        assert_eq!(items[0].label, "［＃］");
    }

    #[test]
    fn empty_wrap_carries_detail_documentation_and_kind() {
        let src = "#";
        let items = snippet_completions(src, pos_at(src, src.len()));
        let wrap = items
            .iter()
            .find(|i| i.label == "［＃］")
            .expect("empty wrap present");
        assert_eq!(
            wrap.detail.as_deref(),
            Some("Empty annotation-slug template (edit the body)")
        );
        let Some(Documentation::MarkupContent(mc)) = &wrap.documentation else {
            panic!("empty wrap must carry markdown documentation")
        };
        assert_eq!(mc.kind, MarkupKind::Markdown);
        assert_eq!(
            mc.value,
            "Convert `#` to `［＃<cursor>］`. Press Enter to confirm."
        );
        assert_eq!(wrap.kind, Some(CompletionItemKind::SNIPPET));
    }

    #[test]
    fn empty_wrap_prose_localizes_by_lang() {
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
        assert_eq!(kaipage.detail.as_deref(), Some(entry.doc));
        assert!(
            kaipage.documentation.is_some(),
            "catalogue item must carry documentation: {kaipage:?}",
        );
        assert_eq!(kaipage.kind, Some(family_to_kind(entry.family)));
        let expected_format = if entry.accepts_param {
            InsertTextFormat::SNIPPET
        } else {
            InsertTextFormat::PLAIN_TEXT
        };
        assert_eq!(kaipage.insert_text_format, Some(expected_format));
    }

    #[test]
    fn open_angle_fires_when_only_non_close_text_follows() {
        let src = "太郎《読み";
        let cursor = "太郎《".len();
        let items = snippet_completions(src, pos_at(src, cursor));
        assert_eq!(items.len(), 1, "reading snippet must fire: {items:?}");
        let CompletionTextEdit::Edit(edit) = items[0].text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, EXPECTED_READING_SNIPPET);
    }

    #[test]
    fn kome_fires_when_only_non_bracket_text_follows() {
        let src = "※あ";
        let cursor = "※".len();
        let items = snippet_completions(src, pos_at(src, cursor));
        assert_eq!(items.len(), 1, "gaiji skeleton must fire: {items:?}");
        let CompletionTextEdit::Edit(edit) = items[0].text_edit.as_ref().unwrap() else {
            panic!("expected Edit")
        };
        assert_eq!(edit.new_text, EXPECTED_GAIJI_SNIPPET);
    }

    #[test]
    fn char_before_returns_char_ending_exactly_at_offset() {
        assert_eq!(char_before("ab", 2), Some('b'));
        assert_eq!(char_before("あ", 3), Some('あ'));
        assert_eq!(char_before("ab", 1), Some('a'));
        assert_eq!(char_before("ab", 0), None);
        assert_eq!(char_before("ab", 5), None);
    }

    #[test]
    fn has_ruby_struct_ahead_matches_both_angle_chars_only() {
        assert!(has_ruby_struct_ahead("《", 0));
        assert!(has_ruby_struct_ahead("》", 0));
        assert!(!has_ruby_struct_ahead("あ", 0));
    }
}
