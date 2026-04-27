//! Gaiji (外字) resolution — mapping `※［＃…、mencode］` references
//! to real Unicode characters.
//!
//! Two incoming shapes per the Aozora annotation manual:
//!
//! ```text
//!   ※［＃「description」、第3水準1-85-54］    ← JIS X 0213 plane-row-cell
//!   ※［＃「description」、U+XXXX、page-line］ ← explicit Unicode codepoint
//! ```
//!
//! The lexer's Phase 3 recogniser (`afm-lexer::phase3_classify::recognize_gaiji`)
//! captures `description` and `mencode` verbatim and leaves `ucs = None`;
//! this module turns that reference into a concrete [`Resolved`] by
//! consulting two `phf::Map`s compiled into the binary
//! ([`JISX0213_MENCODE_TO_CHAR`] for the single-codepoint majority and
//! [`JISX0213_MENCODE_TO_STR`] for the 25 combining-sequence cells)
//! and, for `U+XXXX` shaped mencodes, parsing the hex digits directly.
//!
//! ## Why a `Resolved` enum
//!
//! 25 cells in JIS X 0213:2004 plane 1 (Ainu か゚ family, IPA tone marks,
//! a handful of accented Latin) decode to a *combining sequence* — two
//! Unicode scalars that must travel together. A single `char` cannot
//! carry them, so the resolved value is either a [`char`] (the
//! ~99.4% common path) or a `&'static str` borrowed from the
//! generated combo table. Both variants are `Copy`, so embedding
//! `Option<Resolved>` in [`crate::Gaiji`] does not perturb the
//! parser's `Copy`-able tree.
//!
//! ## Lookup order
//!
//! 1. **`existing`** — the caller-provided codepoint (e.g. extracted
//!    by an earlier escape recogniser); short-circuit identity.
//! 2. **Combo table** — checked first for `mencode` because it is the
//!    only way to honour a 2-codepoint cell.
//! 3. **Single-char table** — the bulk path; one perfect-hash probe
//!    in `.rodata`.
//! 4. **`U+XXXX` prefix** — `U+` followed by 1–6 hex digits. Parsed
//!    as a hex integer, validated via [`char::from_u32`].
//! 5. **Description fallback** — small secondary table keyed by the
//!    literal description text (well-known shapes like 〓, 〻).
//! 6. **None** — unresolved. Renderer falls back to the raw
//!    `description` bytes.
//!
//! ## Why two PHF maps rather than one enum-valued map
//!
//! The single-char map is 4 329 entries; the combo map is 25.
//! Storing the common path as `phf::Map<&str, char>` keeps each value
//! at 4 bytes (vs 16-byte `&str`) and the cache footprint of the hot
//! lookup path tight. The combo map is consulted second; misses
//! there cost a single probe.

use core::fmt;

use crate::jisx0213_table::{
    DESCRIPTION_TO_CHAR, JISX0213_MENCODE_TO_CHAR, JISX0213_MENCODE_TO_STR,
};

/// Resolution outcome — either a single Unicode scalar or a static
/// string covering a combining sequence.
///
/// `Copy` so it can sit inside `Gaiji` without breaking the parser
/// tree's `Copy` chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// Common path: the mencode mapped to a single Unicode scalar
    /// (~99.4% of JIS X 0213:2004 cells, plus all `U+XXXX` shapes
    /// and the description fallback).
    Char(char),
    /// JIS X 0213 combining-sequence cell — 25 entries in plane 1
    /// (Ainu か゚ family, IPA tone marks, accented Latin). The string
    /// is borrowed from a static `phf::Map` value.
    Multi(&'static str),
}

impl Resolved {
    /// Convenience: write the resolved char(s) into any [`fmt::Write`].
    /// Renderer / hover / inlay-hint paths all take this shape.
    ///
    /// # Errors
    /// Propagates the writer's own errors verbatim.
    pub fn write_to<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::Char(c) => w.write_char(c),
            Self::Multi(s) => w.write_str(s),
        }
    }

    /// Returns the resolved single `char` if and only if this is a
    /// [`Resolved::Char`]. Combo cells return `None`.
    #[must_use]
    pub fn as_char(self) -> Option<char> {
        match self {
            Self::Char(c) => Some(c),
            Self::Multi(_) => None,
        }
    }

    /// Total UTF-8 length of the resolved value (1..=8 bytes in
    /// practice).
    #[must_use]
    pub fn utf8_len(self) -> usize {
        match self {
            Self::Char(c) => c.len_utf8(),
            Self::Multi(s) => s.len(),
        }
    }
}

