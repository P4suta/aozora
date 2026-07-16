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

use aozora_i18n::{self as i18n, FluentArgs, LanguageIdentifier};
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
    /// i18n catalog key for the detail shown next to the label. Resolved in
    /// the server's UI language at build time.
    detail_key: &'static str,
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
    // Annotation brackets. The `[#` slug catalogue (in
    // `crate::completion`) takes precedence after `#` is typed; bare
    // `[` here just normalises ASCII to full-width.
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
    // Ruby base marker — explicit-delimiter ruby `｜base《reading》`.
    EmmetRule {
        prefix: "|",
        snippet: "｜",
        label: "｜",
        detail_key: "lsp-emmet-ruby-base",
        is_snippet: false,
    },
    // Gaiji marker — `※[#…]` annotations. `*` is never used as a
    // half-width literal in aozora prose; the only sane meaning is
    // "I want to start a gaiji note."
    EmmetRule {
        prefix: "*",
        snippet: "※",
        label: "※",
        detail_key: "lsp-emmet-gaiji-marker",
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
pub fn emmet_completions(
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
            (candidate == rule.prefix).then(|| build_item(source, cursor, rule, lang))
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
        // Detail prose from the shared catalog; the glyph substitution note in
        // the documentation weaves in the locale-neutral prefix / target glyph.
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

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    /// Shim mirroring the pre-i18n `emmet_completions(source, position)` arity,
    /// pinned to English so the documentation assertion stays locale-stable.
    fn emmet_completions(source: &str, position: Position) -> Vec<CompletionItem> {
        super::emmet_completions(source, position, &en())
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

    /// Fetch the single emmet item a trigger produces, panicking with a
    /// clear message when the completion list is empty.
    fn only_item(source: &str, position: Position) -> CompletionItem {
        emmet_completions(source, position)
            .into_iter()
            .next()
            .expect("expected exactly one emmet item")
    }

    #[test]
    fn slug_context_true_when_open_bracket_precedes_hash() {
        // `[#` and `［#`: walking back from the trailing hash we hit the
        // opening bracket before any closer, so the slug catalogue owns
        // the cursor. Pins `in_slug_context` == true, which kills:
        //   * `||`→`&&` at line 190 (would demand the tail end with both
        //     `#` AND `＃` — impossible — so it would always return
        //     false),
        //   * `&&`→`||` at line 199 (would run the snap loop up to
        //     `cursor`, emptying the look-back window → false),
        //   * deletion of the `'[' | '［'` arm at line 205 (the bracket
        //     would fall through to `_` and never report true).
        assert!(in_slug_context("[#", 2));
        assert!(in_slug_context("［#", "［#".len()));
    }

    #[test]
    fn slug_context_false_when_closer_or_newline_precedes_hash() {
        // A `]`, `］`, or newline seen before the `[` means the cursor is
        // NOT inside an open slug bracket, so each must short-circuit to
        // false. Without the `']' | '］' | '\n'` arm (line 206) the scan
        // would run past the closer, reach the leading `[`, and wrongly
        // report true — so these pin that arm's three alternatives.
        assert!(!in_slug_context("[]#", 3));
        assert!(!in_slug_context("[］#", "[］#".len()));
        assert!(!in_slug_context("[\n#", 3));
    }

    #[test]
    fn slug_context_snaps_window_start_forward_across_multibyte_char() {
        // Force the SLUG_WINDOW look-back cap (256 bytes) to land in the
        // middle of a multi-byte char. Layout (byte ranges):
        //   `あ`      → 0..3
        //   `［`      → 3..6
        //   `あ`×84   → 6..258
        //   `＃`      → 258..261
        // cursor = 261, so `cursor - SLUG_WINDOW` = 5 = the 3rd byte of
        // `［` (bytes 3..6), i.e. mid-codepoint. The correct forward snap
        // starts the window on byte 6 (after `［`), excluding the bracket,
        // so the result is false.
        let src = format!("あ［{}＃", "あ".repeat(84));
        assert_eq!(src.len(), 261);
        let cursor = src.len();
        // This single assertion kills four mutants at once:
        //   * `<`→`==` and `<`→`>` (line 199): the snap loop never runs,
        //     leaving `start` mid-codepoint → `&source[start..cursor]`
        //     panics.
        //   * `+=`→`*=` (line 200): once entered, the snap loop can never
        //     advance → non-termination (caught by cargo-mutants timeout).
        //   * `+=`→`-=` (line 200): the loop snaps BACKWARD onto the start
        //     of `［`, so the window includes the bracket and the result
        //     flips to true.
        assert!(!in_slug_context(&src, cursor));
    }

    #[test]
    fn emmet_item_kind_reflects_snippet_vs_plain() {
        // `kind` must be populated (line 241) and reflect whether the
        // snippet body carries a `${0}` tabstop. Deleting the field
        // defaults it to `None`, failing both arms below.
        let snippet_item = only_item("<", pos(0, 1));
        assert_eq!(snippet_item.kind, Some(CompletionItemKind::SNIPPET));

        let plain_item = only_item("[", pos(0, 1));
        assert_eq!(plain_item.kind, Some(CompletionItemKind::TEXT));
    }

    #[test]
    fn emmet_item_carries_markdown_documentation() {
        // `documentation` must be present (line 243); deleting the field
        // defaults it to `None` and the destructure below panics.
        let item = only_item("[", pos(0, 1));
        let Some(Documentation::MarkupContent(mc)) = item.documentation else {
            panic!("documentation must be Some(MarkupContent), got None/other");
        };
        assert_eq!(mc.kind, MarkupKind::Markdown);
        assert_eq!(mc.value.as_str(), "Half-width `[` → `［`");
    }

    #[test]
    fn emmet_detail_and_doc_localize_by_lang() {
        // Detail prose and the glyph-substitution doc come from the shared
        // catalog, one per locale.
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
        // `preselect` must be `Some(true)` (line 255) so a single Enter
        // accepts the substitution; deleting the field defaults it to
        // `None`.
        let item = only_item("[", pos(0, 1));
        assert_eq!(item.preselect, Some(true));
    }
}
