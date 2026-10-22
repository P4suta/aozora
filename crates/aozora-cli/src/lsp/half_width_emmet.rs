use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

struct EmmetRule {
    prefix: &'static str,
    snippet: &'static str,
    label: &'static str,
    detail_key: &'static str,
    is_snippet: bool,
}

const EMMET_RULES: &[EmmetRule] = &[
    EmmetRule {
        prefix: "<",
        snippet: "《${0}》",
        label: "《...》",
        detail_key: "lsp-emmet-ruby-open",
        is_snippet: true,
    },
    EmmetRule {
        prefix: ">",
        snippet: "》",
        label: "》",
        detail_key: "lsp-emmet-ruby-close",
        is_snippet: false,
    },
    EmmetRule {
        prefix: "[",
        snippet: "［",
        label: "［",
        detail_key: "lsp-emmet-bracket-open",
        is_snippet: false,
    },
    EmmetRule {
        prefix: "]",
        snippet: "］",
        label: "］",
        detail_key: "lsp-emmet-bracket-close",
        is_snippet: false,
    },
    EmmetRule {
        prefix: "|",
        snippet: "｜",
        label: "｜",
        detail_key: "lsp-emmet-ruby-base",
        is_snippet: false,
    },
    EmmetRule {
        prefix: "*",
        snippet: "※",
        label: "※",
        detail_key: "lsp-emmet-gaiji-marker",
        is_snippet: false,
    },
];

const SLUG_WINDOW: usize = 256;

#[must_use]
pub(super) fn emmet_completions(
    source: &str,
    position: Position,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    let Some(cursor) = position_to_byte_offset(source, position) else {
        return Vec::new();
    };
    if cursor == 0 {
        return Vec::new();
    }

    if in_slug_context(source, cursor) {
        return Vec::new();
    }

    EMMET_RULES
        .iter()
        .find_map(|rule| {
            let plen = rule.prefix.len();
            let start = cursor.checked_sub(plen)?;
            if !source.is_char_boundary(start) {
                return None;
            }
            let candidate = &source[start..cursor];
            (candidate == rule.prefix).then(|| build_item(source, cursor, rule, lang))
        })
        .map(|item| vec![item])
        .unwrap_or_default()
}

fn in_slug_context(source: &str, cursor: usize) -> bool {
    let tail = &source[..cursor];
    if !(tail.ends_with('#') || tail.ends_with('＃')) {
        return false;
    }
    let start = (cursor.saturating_sub(SLUG_WINDOW)..=cursor)
        .find(|&offset| source.is_char_boundary(offset))
        .unwrap_or(cursor);
    let window = &source[start..cursor];
    for ch in window.chars().rev() {
        match ch {
            '[' | '［' => return true,
            ']' | '］' | '\n' => return false,
            _ => {}
        }
    }
    false
}

