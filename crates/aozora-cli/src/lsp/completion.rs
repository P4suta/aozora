//! `textDocument/completion` handler.
//!
//! When the cursor sits in or just after either:
//!
//! - `［＃` — the canonical full-width slug-open digraph, or
//! - `[#`  — the half-width ASCII alternative typists reach for first,
//!
//! suggest every entry from the canonical [`aozora::Catalogue`] table.
//! Each suggestion carries:
//!
//! - `label`: the canonical body text the editor displays.
//! - `kind`: a [`CompletionItemKind`] picked from the slug's family.
//! - `detail` / `documentation`: short Japanese description from the
//!   slug entry's `doc` field.
//! - `text_edit`: a [`TextEdit`] that replaces the typed prefix-plus-
//!   body with the full canonical form `［＃canonical］` (or just
//!   `canonical` when the user already has the brackets). Half-width
//!   prefixes are auto-converted to full-width on accept — no
//!   separate "convert to ［＃" code action needed.
//! - `additional_text_edits`: when the slug is paired
//!   (`BlockContainerOpen` etc.) and the source does not already
//!   contain a matching close, append the close marker on a fresh
//!   line so the editor can land both in one accept.

use crate::i18n::{self as i18n, LanguageIdentifier};
use aozora::{Catalogue, CatalogueEntry as SlugEntry, CatalogueFamily as SlugFamily};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position, Range, TextEdit,
};

use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

// We deliberately fire the catalogue from the very first byte of body
// — the editor's own fuzzy-match layer narrows from there. No minimum
// prefix length gate is applied (any "trigger after N keystrokes"
// behaviour belongs to the client side, not the server).

/// Recognised slug-open digraphs.
///
/// Four variants, not two: when `onTypeFormatting`
/// ([`crate::lsp::on_type_formatting`]) auto-converts `[` to `［` but
/// leaves `#` alone (we don't auto-convert `#` because it appears
/// in URLs and citations), the user can briefly land in a mixed
/// `［#` state. The reverse mix `[＃` happens when the user types
/// `[` then a Japanese-IME-converted `＃`. Both should fire the
/// slug catalogue popup. Order matters only for `rfind`
/// disambiguation when multiple forms are present in
/// `source[..cursor]` — we pick whichever sits closer to the
/// cursor.
const OPENERS: &[&str] = &["［＃", "[#", "［#", "[＃"];

/// True when `opener` came from the half-width column for either
/// of its two characters — i.e. the on-accept text edit needs to
/// rewrite at least one of them to its full-width form so the source
/// ends up canonical.
fn opener_is_half_width(opener: &str) -> bool {
    opener != "［＃"
}

/// Resolved slug context for a cursor position.
struct SlugCtx {
    /// Byte offset where the opener (one of [`OPENERS`]) starts.
    prefix_start: usize,
    /// Byte offset where the body starts (just after the opener).
    body_start: usize,
    /// Byte offset where the (existing) close `]` / `］` starts, or
    /// the cursor position when no close has been typed yet. The
    /// completion `text_edit` replaces the range
    /// `prefix_start..close_end` with the full canonical form.
    close_end: usize,
    /// The actual opener string (`［＃` / `[#` / `［#` / `[＃`).
    /// Used to build a `filter_text` that exactly matches whatever
    /// the user has typed so far so VS Code's filter doesn't drop
    /// our items on a one-codepoint mismatch.
    opener: &'static str,
}

impl SlugCtx {
    /// True when at least one of the opener's two codepoints is
    /// half-width — i.e. the on-accept text edit needs to rewrite
    /// the whole opener+body to the canonical full-width form.
    fn half_width(&self) -> bool {
        opener_is_half_width(self.opener)
    }
}

/// Compute completion items at `position` in `source`. Returns an
/// empty Vec if the position is not in a slug context.
///
/// Tree-free: the slug-context detection is a small look-back scan
/// from the cursor (cap 256 bytes), so this handler is `O(window)`
/// regardless of document size. A future iteration that filters the
/// catalogue by enclosing container kind can re-introduce the
/// tree-sitter tree as a parameter.
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

    // Return the full catalogue and let the LSP client's own
    // fuzzy-match layer filter — this is the norm for completion
    // providers (rust-analyzer, gopls, …) and avoids accidentally
    // hiding entries when the user types in latin transliteration
    // (`[#bouten` should still show 傍点).
    Catalogue::directives()
        .iter()
        .map(|entry| build_completion_item(source, entry, &ctx, lang))
        .collect()
}

