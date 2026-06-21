//! Half-width → full-width "emmet" completion items for aozora notation.
//!
//! Aozora typesetters need full-width brackets (`［ ］`, `《 》`,
//! `｜`) and on a normal JIS keyboard those are several IME taps
//! away from the home row. This module surfaces a thin emmet-style
//! shortcut: when the user types a recognised half-width trigger
//! (e.g. `<<`), a single completion item appears that, on accept,
//! splices the full-width form (`《${0}》` with the cursor in the
//! reading slot) in place of the typed prefix.
//!
//! This complements the slug catalogue in [`crate::completion`]:
//! the slug path handles `[#` → `［＃canonical］` (with a 100+ entry
//! catalogue), and this module handles every *other* half-width →
//! full-width pair the notation uses.
//!
//! ## Design notes
//!
//! * **Single-item suggestions, not catalogues.** Each trigger
//!   resolves to one full-width target; the editor presents it as
//!   one suggestion, the user accepts with Enter / Tab.
//! * **No auto-replace.** We do not abuse `textDocument/onTypeFormatting`
//!   — that would mangle legitimate ASCII text (`[abc]`, `a < b`,
//!   `pipe | command` inside code blocks, etc.). Completion-driven
//!   leaves the user in control.
//! * **Snippet placeholder for paired triggers.** `<<` becomes
//!   `《${0}》` with the cursor between the brackets so the user can
//!   immediately type the reading. Single-character triggers (`|`)
//!   place the cursor after the substituted glyph.
//! * **Slug-context hand-off.** When the cursor sits inside a `[#`
//!   prefix, the slug catalogue takes precedence — we deliberately
//!   skip emitting the bare `[`→`［` suggestion in that case so the
//!   user's accept on the slug catalogue does not race with the
//!   bracket-only emmet item.

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::position::{byte_offset_to_position, position_to_byte_offset};

/// One half-width → full-width substitution rule.
struct EmmetRule {
    /// Half-width prefix the user typed, immediately before the cursor.
    prefix: &'static str,
    /// Snippet body that replaces the prefix on accept. Use `${0}` to
    /// position the final cursor (paired delimiters) or omit it
    /// (single-character output).
    snippet: &'static str,
    /// Label shown in the completion popup.
    label: &'static str,
    /// Detail shown next to the label.
    detail: &'static str,
    /// Plain-text format vs snippet. `true` when the snippet contains
    /// `${0}` or other tabstops.
    is_snippet: bool,
}

/// Catalogue of half-width emmet rules. Every entry covers a
/// well-known aozora notation glyph that the typesetter needs in
/// full-width form.
///
/// Single-character triggers. Each rule fires the moment the user
/// types the half-width char, and the suggestion is `preselect: true`
/// so a single Enter accepts. If the user actually wanted a literal
/// half-width char (rare in aozora prose), Esc dismisses.
///
/// We deliberately do NOT use 2-char prefixes (`<<` etc.) because
/// VS Code's completion session does not always re-fire on the
/// second keystroke when the first returned an empty list — the
/// suggestion silently never shows up. Single-char rules are
/// reliable.
const EMMET_RULES: &[EmmetRule] = &[
    // Ruby reading delimiter — `《...》` opens a snippet pair so the
    // user types the reading inside `《┃》` directly.
    EmmetRule {
        prefix: "<",
        snippet: "《${0}》",
        label: "《...》",
        detail: "ルビ読み (半角『<』→全角ペア『《》』)",
        is_snippet: true,
    },
    EmmetRule {
        prefix: ">",
        snippet: "》",
        label: "》",
        detail: "ルビ読み閉じ (半角『>』→全角『》』)",
        is_snippet: false,
    },
    // Annotation brackets. The `[#` slug catalogue (in
    // `crate::completion`) takes precedence after `#` is typed; bare
    // `[` here just normalises ASCII to full-width.
    EmmetRule {
        prefix: "[",
        snippet: "［",
        label: "［",
        detail: "全角左ブラケット (半角『[』→全角『［』)",
        is_snippet: false,
    },
    EmmetRule {
        prefix: "]",
        snippet: "］",
        label: "］",
        detail: "全角右ブラケット (半角『]』→全角『］』)",
        is_snippet: false,
    },
    // Ruby base marker — explicit-delimiter ruby `｜base《reading》`.
    EmmetRule {
        prefix: "|",
        snippet: "｜",
        label: "｜",
        detail: "ルビベース印 (半角『|』→全角『｜』)",
        is_snippet: false,
    },
    // Gaiji marker — `※[#…]` annotations. `*` is never used as a
    // half-width literal in aozora prose; the only sane meaning is
    // "I want to start a gaiji note."
    EmmetRule {
        prefix: "*",
        snippet: "※",
        label: "※",
        detail: "外字マーカー (半角『*』→全角『※』)",
        is_snippet: false,
    },
];

/// Maximum trigger prefix length, used to cap the look-back window.
const MAX_PREFIX_LEN: usize = 1;

/// Look-back window for `in_slug_context`. A slug body never spans
/// hundreds of bytes, so 256 covers every realistic case while
/// keeping the scan O(1).
const SLUG_WINDOW: usize = 256;