/// Pure-function lookup used by `aozora-lexer`'s Phase 3 classifier
/// to populate `borrowed::Gaiji::ucs` at construction time.
///
/// `existing` is the short-circuit for callers that already extracted
/// a codepoint from the source. Pass `None` to fall through to the
/// table layers.
#[must_use]
pub fn lookup(
    existing: Option<char>,
    mencode: Option<&str>,
    description: &str,
) -> Option<Resolved> {
    if let Some(ch) = existing {
        return Some(Resolved::Char(ch));
    }
    if let Some(m) = mencode {
        // Combo table first: the 25 multi-codepoint cells live only
        // here. A miss is a single PHF probe — cheap.
        if let Some(&s) = JISX0213_MENCODE_TO_STR.get(m) {
            return Some(Resolved::Multi(s));
        }
        if let Some(&ch) = JISX0213_MENCODE_TO_CHAR.get(m) {
            return Some(Resolved::Char(ch));
        }
        if let Some(ch) = parse_u_plus(m) {
            return Some(Resolved::Char(ch));
        }
    }
    if let Some(&ch) = DESCRIPTION_TO_CHAR.get(description) {
        return Some(Resolved::Char(ch));
    }
    // Smart fallback: a description that is *itself* a single
    // character resolves to that character. Common in real corpora
    // when the author CAN type the kanji (e.g. on a modern IME) but
    // wants the reader to see a `※[#…]` annotation pointing at the
    // JIS source. Mencode/dictionary tiers above already short-
    // circuited any case where the table had a more specific answer,
    // so this only fires when description is a one-glyph payload
    // and nothing else matched.
    //
    // Counts grapheme clusters by Unicode scalars: a base-plus-
    // combining sequence (e.g. アクセント分解) returns >1 char and
    // falls through to the final `None`. Surrogate halves can't
    // appear in `&str` so single-`char` is unambiguous here.
    let mut chars = description.chars();
    if let Some(only) = chars.next()
        && chars.next().is_none()
    {
        return Some(Resolved::Char(only));
    }
    None
}

/// Parse a `U+XXXX` style mencode — 1 to 6 hex digits after the
/// literal `U+` prefix — and validate the result via
/// [`char::from_u32`]. Returns `None` for surrogates, non-characters,
/// and out-of-range integers, rather than panicking, so malformed
/// input falls cleanly through to the description fallback.
#[must_use]
fn parse_u_plus(mencode: &str) -> Option<char> {
    let hex = mencode.strip_prefix("U+")?;
    // Reject empty / oversized; `u32::from_str_radix` would accept
    // 10-digit inputs but those can't fit a Unicode scalar.
    if hex.is_empty() || hex.len() > 6 {
        return None;
    }
    let code = u32::from_str_radix(hex, 16).ok()?;
    char::from_u32(code)
}

// Gaiji descriptions (the text inside `「…」`) that resolve to a
// canonical character without depending on the mencode tail. Sourced
// from `crates/aozora-encoding/data/aozora-gaiji-chuki.tsv` (the
// official 8th-edition 外字注記辞書, ~8 800 entries) plus
// `aozora-gaiji-special.tsv` (hand-curated 〓 / 〻 placeholders).
// Generated by `xtask gaiji-gen` and exported from
// `crate::jisx0213_table::DESCRIPTION_TO_CHAR` (alias-imported at
// the top of this module).

