//! `textDocument/onTypeFormatting` handler — half-width → full-width
//! substitution as the user types.
//!
//! ## Why this is the right LSP feature for the job
//!
//! The completion-popup path ([`crate::half_width_emmet`]) needs the
//! user to accept a suggestion (Enter/Tab) before the substitution
//! lands. In practice — observed in user testing 2026-04-29 — VS
//! Code's fuzzy filter scores single-ASCII triggers like `[` against
//! the full-width labels (`［`) at near-zero, so the popup either
//! does not appear or sinks below other items even with `filter_text`
//! pinned. The popup also adds a confirm-keystroke between every
//! typed char which derails normal Japanese typing flow.
//!
//! `onTypeFormatting` solves both: VS Code fires the request the
//! moment the trigger char is typed, the server replies with a
//! [`TextEdit`] that REPLACES the typed char with its full-width
//! counterpart, no popup, no accept. The result is the snappy
//! "type [, see ［" UX the user wants.
//!
//! ## Safety / why ASCII mangling is not a real concern in aozora
//!
//! The trigger set is `[`, `]`, `<`, `>`, `|`, `*` — every one of
//! these has a dedicated full-width counterpart in aozora notation
//! (`［`, `］`, `《`, `》`, `｜`, `※`) and is **never** intended as
//! a half-width literal in aozora prose. URLs, source code, etc do
//! not appear in well-formed aozora text; if the user genuinely
//! wants a literal half-width char they can undo with `Ctrl+Z` (the
//! `TextEdit` is one undo step) or disable formatOnType for that
//! moment.
//!
//! `#` is deliberately NOT a trigger — `#` appears in URLs and is
//! never load-bearing on its own; the slug catalogue handles the
//! `[#` → slug-catalogue popup flow downstream.

use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::position::{byte_offset_to_position, position_to_byte_offset};

/// The trigger char list advertised in `documentOnTypeFormattingProvider`.
///
/// The server capability splits this into `first_trigger_character` +
/// `more_trigger_character`; this constant keeps the canonical list in
/// one place so `backend.rs` and tests cannot drift.
pub const TRIGGERS: &[&str] = &["[", "]", "<", ">", "|", "*", "{", "}"];

/// Per-char substitution rule. Pure data so the table is trivial to
/// audit. The `replacement` field is what gets spliced over the typed
/// char.
struct Rule {
    typed: char,
    replacement: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        typed: '[',
        replacement: "［",
    },
    Rule {
        typed: ']',
        replacement: "］",
    },
    Rule {
        typed: '<',
        replacement: "《",
    },
    Rule {
        typed: '>',
        replacement: "》",
    },
    Rule {
        typed: '|',
        replacement: "｜",
    },
    Rule {
        typed: '*',
        replacement: "※",
    },
    // `{` / `}` → 亀甲括弧 `〔` / `〕` (kikkou). Used for accent-
    // decomposition annotations like `〔café〕`. Picking the brace
    // keys (rather than the round-paren keys) preserves `(` / `)`
    // for parenthetical asides written in either width — those stay
    // as-typed because aozora-bunko texts mix half- and full-width
    // parens depending on context.
    Rule {
        typed: '{',
        replacement: "〔",
    },
    Rule {
        typed: '}',
        replacement: "〕",
    },
];

/// Compute the on-type substitution edit for `ch` typed at `position`.
///
/// Returns at most one [`TextEdit`] that replaces the just-typed
/// half-width char with its full-width counterpart. Returns an empty
/// vec when:
///
/// - `ch` is not in the trigger table (defensive — VS Code only fires
///   us for chars we declared in the capability, but be safe);
/// - the cursor / preceding-byte invariant doesn't hold (e.g. position
///   converts to an out-of-bounds offset);
/// - the byte immediately before the cursor doesn't actually match
///   `ch` (e.g. the editor auto-inserted a paired bracket between
///   the typed char and the cursor — the LSP spec explicitly
///   warns this can happen).
#[must_use]
pub fn format_on_type(source: &str, position: Position, ch: &str) -> Vec<TextEdit> {
    let Some(rule) = lookup_rule(ch) else {
        return Vec::new();
    };
    let Some(cursor) = position_to_byte_offset(source, position) else {
        return Vec::new();
    };
    let typed_len = rule.typed.len_utf8();
    if cursor < typed_len {
        return Vec::new();
    }
    let typed_start = cursor - typed_len;
    // Defensive: confirm the byte just before the cursor is the char
    // VS Code claims was typed. The LSP spec notes the editor may
    // auto-insert (e.g. paired brackets) between the typed char and
    // the cursor; we only convert when our trigger really sits at
    // the cursor's left.
    if !source.is_char_boundary(typed_start) || !source[typed_start..].starts_with(rule.typed) {
        return Vec::new();
    }
    let range = Range::new(
        byte_offset_to_position(source, typed_start),
        byte_offset_to_position(source, cursor),
    );
    vec![TextEdit {
        range,
        new_text: rule.replacement.to_owned(),
    }]
}

