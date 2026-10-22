//! Completes catalogue directives through [`completion_at`].

use crate::i18n::{self as i18n, LanguageIdentifier};
use aozora::{Catalogue, CatalogueEntry as SlugEntry, CatalogueFamily as SlugFamily};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

const OPENERS: &[&str] = &["［＃", "[#", "［#", "[＃"];

fn opener_is_half_width(opener: &str) -> bool {
    opener != "［＃"
}

struct SlugCtx {
    prefix_start: usize,
    body_start: usize,
    /// Byte offset after an existing close, or the cursor when no close exists.
    close_end: usize,
    opener: &'static str,
}

impl SlugCtx {
    fn half_width(&self) -> bool {
        opener_is_half_width(self.opener)
    }
}

/// Returns catalogue completions, or an empty vector outside a directive.
#[must_use]
pub(super) fn completion_at(
    source: &str,
    position: Position,
    lang: &LanguageIdentifier,
) -> Vec<CompletionItem> {
    let Some(byte_offset) = position_to_byte_offset(source, position) else {
        return Vec::new();
    };
    let Some(ctx) = resolve_slug_context(source, byte_offset) else {
        return Vec::new();
    };

    Catalogue::directives()
        .iter()
        .map(|entry| build_completion_item(source, entry, &ctx, lang))
        .collect()
}

fn resolve_slug_context(source: &str, cursor: usize) -> Option<SlugCtx> {
    let prefix = &source[..cursor];
    let (prefix_start, opener) = OPENERS
        .iter()
        .filter_map(|&op| prefix.rfind(op).map(|pos| (pos, op)))
        .max_by_key(|&(pos, _)| pos)?;
    let opener_len = opener.len();
    let body_start = prefix_start + opener_len;
    if cursor < body_start {
        return None;
    }
    let body_so_far = &source[body_start..cursor];
    if body_so_far.contains('］') || body_so_far.contains(']') {
        return None;
    }
    let close_end = source[cursor..]
        .char_indices()
        .find_map(|(rel, ch)| match ch {
            '］' => Some(cursor + rel + '］'.len_utf8()),
            ']' => Some(cursor + rel + ']'.len_utf8()),
            '\n' => Some(cursor),
            _ => None,
        })
        .unwrap_or(cursor);
    Some(SlugCtx {
        prefix_start,
        body_start,
        close_end,
        opener,
    })
}

fn build_completion_item(
    source: &str,
    entry: &SlugEntry,
    ctx: &SlugCtx,
    lang: &LanguageIdentifier,
) -> CompletionItem {
    let detail = if ctx.half_width() {
        format!(
            "{}  {}",
            entry.doc,
            i18n::t(lang, "lsp-completion-half-to-full-hint")
        )
    } else {
        entry.doc.to_owned()
    };
    let filter_text = format!("{}{}", ctx.opener, entry.canonical);
    let mut item = CompletionItem {
        label: entry.canonical.to_owned(),
        filter_text: Some(filter_text),
        kind: Some(family_to_kind(entry.family)),
        detail: Some(detail),
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
        ..CompletionItem::default()
    };

    let (body_text, format) = if entry.accepts_param {
        (
            canonical_to_snippet(entry.canonical),
            InsertTextFormat::SNIPPET,
        )
    } else {
        (entry.canonical.to_owned(), InsertTextFormat::PLAIN_TEXT)
    };

    let existing_full_close_start: Option<usize> = (!ctx.half_width()
        && source
            .get(ctx.body_start..ctx.close_end)
            .is_some_and(|body| body.ends_with('］')))
    .then(|| ctx.close_end - '］'.len_utf8());

    let new_text = if ctx.half_width() {
        format!("［＃{body_text}］")
    } else if existing_full_close_start.is_some() {
        body_text
    } else {
        format!("{body_text}］")
    };

    let edit_start = if ctx.half_width() {
        ctx.prefix_start
    } else {
        ctx.body_start
    };
    let edit_end = existing_full_close_start.unwrap_or_else(|| ctx.close_end.max(edit_start));
    let range = Range::new(
        byte_offset_to_position(source, edit_start),
        byte_offset_to_position(source, edit_end),
    );
    item.text_edit = Some(CompletionTextEdit::Edit(TextEdit { range, new_text }));
    item.insert_text_format = Some(format);

    if entry.family == SlugFamily::BlockContainerOpen
        && let Some(partner) = entry.partner
        && !forward_already_has(source, ctx.close_end, partner)
    {
        let pos = byte_offset_to_position(source, ctx.close_end);
        item.additional_text_edits = Some(vec![TextEdit {
            range: Range::new(pos, pos),
            new_text: format!("\n\n［＃{partner}］"),
        }]);
    }
    item
}