/// Pretty-printer for tests and diagnostics. Returns
/// `(single_char_count, combo_count, description_count)`.
#[must_use]
pub fn table_sizes() -> (usize, usize, usize) {
    (
        JISX0213_MENCODE_TO_CHAR.len(),
        JISX0213_MENCODE_TO_STR.len(),
        DESCRIPTION_TO_CHAR.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_prefers_existing_ucs_when_already_set() {
        // The "existing" short-circuit returns the caller-provided
        // codepoint without consulting either table.
        assert_eq!(
            lookup(Some('\u{1234}'), Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{1234}'))
        );
    }

    #[test]
    fn lookup_via_mencode_table_when_ucs_missing() {
        // 罪と罰 fixture: `木＋吶のつくり` with 第3水準1-85-54.
        // Per JIS X 0213:2004 plane 1, row 85, cell 54 = 枘 (U+6798).
        // ("吶のつくり" = right-side component of 吶 = 内, so 木+内 = 枘.)
        assert_eq!(
            lookup(None, Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
    }

    #[test]
    fn lookup_via_combo_table_returns_multi() {
        // 第3水準1-4-87 = か゚ = U+304B U+309A (combining handakuten).
        // The combo path is the *only* way to honour these 25 cells.
        assert_eq!(
            lookup(None, Some("第3水準1-4-87"), ""),
            Some(Resolved::Multi("\u{304B}\u{309A}"))
        );
    }

    #[test]
    fn combo_resolution_writes_both_codepoints() {
        // End-to-end: combo lookup + write_to should yield the full
        // 2-codepoint sequence (6 UTF-8 bytes for か + handakuten).
        let resolved = lookup(None, Some("第3水準1-4-87"), "").expect("combo resolves");
        let mut s = String::new();
        resolved.write_to(&mut s).expect("write to String never fails");
        assert_eq!(s, "\u{304B}\u{309A}");
        assert_eq!(s.chars().count(), 2);
    }

    #[test]
    fn lookup_via_u_plus_form() {
        assert_eq!(
            lookup(None, Some("U+01F5"), "Latin Small Letter G With Acute"),
            Some(Resolved::Char('\u{01F5}'))
        );
    }

    #[test]
    fn lookup_via_u_plus_max_six_hex_digits() {
        // U+10FFFF is the Unicode max; any shape past 6 digits is rejected.
        assert_eq!(
            lookup(None, Some("U+10FFFF"), ""),
            Some(Resolved::Char('\u{10FFFF}'))
        );
    }

    #[test]
    fn lookup_rejects_u_plus_beyond_seven_hex_digits() {
        assert_eq!(lookup(None, Some("U+1234567"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_surrogate() {
        assert_eq!(lookup(None, Some("U+D800"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_non_hex() {
        assert_eq!(lookup(None, Some("U+GG12"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_without_digits() {
        assert_eq!(lookup(None, Some("U+"), ""), None);
    }

    #[test]
    fn lookup_via_description_fallback_when_mencode_absent() {
        assert_eq!(
            lookup(None, None, "〓"),
            Some(Resolved::Char('\u{3013}'))
        );
    }

    #[test]
    fn lookup_returns_none_when_all_paths_miss() {
        // Multi-char description AND missing mencode → no resolution.
        assert_eq!(
            lookup(None, Some("not-in-any-table"), "unresolved gaiji"),
            None
        );
    }

    #[test]
    fn lookup_falls_back_to_description_self_when_single_char() {
        // 丂 is in the JIS X 0213 plane 2 table at row 1 cell 2 — but
        // a real-world author wrote `※[#「丂」、第4水準2-16-1]` with a
        // mencode that doesn't exist in the table. The description IS
        // the kanji itself, so the smart fallback resolves to it.
        assert_eq!(
            lookup(None, Some("第4水準2-16-1"), "丂"),
            Some(Resolved::Char('\u{4E02}'))
        );
        // Same for descriptions with no mencode at all.
        assert_eq!(lookup(None, None, "畺"), Some(Resolved::Char('\u{757A}')));
        assert_eq!(lookup(None, None, "龔"), Some(Resolved::Char('\u{9F94}')));
    }

    #[test]
    fn single_char_fallback_does_not_override_dictionary_hit() {
        // `〓` is in the special-placeholder table mapping to
        // `〓 U+3013`. (Yes, that's a no-op mapping, but it exercises
        // the dictionary path winning over the single-char fallback.)
        // If the fallback fired in spite of the table hit, the
        // dictionary's value would still match here — so the contract
        // is "fallback only fires when nothing else matched".
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn single_char_fallback_does_not_fire_for_multi_char_descriptions() {
        // Multi-char description not in any table → must still be None.
        // Confirms the early-return on `chars.next().is_none()`.
        assert_eq!(lookup(None, None, "未知の字形"), None);
        assert_eq!(lookup(None, None, "ab"), None);
    }

    #[test]
    fn mencode_table_covers_the_fixture_gaiji() {
        // Pin the corrected 罪と罰 fixture mapping (枘 U+6798, not the
        // pre-regen hand-seed's wrong U+6903 椃).
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-54"),
            Some(&'\u{6798}')
        );
    }

    #[test]
    fn table_sizes_match_jisx0213_2004_spec() {
        // Pinned against the JIS X 0213:2004 normative count + the
        // 外字注記辞書 8th edition (8 881 entries) + 2 hand-curated
        // specials (〓 / 〻). Both data sources are checked into
        // `crates/aozora-encoding/data/`.
        use crate::jisx0213_table::{
            DESCRIPTION_COUNT, JISX0213_COMBO_COUNT, JISX0213_PLANE1_COUNT, JISX0213_PLANE2_COUNT,
        };
        let (single, combo, description) = table_sizes();
        assert_eq!(single, JISX0213_PLANE1_COUNT + JISX0213_PLANE2_COUNT);
        assert_eq!(combo, JISX0213_COMBO_COUNT);
        assert_eq!(description, DESCRIPTION_COUNT);
        assert_eq!(
            JISX0213_PLANE1_COUNT, 1893,
            "第3水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_PLANE2_COUNT, 2436,
            "第4水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_COMBO_COUNT, 25,
            "combining-sequence cells must equal spec",
        );
        assert!(
            description >= 8_000,
            "description-fallback table looks too small ({description}) — \
             did the gaiji-chuki extraction drop entries?",
        );
    }

    #[test]
    fn description_table_resolves_a_known_dictionary_entry() {
        // 「木＋吶のつくり」 is a hallmark fixture description for 枘
        // (U+6798, JIS X 0213 plane 1 row 85 cell 54). The dictionary
        // path resolves the same character as the mencode path, so a
        // test with description-only (no mencode) must hit U+6798.
        assert_eq!(
            lookup(None, None, "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}')),
        );
    }

    #[test]
    fn description_table_preserves_special_placeholders() {
        // 〓 / 〻 are hand-curated specials kept in
        // `aozora-gaiji-special.tsv` and merged into the generated map.
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
        assert_eq!(lookup(None, None, "〻"), Some(Resolved::Char('\u{303B}')));
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane1_third_tier_kanji() {
        // 第3水準1-85-9 = 敧 (U+6567) per JIS X 0213:2004.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-9"),
            Some(&'\u{6567}')
        );
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane2_fourth_tier_entry() {
        // 第4水準2-1-1 = 𠂉 (U+20089) — first plane-2 cell.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第4水準2-1-1"),
            Some(&'\u{20089}')
        );
    }

    #[test]
    fn resolved_utf8_len_matches_actual_encoding() {
        assert_eq!(Resolved::Char('A').utf8_len(), 1);
        assert_eq!(Resolved::Char('あ').utf8_len(), 3);
        assert_eq!(Resolved::Char('𠂉').utf8_len(), 4);
        assert_eq!(Resolved::Multi("\u{304B}\u{309A}").utf8_len(), 6);
    }

    #[test]
    fn resolved_as_char_returns_none_for_combos() {
        assert_eq!(Resolved::Char('A').as_char(), Some('A'));
        assert_eq!(Resolved::Multi("か゚").as_char(), None);
    }

    #[test]
    fn lookup_is_identity_on_the_ucs_input_when_set() {
        // The "existing" short-circuit honours the caller-provided
        // scalar without a wasted table probe.
        assert_eq!(
            lookup(Some('あ'), Some("anything"), "anything"),
            Some(Resolved::Char('あ'))
        );
    }
}