/// Compute emmet completion items at `position`. Returns an empty
/// vec if no half-width trigger sits immediately before the cursor.
#[must_use]
pub fn emmet_completions(source: &str, position: Position) -> Vec<CompletionItem> {
    let Some(cursor) = position_to_byte_offset(source, position) else {
        return Vec::new();
    };
    if cursor == 0 {
        return Vec::new();
    }

    // Hand-off to the slug path: when the user is inside a `[#` /
    // `［＃` prefix, the slug catalogue owns the suggestion list.
    // We bail to avoid offering the bare `[`→`［` item right after
    // they typed `[`-then-`#`.
    if in_slug_context(source, cursor) {
        return Vec::new();
    }

    // Walk back up to MAX_PREFIX_LEN bytes to find a matching rule.
    // Longer prefixes win (the `EMMET_RULES` order does the work).
    //
    // Skip any rule whose look-back lands inside a multi-byte UTF-8
    // codepoint — that's never a valid trigger (every trigger we
    // handle is ASCII), so the candidate byte slice would be a
    // pre-trigger Japanese char and `is_char_boundary` short-circuits
    // the costly `==` comparison.
    EMMET_RULES
        .iter()
        .find_map(|rule| {
            let plen = rule.prefix.len();
            if plen > cursor || plen > MAX_PREFIX_LEN {
                return None;
            }
            let start = cursor - plen;
            if !source.is_char_boundary(start) {
                return None;
            }
            let candidate = &source[start..cursor];
            (candidate == rule.prefix).then(|| build_item(source, cursor, rule))
        })
        .map(|item| vec![item])
        .unwrap_or_default()
}

fn in_slug_context(source: &str, cursor: usize) -> bool {
    // Slug path "owns" the cursor when:
    //   * the chars at cursor end with `#` or `＃`, AND
    //   * walking back, we find a `[` or `［` before any `]`/`］`/newline
    //
    // Concretely the only conflict we care about is `[#` after the
    // user typed both — the bare `[` rule already fired on the
    // first keystroke (correctly), and the `#` second keystroke
    // should hand off to the slug catalogue.
    let tail = &source[..cursor];
    if !(tail.ends_with('#') || tail.ends_with('＃')) {
        return false;
    }
    // Look back for the matching `[` / `［` within a bounded window.
    // `saturating_sub` can land mid-codepoint when the byte cap
    // chops a multi-byte char in two, so we snap forward to the next
    // valid char boundary before slicing — otherwise a long
    // Japanese paragraph above the cursor would panic.
    let mut start = cursor.saturating_sub(SLUG_WINDOW);
    while start < cursor && !source.is_char_boundary(start) {
        start += 1;
    }
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

fn build_item(source: &str, cursor: usize, rule: &EmmetRule) -> CompletionItem {
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
        // Paired delimiter — snippet semantics most natural here.
        CompletionItemKind::SNIPPET
    } else {
        CompletionItemKind::TEXT
    };
    CompletionItem {
        label: rule.label.to_owned(),
        // VS Code filters items by matching the typed prefix against
        // (filter_text || label). Our `label` is the full-width
        // target glyph (`［`), but the user types the half-width
        // prefix (`[`) — without `filter_text` the fuzzy matcher
        // sees "［" vs "[", scores zero, and the popup hides our
        // suggestion. Setting `filter_text` to the half-width prefix
        // makes the match exact.
        filter_text: Some(rule.prefix.to_owned()),
        kind: Some(kind),
        detail: Some(rule.detail.to_owned()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("半角 `{}` → `{}`", rule.prefix, rule.label),
        })),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: rule.snippet.to_owned(),
        })),
        insert_text_format: Some(format),
        // Mark as preselect so a single Enter accepts the substitution
        // when the user has only typed the trigger (the popup then
        // reads as "press Enter to expand").
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

    fn first_label(source: &str, position: Position) -> Option<String> {
        let items = emmet_completions(source, position);
        items.into_iter().next().map(|it| it.label)
    }

    #[test]
    fn empty_source_yields_nothing() {
        assert!(emmet_completions("", pos(0, 0)).is_empty());
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
        // Single `<` is enough to fire — using `<<` would be more
        // specific but VS Code doesn't reliably re-query after an
        // empty initial response, so single-char triggers are the
        // robust choice.
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
        // `<` typed at offset 0; on accept the text edit must
        // replace it with `《${0}》`. Range start = 0, end = 1.
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
        // After `[#`, the slug catalogue takes over (handled in
        // `crate::completion`). We must NOT also offer the bare `[`
        // emmet item, because `#` won't match the bare-bracket
        // rule's prefix anyway, but the slug-context guard pins
        // the policy explicitly.
        let src = "前置きの本文 [#";
        let cursor = src.len();
        let position = byte_offset_to_position(src, cursor);
        // The trigger char immediately before cursor is `#`, which
        // is not in the EMMET_RULES — verify and also confirm the
        // guard kicks in for completeness.
        assert!(emmet_completions(src, position).is_empty());
    }

    #[test]
    fn pipe_inserts_full_width_pipe_with_no_snippet() {
        let items = emmet_completions("|", pos(0, 1));
        let item = items.first().expect("expected one item");
        let CompletionTextEdit::Edit(edit) = item.text_edit.as_ref().unwrap() else {
            unreachable!()
        };
        assert_eq!(edit.new_text, "｜");
        // Pipe is single-character → plain text format, no `${0}`.
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
    }

    #[test]
    fn long_text_with_pipe_at_end_still_triggers() {
        // Replicates real-world flow: user is mid-sentence and types
        // `|` to start an explicit-delimiter ruby. The look-back
        // walks past the previous Japanese context fine.
        let src = "本文の途中で|";
        let cursor = src.len();
        let position = byte_offset_to_position(src, cursor);
        assert_eq!(first_label(src, position).as_deref(), Some("｜"));
    }

    #[test]
    fn every_emmet_item_carries_filter_text_matching_the_typed_prefix() {
        // Regression pin: VS Code matches `(filter_text || label)`
        // against the user's typed input. Our labels are full-width
        // (`［`, `《...》`, `｜`) but typed input is the half-width
        // prefix (`[`, `<`, `|`). Without filter_text the popup
        // hides the suggestion. Pin every rule's emitted item.
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
}