/// Find the latest slug opener (any of [`OPENERS`]) before `cursor`,
/// classify whether the close has been typed, and bail out when the
/// cursor sits past an already-closed slug.
fn resolve_slug_context(source: &str, cursor: usize) -> Option<SlugCtx> {
    let prefix = &source[..cursor];
    // Pick the latest-positioned opener regardless of variant.
    // `rfind` is O(n) per call but the openers are 2-codepoint;
    // for the cap-256 look-back this stays well under a microsecond.
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
    // Bail when the user has already closed *this* slug between the
    // opener and the cursor (the prefix is not the *current* slug
    // context — the cursor is past it).
    if body_so_far.contains('］') || body_so_far.contains(']') {
        return None;
    }
    // Look forward for a matching close. We accept either ] or ］ as
    // a candidate close, stopping at the first newline (slug bodies
    // never span lines in well-formed Aozora).
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
    // `entry.doc` is authored Japanese from the aozora slug catalogue (data,
    // not LSP-owned prose); only the half-width hint suffix is localized.
    let detail = if ctx.half_width() {
        format!(
            "{}  {}",
            entry.doc,
            i18n::t(lang, "lsp-completion-half-to-full-hint")
        )
    } else {
        entry.doc.to_owned()
    };
    // VS Code matches typed input against `filter_text || label`.
    // Our `text_edit.range` covers from the opener (any of OPENERS)
    // through whatever the user has typed, so VS Code's filter sees
    // e.g. "[#改ペ" / "［＃改ペ" / "［#改ペ" as the input. The
    // bare `label` (`改ページ`) doesn't start with the opener, so
    // the fuzzy matcher scores zero and the popup hides every
    // suggestion. Concatenating the actual opener + canonical lets
    // the matcher see the typed opener as a literal prefix match
    // and ranks the body characters via fuzzy on top — and crucially
    // works for the mixed-opener variants (`[＃` / `［#`) that
    // appear briefly while `onTypeFormatting` is converting one
    // bracket but the other hasn't been typed yet.
    let filter_text = format!("{}{}", ctx.opener, entry.canonical);
    let mut item = CompletionItem {
        label: entry.canonical.to_owned(),
        filter_text: Some(filter_text),
        kind: Some(family_to_kind(entry.family)),
        detail: Some(detail),
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
        ..CompletionItem::default()
    };

    // Build the body text (snippet for parametric entries, plain
    // otherwise). The body text is the slug's canonical form sans
    // brackets.
    let (body_text, format) = if entry.accepts_param {
        (
            canonical_to_snippet(entry.canonical),
            InsertTextFormat::SNIPPET,
        )
    } else {
        (entry.canonical.to_owned(), InsertTextFormat::PLAIN_TEXT)
    };

    // Detect the "full-width opener already has a full-width close"
    // case once, before deriving the replacement text and range from
    // it. The branch matters in both places because the existing
    // close must stay outside the replaced range — `new_text` is the
    // body alone, and `edit_end` aims at the byte before the close
    // so accepting a completion inside `［＃｜］` does not collapse
    // the existing `］`. The half-open `TextEdit.range` semantics
    // are explicit in the LSP spec (start inclusive, end exclusive);
    // we must aim end at the
    // START of `］`, not its END.
    let existing_full_close_start: Option<usize> = (!ctx.half_width()
        && source
            .get(ctx.body_start..ctx.close_end)
            .is_some_and(|body| body.ends_with('］')))
    .then(|| ctx.close_end - '］'.len_utf8());

    // Splice into the document. For half-width openers, rewrite the
    // entire `[#...]` (or mixed variant) to the canonical full-width
    // form. For full-width openers, decide based on whether the
    // close has been typed yet.
    let new_text = if ctx.half_width() {
        format!("［＃{body_text}］")
    } else if existing_full_close_start.is_some() {
        // Body-only — the close is already in place and we'll stop
        // the replacement range just before it.
        body_text
    } else {
        // Full-width opener but no close yet — emit the body and the
        // close so the user lands a balanced slug.
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

    // For BlockContainerOpen entries, append the matching close on a
    // fresh line so the user lands both halves in one accept. We
    // don't auto-insert when the partner is already present; a quick
    // forward scan covers the common case.
    if entry.family == SlugFamily::BlockContainerOpen
        && let Some(partner) = entry.partner
        && !forward_already_has(source, ctx.close_end, partner)
    {
        let pos = byte_offset_to_position(source, ctx.close_end);
        item.additional_text_edits = Some(vec![TextEdit {
            range: Range::new(pos, pos),
            // Two-line gap before the close so the body is visually
            // distinct. The editor's auto-format pass collapses extra
            // newlines if the user prefers tighter spacing.
            new_text: format!("\n\n［＃{partner}］"),
        }]);
    }
    item
}

/// Translate a canonical slug like `ここから{N}字下げ` into the LSP
/// snippet form `ここから${1:N}字下げ`. Plain text without `{...}`
/// pass-through unchanged. `{path}` becomes `${1:path}`.
pub(crate) fn canonical_to_snippet(canonical: &str) -> String {
    let mut out = String::with_capacity(canonical.len() + 4);
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

/// Quick forward scan: does `source[cursor..]` already contain
/// `partner` before EOF? Used to decide whether the
/// `BlockContainerOpen` completion should auto-insert the close.
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
        // SlugFamily is `#[non_exhaustive]`; future families default
        // to TEXT until a kind is chosen.
        _ => CompletionItemKind::TEXT,
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    /// Shim mirroring the pre-i18n `completion_at(source, position)` arity,
    /// pinned to English so the detail assertions below stay locale-stable.
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
        // Half-width entries get a hint in the detail.
        assert!(
            items
                .iter()
                .all(|i| i.detail.as_deref().unwrap_or("").contains("half-width")),
            "half-width hint missing in detail",
        );
    }

    #[test]
    fn half_width_hint_localizes_by_lang() {
        // The half-width→full-width detail hint comes from the shared catalog.
        // The slug `doc` (Japanese, from the aozora catalogue) is data and is
        // unchanged; only the parenthetical hint follows the locale.
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
        // `[#bouten]` — user typed both halves but in ASCII. Accepting
        // a completion should swap to `［＃改ページ］` (the matched
        // close is part of the replacement range).
        let src = "[#bouten]";

        // Cursor immediately after `bouten`.
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
        // The text_edit's range must cover the full `[#bouten]`.
        assert_eq!(edit.range.start, byte_offset_to_position(src, 0));
        assert_eq!(edit.range.end, byte_offset_to_position(src, src.len()));
    }

    #[test]
    fn full_width_completion_keeps_brackets_in_place() {
        // User has `［＃改］` and the cursor inside the body. Accepting
        // a completion should replace just the body, leaving the
        // brackets untouched.
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
        // Body text only — no surrounding brackets.
        assert_eq!(edit.new_text, "改ページ");
        // Regression: the range must end at the START of `］`, not after
        // it. LSP `TextEdit.range` is end-exclusive, so a body-only
        // new_text with end aimed past `］` silently collapses the close.
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
        // The exact UX flow the user hits: snippetTrigger expands
        // `#` to `［＃${0}］` placing the cursor between `＃` and
        // `］`, the editor fires `triggerSuggest`, and the user
        // accepts a parametric slug like `N字下げ`. The final text
        // must read `［＃ここから1字下げ］` — the close MUST survive.
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
        // Snippet form because the entry is parametric.
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
        // Range MUST be a zero-width insertion at the cursor (right
        // before `］`); never extend across `］`.
        let cursor_byte = "［＃".len();
        assert_eq!(edit.range.start, byte_offset_to_position(src, cursor_byte));
        assert_eq!(edit.range.end, byte_offset_to_position(src, cursor_byte));
    }

    #[test]
    fn completion_in_plain_text_returns_empty() {
        let src = "no annotation context";

        let pos = byte_offset_to_position(src, 4);
        assert!(completion_at(src, pos).is_empty());
    }

    #[test]
    fn half_width_completion_carries_filter_text_with_opener_prefix() {
        // VS Code's filter sees the typed input verbatim (`[#改ペ`).
        // Without filter_text the bare `label` (`改ページ`) doesn't
        // start with `[#`, the fuzzy matcher scores zero, and the
        // popup hides every entry. Pin the prefix so the matcher
        // sees `[#改ページ` and can rank by the body chars.
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
        // The transient state when `onTypeFormatting` has converted
        // `[` to `［` but the user has just typed `#` (which we
        // deliberately don't auto-convert to avoid mangling URLs).
        // The catalogue MUST still fire so the user's flow isn't
        // interrupted by a one-codepoint mismatch.
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
        // Reverse mix: `[` typed normally, then a Japanese-IME-
        // converted `＃` lands. Same requirement.
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
        // Regardless of which bracket the user typed first, accepting
        // a suggestion must leave the source fully canonical
        // (`［＃canonical］`) so subsequent serialisation /
        // diagnostics see one shape, not three.
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
        // The slug already closed before the cursor — no current open
        // context.
        let src = "前［＃改ページ］後";

        let pos = byte_offset_to_position(src, src.len());
        assert!(completion_at(src, pos).is_empty());
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

    // --- resolve_slug_context forward-scan offset math ---------------------

    #[test]
    fn newline_stops_forward_close_scan() {
        // The forward close scan MUST stop at the first newline — slug
        // bodies never span lines. Cursor sits right after `＃` with the
        // `］` on the *next* line. With the `'\n'` arm, close_end pins to
        // the cursor (no existing close → the completion supplies its own
        // `］` and the range ends at the cursor). Delete that arm and the
        // scan runs past the newline, adopts the next-line `］` as this
        // slug's close, emits a body-only new_text and pulls the range
        // end onto line 1.
        let src = "［＃\n］";
        let cursor = "［＃".len(); // byte 6, just after ＃
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
        // The `］` arm computes close_end = cursor + rel + len_utf8('］')
        // with a body char (`あ`) between the cursor and the close, so
        // rel > 0 — this catches a `+`→`-` flip in either addend that a
        // rel==0 case leaves equivalent. Correct math finds the existing
        // full-width close, so new_text is body-only and the range ends
        // exactly at the START of `］`.
        let src = "［＃あ］";
        let cursor = "［＃".len(); // byte 6
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
        let close_start = "［＃あ".len(); // byte 9 = START of ］
        assert_eq!(edit.range.end, byte_offset_to_position(src, close_start));
    }

    #[test]
    fn ascii_close_offset_covers_through_close() {
        // The `]` (half-width close) arm computes
        // close_end = cursor + rel + len_utf8(']') with rel > 0 (body
        // char `あ` precedes the `]`). A full-width opener paired with an
        // ASCII close is NOT treated as an existing canonical close, so
        // the whole span through the `]` is replaced and the range end
        // must reach the byte AFTER `]`. A `+`→`-` flip shrinks close_end
        // and pulls the range end back.
        let src = "［＃あ]";
        let cursor = "［＃".len(); // byte 6
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

    // --- CompletionItem field population -----------------------------------

    #[test]
    fn completion_item_carries_family_kind() {
        // The `kind` field must be populated from the slug family;
        // deleting it defaults to None and strips the per-family icon.
        // 改ページ is PageBreak → EVENT.
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
        // The `documentation` field must be present and embed the slug's
        // doc string; deleting the field defaults it to None.
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
    }

    #[test]
    fn half_width_existing_full_close_does_not_shrink_replacement_range() {
        // `[#あ］` — half-width opener, full-width close typed. The
        // `existing_full_close_start` guard is gated on `!half_width()`,
        // so for a half-width opener it MUST stay None: the whole
        // `[#あ］` span (through the `］`) is rewritten to the canonical
        // form. Flipping either `&&` to `||` in that guard treats the
        // close as a pre-existing full-width close and pulls the range
        // end back to the START of `］`, doubling the bracket.
        let src = "[#あ］";
        let cursor = "[#".len(); // byte 2
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

    // --- canonical_to_snippet placeholder-brace consumption ----------------

    #[test]
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "`${1:N}` is LSP snippet placeholder syntax in the expected output, not a Rust format arg"
    )]
    fn snippet_consumes_exactly_through_the_placeholder_brace() {
        // After emitting `${1:N}` for a `{N}` placeholder the scanner
        // advances `chars` past the closing `}` — the `j > i + close_rel`
        // guard consumes up to AND including the `}` and no further.
        // `==`/`>=` leave the `}` un-consumed (stray `}` in output); `<`
        // stops before the placeholder body char (re-emits it verbatim).
        assert_eq!(canonical_to_snippet("{N}字下げ"), "${1:N}字下げ");
    }

    // --- forward_already_has partner detection -----------------------------

    #[test]
    fn forward_already_has_detects_partner_presence() {
        // True when `partner` occurs at/after `cursor`; used to suppress
        // the auto-inserted close. A body replaced by `false` would never
        // detect an existing close and always duplicate it.
        assert!(forward_already_has(
            "本文\n［＃ここで字下げ終わり］",
            0,
            "ここで字下げ終わり",
        ));
        // Real forward content but the partner is genuinely absent.
        assert!(!forward_already_has(
            "本文［＃ここから字下げ］\nつづき",
            0,
            "ここで字下げ終わり",
        ));
        // Scans FORWARD only: the partner sits before the cursor here.
        let src = "［＃ここで字下げ終わり］";
        assert!(!forward_already_has(src, src.len(), "ここで字下げ終わり"));
    }

    // --- family_to_kind arm mapping ----------------------------------------

    #[test]
    fn family_to_kind_maps_every_family_to_its_icon() {
        // Each arm pins one family (or family pair) to a distinct icon;
        // deleting any arm falls its family through to the `_ => TEXT`
        // default, so every mapping below is load-bearing.
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
}