/// Converts `{name}` placeholders into numbered snippet fields.
pub(crate) fn canonical_to_snippet(canonical: &str) -> String {
    const SNIPPET_CAPACITY_SLACK: usize = 4;
    let mut out = String::with_capacity(canonical.len() + SNIPPET_CAPACITY_SLACK);
    let mut chars = canonical.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '{' {
            let rest = &canonical[i..];
            if let Some(close_rel) = rest.find('}') {
                let placeholder = &rest[1..close_rel];
                out.push_str("${1:");
                out.push_str(placeholder);
                out.push('}');
                while let Some(&(j, _)) = chars.peek() {
                    if j > i + close_rel {
                        break;
                    }
                    chars.next();
                }
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn forward_already_has(source: &str, cursor: usize, partner: &str) -> bool {
    source[cursor..].contains(partner)
}

pub(crate) const fn family_to_kind(family: SlugFamily) -> CompletionItemKind {
    match family {
        SlugFamily::PageBreak | SlugFamily::Section => CompletionItemKind::EVENT,
        SlugFamily::BlockContainerOpen | SlugFamily::BlockContainerClose => {
            CompletionItemKind::CLASS
        }
        SlugFamily::LeafAlign => CompletionItemKind::ENUM_MEMBER,
        SlugFamily::Bouten => CompletionItemKind::FIELD,
        SlugFamily::Illustration => CompletionItemKind::FILE,
        SlugFamily::Framed | SlugFamily::Warichu => CompletionItemKind::STRUCT,
        SlugFamily::CombineUpright => CompletionItemKind::OPERATOR,
        SlugFamily::KaeritenSingle | SlugFamily::KaeritenCompound => CompletionItemKind::CONSTANT,
        _ => CompletionItemKind::TEXT,
    }
}

pub(crate) fn family_label(lang: &LanguageIdentifier, family: SlugFamily) -> String {
    let key = match family {
        SlugFamily::PageBreak => "lsp-completion-family-page-break",
        SlugFamily::Section => "lsp-completion-family-section",
        SlugFamily::BlockContainerOpen => "lsp-completion-family-block-open",
        SlugFamily::BlockContainerClose => "lsp-completion-family-block-close",
        SlugFamily::LeafAlign => "lsp-completion-family-leaf-align",
        SlugFamily::Bouten => "lsp-completion-family-bouten",
        SlugFamily::Illustration => "lsp-completion-family-illustration",
        SlugFamily::Framed => "lsp-completion-family-framed",
        SlugFamily::Warichu => "lsp-completion-family-warichu",
        SlugFamily::CombineUpright => "lsp-completion-family-combine-upright",
        SlugFamily::KaeritenSingle => "lsp-completion-family-kaeriten-single",
        SlugFamily::KaeritenCompound => "lsp-completion-family-kaeriten-compound",
        _ => "lsp-completion-family-other",
    };
    i18n::t(lang, key)
}

#[cfg(test)]
mod tests {

    use super::*;

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    fn completion_at(source: &str, position: Position) -> Vec<CompletionItem> {
        super::completion_at(source, position, &en())
    }

    #[test]
    fn completion_inside_full_width_open_returns_full_catalogue() {
        let src = "前文［＃";

        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        assert!(!items.is_empty(), "expected catalogue suggestions");
        assert!(
            items.iter().any(|i| i.label == "改ページ"),
            "expected 改ページ in completions",
        );
    }

    #[test]
    fn completion_inside_half_width_open_returns_full_catalogue() {
        let src = "[#";

        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        assert!(!items.is_empty(), "[# must trigger completions too");
        assert!(
            items
                .iter()
                .all(|i| i.detail.as_deref().unwrap_or("").contains("half-width")),
            "half-width hint missing in detail",
        );
    }

    #[test]
    fn half_width_hint_localizes_by_lang() {
        let src = "[#";
        let pos = byte_offset_to_position(src, src.len());
        let hint = |tag: &str| {
            let lang: LanguageIdentifier = tag.parse().expect("locale parses");
            super::completion_at(src, pos, &lang)[0]
                .detail
                .clone()
                .expect("detail present")
        };
        assert!(hint("ja").contains("(半角→全角)"), "ja: {}", hint("ja"));
        assert!(hint("zh").contains("(半角→全角)"), "zh: {}", hint("zh"));
    }

    #[test]
    fn half_width_completion_replaces_prefix_with_full_width_form() {
        let src = "[#";

        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry
            .text_edit
            .as_ref()
            .expect("text_edit present for half-width completion")
        else {
            panic!("expected Edit variant");
        };
        assert_eq!(edit.new_text, "［＃改ページ］");
    }

    #[test]
    fn half_width_completion_with_existing_close_replaces_through_close() {
        let src = "[#bouten]";

        let pos = byte_offset_to_position(src, "[#bouten".len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "［＃改ページ］");
        assert_eq!(edit.range.start, byte_offset_to_position(src, 0));
        assert_eq!(edit.range.end, byte_offset_to_position(src, src.len()));
    }

    #[test]
    fn full_width_completion_keeps_brackets_in_place() {
        let src = "［＃改］";

        let pos = byte_offset_to_position(src, "［＃".len() + "改".len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "改ページ");
        let expected_start_byte = "［＃".len();
        let expected_end_byte = "［＃改".len();
        assert_eq!(
            edit.range.start,
            byte_offset_to_position(src, expected_start_byte),
        );
        assert_eq!(
            edit.range.end,
            byte_offset_to_position(src, expected_end_byte),
        );
    }

    #[test]
    fn full_width_completion_on_empty_pair_inserts_body_only() {
        let src = "［＃］";
        let pos = byte_offset_to_position(src, "［＃".len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "ここから{N}字下げ")
            .expect("ここから{N}字下げ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert!(
            edit.new_text.starts_with("ここから${1:"),
            "expected snippet form, got {:?}",
            edit.new_text,
        );
        assert!(edit.new_text.ends_with("字下げ"));
        assert!(
            !edit.new_text.contains('］'),
            "new_text must be body-only when close already exists, got {:?}",
            edit.new_text,
        );
        let cursor_byte = "［＃".len();
        assert_eq!(edit.range.start, byte_offset_to_position(src, cursor_byte));
        assert_eq!(edit.range.end, byte_offset_to_position(src, cursor_byte));
    }

    #[test]
    fn completion_in_plain_text_returns_empty() {
        let src = "no annotation context";

        let pos = byte_offset_to_position(src, 4);
        assert_eq!(completion_at(src, pos), Vec::<CompletionItem>::new());
    }

    #[test]
    fn half_width_completion_carries_filter_text_with_opener_prefix() {
        let src = "[#";
        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        assert_eq!(
            entry.filter_text.as_deref(),
            Some("[#改ページ"),
            "filter_text must include the half-width opener so VS Code's filter accepts the typed prefix",
        );
    }

    #[test]
    fn full_width_completion_carries_filter_text_with_full_width_opener() {
        let src = "［＃";
        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        assert_eq!(entry.filter_text.as_deref(), Some("［＃改ページ"));
    }

    #[test]
    fn mixed_opener_full_bracket_half_hash_fires_catalogue() {
        let src = "［#";
        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        assert!(!items.is_empty(), "［# (mixed) must trigger completions");
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        assert_eq!(
            entry.filter_text.as_deref(),
            Some("［#改ページ"),
            "filter_text must reflect the actual opener so VS Code accepts the typed prefix",
        );
    }

    #[test]
    fn mixed_opener_half_bracket_full_hash_fires_catalogue() {
        let src = "[＃";
        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        assert!(!items.is_empty(), "[＃ (mixed) must trigger completions");
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        assert_eq!(entry.filter_text.as_deref(), Some("[＃改ページ"));
    }

    #[test]
    fn mixed_opener_completion_emits_canonical_full_width_form() {
        for src in ["［#", "[＃", "[#"] {
            let pos = byte_offset_to_position(src, src.len());
            let items = completion_at(src, pos);
            let entry = items
                .iter()
                .find(|i| i.label == "改ページ")
                .unwrap_or_else(|| panic!("改ページ in completions for opener {src}"));
            let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
                unreachable!()
            };
            assert_eq!(
                edit.new_text, "［＃改ページ］",
                "non-canonical splice for opener {src}",
            );
        }
    }

    #[test]
    fn completion_after_closed_full_width_bracket_returns_empty() {
        let src = "前［＃改ページ］後";

        let pos = byte_offset_to_position(src, src.len());
        assert_eq!(completion_at(src, pos), Vec::<CompletionItem>::new());
    }

    #[test]
    fn block_container_open_attaches_partner_close() {
        let src = "本文\n［＃";

        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "ここから字下げ")
            .expect("ここから字下げ in suggestions");
        let edits = entry
            .additional_text_edits
            .as_ref()
            .expect("partner edit attached");
        assert_eq!(edits.len(), 1);
        assert!(
            edits[0].new_text.contains("ここで字下げ終わり"),
            "partner close text: {}",
            edits[0].new_text
        );
    }

    #[test]
    fn parametric_slug_emits_snippet_with_tabstop() {
        let src = "［＃";

        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "{N}字下げ")
            .expect("{N}字下げ in suggestions");
        assert_eq!(entry.insert_text_format, Some(InsertTextFormat::SNIPPET));
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert!(
            edit.new_text.contains("${1:"),
            "snippet missing tabstop: {:?}",
            edit.new_text
        );
    }

    #[test]
    fn newline_stops_forward_close_scan() {
        let src = "［＃\n］";
        let cursor = "［＃".len();
        let pos = byte_offset_to_position(src, cursor);
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "改ページ］");
        assert_eq!(edit.range.end, byte_offset_to_position(src, cursor));
    }

    #[test]
    fn full_width_close_offset_lands_at_bracket_start() {
        let src = "［＃あ］";
        let cursor = "［＃".len();
        let pos = byte_offset_to_position(src, cursor);
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "改ページ");
        let close_start = "［＃あ".len();
        assert_eq!(edit.range.end, byte_offset_to_position(src, close_start));
    }

    #[test]
    fn ascii_close_offset_covers_through_close() {
        let src = "［＃あ]";
        let cursor = "［＃".len();
        let pos = byte_offset_to_position(src, cursor);
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.range.end, byte_offset_to_position(src, src.len()));
    }

    #[test]
    fn completion_item_carries_family_kind() {
        let src = "［＃";
        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        assert_eq!(entry.kind, Some(CompletionItemKind::EVENT));
    }

    #[test]
    fn completion_item_carries_documentation() {
        let src = "［＃";
        let pos = byte_offset_to_position(src, src.len());
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let Some(Documentation::MarkupContent(markup)) = entry.documentation.as_ref() else {
            panic!(
                "expected markdown documentation, got {:?}",
                entry.documentation
            );
        };
        assert_eq!(markup.kind, MarkupKind::Markdown);
        assert!(
            markup.value.contains("ページを改める"),
            "documentation must embed the slug doc string, got {:?}",
            markup.value,
        );
        assert!(markup.value.contains("Page break"));
        assert!(!markup.value.contains("PageBreak"));
    }

    #[test]
    fn half_width_existing_full_close_does_not_shrink_replacement_range() {
        let src = "[#あ］";
        let cursor = "[#".len();
        let pos = byte_offset_to_position(src, cursor);
        let items = completion_at(src, pos);
        let entry = items
            .iter()
            .find(|i| i.label == "改ページ")
            .expect("改ページ in completions");
        let CompletionTextEdit::Edit(edit) = entry.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "［＃改ページ］");
        assert_eq!(edit.range.start, byte_offset_to_position(src, 0));
        assert_eq!(edit.range.end, byte_offset_to_position(src, src.len()));
    }

    #[test]
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "`${1:N}` is LSP snippet placeholder syntax in the expected output, not a Rust format arg"
    )]
    fn snippet_consumes_exactly_through_the_placeholder_brace() {
        assert_eq!(canonical_to_snippet("{N}字下げ"), "${1:N}字下げ");
    }

    #[test]
    fn forward_already_has_detects_partner_presence() {
        assert!(forward_already_has(
            "本文\n［＃ここで字下げ終わり］",
            0,
            "ここで字下げ終わり",
        ));
        assert!(!forward_already_has(
            "本文［＃ここから字下げ］\nつづき",
            0,
            "ここで字下げ終わり",
        ));
        let src = "［＃ここで字下げ終わり］";
        assert!(!forward_already_has(src, src.len(), "ここで字下げ終わり"));
    }

    #[test]
    fn family_to_kind_maps_every_family_to_its_icon() {
        assert_eq!(
            family_to_kind(SlugFamily::PageBreak),
            CompletionItemKind::EVENT
        );
        assert_eq!(
            family_to_kind(SlugFamily::Section),
            CompletionItemKind::EVENT
        );
        assert_eq!(
            family_to_kind(SlugFamily::BlockContainerOpen),
            CompletionItemKind::CLASS
        );
        assert_eq!(
            family_to_kind(SlugFamily::BlockContainerClose),
            CompletionItemKind::CLASS
        );
        assert_eq!(
            family_to_kind(SlugFamily::LeafAlign),
            CompletionItemKind::ENUM_MEMBER
        );
        assert_eq!(
            family_to_kind(SlugFamily::Bouten),
            CompletionItemKind::FIELD
        );
        assert_eq!(
            family_to_kind(SlugFamily::Illustration),
            CompletionItemKind::FILE
        );
        assert_eq!(
            family_to_kind(SlugFamily::Framed),
            CompletionItemKind::STRUCT
        );
        assert_eq!(
            family_to_kind(SlugFamily::Warichu),
            CompletionItemKind::STRUCT
        );
        assert_eq!(
            family_to_kind(SlugFamily::CombineUpright),
            CompletionItemKind::OPERATOR
        );
        assert_eq!(
            family_to_kind(SlugFamily::KaeritenSingle),
            CompletionItemKind::CONSTANT
        );
        assert_eq!(
            family_to_kind(SlugFamily::KaeritenCompound),
            CompletionItemKind::CONSTANT
        );
    }

    #[test]
    fn family_labels_hide_internal_variant_names() {
        let families = [
            (SlugFamily::PageBreak, "Page break"),
            (SlugFamily::Section, "Section break"),
            (SlugFamily::BlockContainerOpen, "Block start"),
            (SlugFamily::BlockContainerClose, "Block end"),
            (SlugFamily::LeafAlign, "Line alignment"),
            (SlugFamily::Bouten, "Emphasis marks"),
            (SlugFamily::Illustration, "Illustration"),
            (SlugFamily::Framed, "Framed text"),
            (SlugFamily::Warichu, "Warichu"),
            (SlugFamily::CombineUpright, "Tate-chu-yoko"),
            (SlugFamily::KaeritenSingle, "Kaeriten"),
            (SlugFamily::KaeritenCompound, "Compound kaeriten"),
        ];
        for (family, expected) in families {
            assert_eq!(family_label(&en(), family), expected);
        }
    }
}