fn lookup_rule(ch: &str) -> Option<&'static Rule> {
    let mut chars = ch.chars();
    let only = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    RULES.iter().find(|r| r.typed == only)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, col: u32) -> Position {
        Position::new(line, col)
    }

    #[test]
    fn left_bracket_replaces_in_place() {
        let edits = format_on_type("[", pos(0, 1), "[");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "［");
        assert_eq!(edits[0].range.start, pos(0, 0));
        assert_eq!(edits[0].range.end, pos(0, 1));
    }

    #[test]
    fn every_trigger_round_trips_to_its_full_width_form() {
        // Lock in the exact mapping the user-facing UX depends on.
        let cases = [
            ("[", "［"),
            ("]", "］"),
            ("<", "《"),
            (">", "》"),
            ("|", "｜"),
            ("*", "※"),
            ("{", "〔"),
            ("}", "〕"),
        ];
        for (typed, expected) in cases {
            let edits = format_on_type(typed, pos(0, 1), typed);
            assert_eq!(edits.len(), 1, "no edit for typed={typed}");
            assert_eq!(
                edits[0].new_text, expected,
                "wrong replacement for typed={typed}",
            );
        }
    }

    #[test]
    fn unknown_char_returns_empty() {
        // `#` is not a trigger — slug catalogue handles that downstream.
        assert!(format_on_type("#", pos(0, 1), "#").is_empty());
        assert!(format_on_type("a", pos(0, 1), "a").is_empty());
    }

    #[test]
    fn convert_at_end_of_japanese_prose_keeps_offsets_correct() {
        // Replicates real flow: user is mid-sentence in Japanese
        // and types `[` at the end. The TextEdit must address the
        // single byte just typed, not the multi-byte Japanese chars
        // before it.
        let src = "本文の途中で[";
        let cursor_byte = src.len();
        let position = byte_offset_to_position(src, cursor_byte);
        let edits = format_on_type(src, position, "[");
        assert_eq!(edits.len(), 1);
        // The replaced range must be exactly the typed `[` — one
        // ASCII byte at the very end.
        let rep_start = position_to_byte_offset(src, edits[0].range.start).unwrap();
        let rep_end = position_to_byte_offset(src, edits[0].range.end).unwrap();
        assert_eq!(rep_end - rep_start, 1);
        assert_eq!(&src[rep_start..rep_end], "[");
        assert_eq!(edits[0].new_text, "［");
    }

    #[test]
    fn cursor_not_at_typed_char_returns_empty() {
        // The editor may auto-insert between the typed char and the
        // cursor (e.g. autoClosingPairs). When the byte just before
        // the cursor doesn't match `ch`, we MUST NOT convert; the
        // editor's view of the document and ours will be at odds.
        let src = "[]";
        let cursor_byte = src.len(); // cursor sits AFTER `]`, not after `[`
        let position = byte_offset_to_position(src, cursor_byte);
        // We claim `[` was typed but the byte before cursor is `]` —
        // refuse to act.
        assert!(format_on_type(src, position, "[").is_empty());
    }

    #[test]
    fn out_of_bounds_position_returns_empty() {
        // Beyond-EOF position safely yields no edit instead of panicking.
        assert!(format_on_type("abc", pos(99, 99), "[").is_empty());
    }

    #[test]
    fn empty_source_returns_empty() {
        // No char to convert — defensive.
        assert!(format_on_type("", pos(0, 0), "[").is_empty());
    }

    #[test]
    fn triggers_constant_matches_rule_table() {
        // Lock-in: the advertised capability and the substitution
        // table must agree — drift between them would mean VS Code
        // fires us for chars we don't handle, or doesn't fire us
        // for chars we DO handle. Both are silent UX bugs.
        let from_rules: Vec<String> = RULES.iter().map(|r| r.typed.to_string()).collect();
        let from_triggers: Vec<String> = TRIGGERS.iter().map(|&s| s.to_owned()).collect();
        assert_eq!(from_rules, from_triggers);
    }
}