fn build_item(
    source: &str,
    cursor: usize,
    rule: &EmmetRule,
    lang: &LanguageIdentifier,
) -> CompletionItem {
    let plen = rule.prefix.len();
    let edit_start = cursor - plen;
    let range = Range::new(
        byte_offset_to_position(source, edit_start),
        byte_offset_to_position(source, cursor),
    );
    let format = if rule.is_snippet {
        InsertTextFormat::SNIPPET
    } else {
        InsertTextFormat::PLAIN_TEXT
    };
    let kind = if rule.snippet.contains("${0}") {
        CompletionItemKind::SNIPPET
    } else {
        CompletionItemKind::TEXT
    };
    CompletionItem {
        label: rule.label.to_owned(),
        filter_text: Some(rule.prefix.to_owned()),
        kind: Some(kind),
        detail: Some(i18n::t(lang, rule.detail_key)),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: {
                let mut args = FluentArgs::new();
                args.set("prefix", rule.prefix.to_owned());
                args.set("glyph", rule.label.to_owned());
                i18n::tf(lang, "lsp-emmet-doc", &args)
            },
        })),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: rule.snippet.to_owned(),
        })),
        insert_text_format: Some(format),
        preselect: Some(true),
        ..CompletionItem::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, col: u32) -> Position {
        Position::new(line, col)
    }

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    fn emmet_completions(source: &str, position: Position) -> Vec<CompletionItem> {
        super::emmet_completions(source, position, &en())
    }

    fn first_label(source: &str, position: Position) -> Option<String> {
        let items = emmet_completions(source, position);
        items.into_iter().next().map(|it| it.label)
    }

    #[test]
    fn empty_source_yields_nothing() {
        assert_eq!(
            emmet_completions("", pos(0, 0)),
            Vec::<CompletionItem>::new()
        );
    }

    #[test]
    fn left_bracket_triggers_full_width_left_bracket() {
        assert_eq!(first_label("[", pos(0, 1)).as_deref(), Some("［"));
    }

    #[test]
    fn right_bracket_triggers_full_width_right_bracket() {
        assert_eq!(first_label("]", pos(0, 1)).as_deref(), Some("］"));
    }

    #[test]
    fn single_left_angle_triggers_ruby_open_pair() {
        assert_eq!(first_label("<", pos(0, 1)).as_deref(), Some("《...》"));
    }

    #[test]
    fn single_right_angle_triggers_ruby_close() {
        assert_eq!(first_label(">", pos(0, 1)).as_deref(), Some("》"));
    }

    #[test]
    fn pipe_triggers_ruby_base_marker() {
        assert_eq!(first_label("|", pos(0, 1)).as_deref(), Some("｜"));
    }

    #[test]
    fn ruby_pair_text_edit_range_covers_typed_angle() {
        let items = emmet_completions("<", pos(0, 1));
        let item = items.first().expect("expected one item");
        let CompletionTextEdit::Edit(edit) = item
            .text_edit
            .as_ref()
            .expect("text_edit must be set so VS Code replaces the prefix")
        else {
            panic!("expected Edit, got InsertReplace");
        };
        assert_eq!(edit.range.start, pos(0, 0));
        assert_eq!(edit.range.end, pos(0, 1));
        assert_eq!(edit.new_text, "《${0}》");
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    }

    #[test]
    fn left_bracket_inside_slug_context_yields_no_emmet() {
        let src = "前置きの本文 [#";
        let cursor = src.len();
        let position = byte_offset_to_position(src, cursor);
        assert_eq!(
            emmet_completions(src, position),
            Vec::<CompletionItem>::new()
        );
    }

    #[test]
    fn pipe_inserts_full_width_pipe_with_no_snippet() {
        let items = emmet_completions("|", pos(0, 1));
        let item = items.first().expect("expected one item");
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "｜");
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
    }

    #[test]
    fn long_text_with_pipe_at_end_still_triggers() {
        let src = "本文の途中で|";
        let cursor = src.len();
        let position = byte_offset_to_position(src, cursor);
        assert_eq!(first_label(src, position).as_deref(), Some("｜"));
    }

    #[test]
    fn every_emmet_item_carries_filter_text_matching_the_typed_prefix() {
        for trigger in ["[", "]", "<", ">", "|", "*"] {
            let items = emmet_completions(trigger, pos(0, 1));
            let item = items
                .first()
                .unwrap_or_else(|| panic!("expected an emmet item for `{trigger}`"));
            assert_eq!(
                item.filter_text.as_deref(),
                Some(trigger),
                "filter_text must match the typed prefix `{trigger}` so VS Code's filter accepts it",
            );
        }
    }

    #[test]
    fn asterisk_triggers_full_width_gaiji_marker() {
        assert_eq!(first_label("*", pos(0, 1)).as_deref(), Some("※"));
    }

    fn only_item(source: &str, position: Position) -> CompletionItem {
        emmet_completions(source, position)
            .into_iter()
            .next()
            .expect("expected exactly one emmet item")
    }

    #[test]
    fn slug_context_true_when_open_bracket_precedes_hash() {
        assert!(in_slug_context("[#", 2));
        assert!(in_slug_context("［#", "［#".len()));
    }

    #[test]
    fn slug_context_false_when_closer_or_newline_precedes_hash() {
        assert!(!in_slug_context("[]#", 3));
        assert!(!in_slug_context("[］#", "[］#".len()));
        assert!(!in_slug_context("[\n#", 3));
    }

    #[test]
    fn slug_context_snaps_window_start_forward_across_multibyte_char() {
        let src = format!("あ［{}＃", "あ".repeat(84));
        assert_eq!(src.len(), 261);
        let cursor = src.len();
        assert!(!in_slug_context(&src, cursor));
    }

    #[test]
    fn emmet_item_kind_reflects_snippet_vs_plain() {
        let snippet_item = only_item("<", pos(0, 1));
        assert_eq!(snippet_item.kind, Some(CompletionItemKind::SNIPPET));

        let plain_item = only_item("[", pos(0, 1));
        assert_eq!(plain_item.kind, Some(CompletionItemKind::TEXT));
    }

    #[test]
    fn emmet_item_carries_markdown_documentation() {
        let item = only_item("[", pos(0, 1));
        let Some(Documentation::MarkupContent(mc)) = item.documentation else {
            panic!("documentation must be Some(MarkupContent), got None/other");
        };
        assert_eq!(mc.kind, MarkupKind::Markdown);
        assert_eq!(mc.value.as_str(), "Half-width `[` → `［`");
    }

    #[test]
    fn emmet_detail_and_doc_localize_by_lang() {
        let item = |tag: &str| {
            let lang: LanguageIdentifier = tag.parse().expect("locale parses");
            super::emmet_completions("[", pos(0, 1), &lang)
                .into_iter()
                .next()
                .expect("one emmet item")
        };
        let ja = item("ja");
        assert_eq!(
            ja.detail.as_deref(),
            Some("全角左ブラケット (半角『[』→全角『［』)")
        );
        let Some(Documentation::MarkupContent(mc)) = ja.documentation else {
            panic!("ja documentation present")
        };
        assert_eq!(mc.value.as_str(), "半角 `[` → `［`");
        let zh = item("zh");
        assert_eq!(
            zh.detail.as_deref(),
            Some("全角左括号（半角『[』→全角『［』）")
        );
    }

    #[test]
    fn emmet_item_is_preselected() {
        let item = only_item("[", pos(0, 1));
        assert_eq!(item.preselect, Some(true));
    }
}
