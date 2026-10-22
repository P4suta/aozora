use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

pub(super) const TRIGGERS: &[&str] = &["[", "]", "<", ">", "|", "*", "{", "}"];

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
    Rule {
        typed: '{',
        replacement: "〔",
    },
    Rule {
        typed: '}',
        replacement: "〕",
    },
];

#[must_use]
pub(super) fn format_on_type(source: &str, position: Position, ch: &str) -> Vec<TextEdit> {
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
        assert_eq!(format_on_type("#", pos(0, 1), "#"), Vec::<TextEdit>::new());
        assert_eq!(format_on_type("a", pos(0, 1), "a"), Vec::<TextEdit>::new());
    }

    #[test]
    fn convert_at_end_of_japanese_prose_keeps_offsets_correct() {
        let src = "本文の途中で[";
        let cursor_byte = src.len();
        let position = byte_offset_to_position(src, cursor_byte);
        let edits = format_on_type(src, position, "[");
        assert_eq!(edits.len(), 1);
        let rep_start = position_to_byte_offset(src, edits[0].range.start).unwrap();
        let rep_end = position_to_byte_offset(src, edits[0].range.end).unwrap();
        assert_eq!(rep_end - rep_start, 1);
        assert_eq!(&src[rep_start..rep_end], "[");
        assert_eq!(edits[0].new_text, "［");
    }

    #[test]
    fn cursor_not_at_typed_char_returns_empty() {
        let src = "[]";
        let cursor_byte = src.len();
        let position = byte_offset_to_position(src, cursor_byte);
        assert_eq!(format_on_type(src, position, "["), Vec::<TextEdit>::new());
    }

    #[test]
    fn out_of_bounds_position_returns_empty() {
        assert_eq!(
            format_on_type("abc", pos(99, 99), "["),
            Vec::<TextEdit>::new()
        );
    }

    #[test]
    fn empty_source_returns_empty() {
        assert_eq!(format_on_type("", pos(0, 0), "["), Vec::<TextEdit>::new());
    }

    #[test]
    fn triggers_constant_matches_rule_table() {
        let from_rules: Vec<String> = RULES.iter().map(|r| r.typed.to_string()).collect();
        let from_triggers: Vec<String> = TRIGGERS.iter().map(|&s| s.to_owned()).collect();
        assert_eq!(from_rules, from_triggers);
    }
}
