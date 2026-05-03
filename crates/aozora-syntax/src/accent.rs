//! Aozora Bunko accent decomposition — ASCII digraph → Unicode letter.
//!
//! Spec: <https://www.aozora.gr.jp/accent_separation.html>
//!
//! The scheme encodes accented Latin letters using a base ASCII letter followed
//! by a one-character marker. The full 118-entry table from the spec is
//! encoded here as a compile-time slice so the lexer (for pre-parse
//! rewriting) and downstream tools share the same authoritative lookup.
//!
//! ```
//! use aozora_syntax::accent::decompose_fragment;
//! assert_eq!(decompose_fragment("fune`bre"), "funèbre");
//! assert_eq!(decompose_fragment("ae&on"), "æon");
//! assert_eq!(decompose_fragment("plain"), "plain");
//! ```
//!
//! # Invariants
//!
//! - The table is closed: no ASCII digraph maps to more than one Unicode
//!   codepoint. Longest-match on ligatures first (`ae&`, `AE&`, `oe&`, `OE&`)
//!   then single-letter digraphs.
//! - `decompose_fragment` may **grow** the byte length of some substrings
//!   (`m'` = ḿ, `e~` = ẽ are BMP codepoints ≥ U+1E00 whose UTF-8 forms are
//!   3 bytes — larger than their 2-byte ASCII digraphs). Callers that back-map
//!   diagnostic spans across the rewrite must record a per-position delta.
//!
//! # Scope of use
//!
//! The function is **only safe to call on the body of a `〔...〕` span**:
//! aozora restricts accent decomposition to that convention to avoid
//! false-matching English text like `text,` (which would otherwise be
//! decomposed to `texţ` via the legitimate-in-Polish `t,` = ţ entry).

use std::borrow::Cow;

/// The full accent decomposition table in spec-page order.
///
/// Public for downstream iteration (tests, doc-builders, corpus
/// tooling). For runtime lookup, `decompose_fragment` uses the
/// perfect-hash split tables (`ACCENT_DIGRAPHS` for the 110 two-byte
/// entries; a 4-arm match for the four three-byte ligatures) — the
/// linear `ACCENT_TABLE` scan is no longer on the hot path.
pub const ACCENT_TABLE: &[(&str, char)] = &[
    // --- Ligatures (checked first: 3-char patterns beat the 2-char group) ---
    ("ae&", 'æ'),
    ("AE&", 'Æ'),
    ("oe&", 'œ'),
    ("OE&", 'Œ'),
    ("s&", 'ß'), // eszett — `&` on `s` is a ligature, not ring-above
    // --- 【a】 ---
    ("a`", 'à'),
    ("a'", 'á'),
    ("a^", 'â'),
    ("a~", 'ã'),
    ("a:", 'ä'),
    ("a&", 'å'),
    ("a_", 'ā'),
    // --- 【c】 ---
    ("c,", 'ç'),
    ("c'", 'ć'),
    ("c^", 'ĉ'),
    // --- 【d】 ---
    ("d/", 'đ'),
    // --- 【e】 ---
    ("e`", 'è'),
    ("e'", 'é'),
    ("e^", 'ê'),
    ("e:", 'ë'),
    ("e_", 'ē'),
    ("e~", 'ẽ'),
    // --- 【g】 ---
    ("g^", 'ĝ'),
    // --- 【h】 ---
    ("h^", 'ĥ'),
    ("h/", 'ħ'),
    // --- 【i】 ---
    ("i`", 'ì'),
    ("i'", 'í'),
    ("i^", 'î'),
    ("i:", 'ï'),
    ("i_", 'ī'),
    ("i/", 'ɨ'),
    ("i~", 'ĩ'),
    // --- 【j】 ---
    ("j^", 'ĵ'),
    // --- 【l】 ---
    ("l/", 'ł'),
    ("l'", 'ĺ'),
    // --- 【m】 ---
    ("m'", 'ḿ'),
    // --- 【n】 ---
    ("n`", 'ǹ'),
    ("n~", 'ñ'),
    ("n'", 'ń'),
    // --- 【o】 ---
    ("o`", 'ò'),
    ("o'", 'ó'),
    ("o^", 'ô'),
    ("o~", 'õ'),
    ("o:", 'ö'),
    ("o/", 'ø'),
    ("o_", 'ō'),
    // --- 【r】 ---
    ("r'", 'ŕ'),
    // --- 【s】 ---
    ("s'", 'ś'),
    ("s,", 'ş'),
    ("s^", 'ŝ'),
    // --- 【t】 ---
    ("t,", 'ţ'),
    // --- 【u】 ---
    ("u`", 'ù'),
    ("u'", 'ú'),
    ("u^", 'û'),
    ("u:", 'ü'),
    ("u_", 'ū'),
    ("u&", 'ů'),
    ("u~", 'ũ'),
    // --- 【y】 ---
    ("y'", 'ý'),
    ("y:", 'ÿ'),
    // --- 【z】 ---
    ("z'", 'ź'),
    // --- 【A】 ---
    ("A`", 'À'),
    ("A'", 'Á'),
    ("A^", 'Â'),
    ("A~", 'Ã'),
    ("A:", 'Ä'),
    ("A&", 'Å'),
    ("A_", 'Ā'),
    // --- 【C】 ---
    ("C,", 'Ç'),
    ("C'", 'Ć'),
    ("C^", 'Ĉ'),
    // --- 【D】 ---
    ("D/", 'Đ'),
    // --- 【E】 ---
    ("E`", 'È'),
    ("E'", 'É'),
    ("E^", 'Ê'),
    ("E:", 'Ë'),
    ("E_", 'Ē'),
    ("E~", 'Ẽ'),
    // --- 【G】 ---
    ("G^", 'Ĝ'),
    // --- 【H】 ---
    ("H^", 'Ĥ'),
    // --- 【I】 ---
    ("I`", 'Ì'),
    ("I'", 'Í'),
    ("I^", 'Î'),
    ("I:", 'Ï'),
    ("I_", 'Ī'),
    ("I~", 'Ĩ'),
    // --- 【J】 ---
    ("J^", 'Ĵ'),
    // --- 【L】 ---
    ("L/", 'Ł'),
    ("L'", 'Ĺ'),
    // --- 【M】 ---
    ("M'", 'Ḿ'),
    // --- 【N】 ---
    ("N`", 'Ǹ'),
    ("N~", 'Ñ'),
    ("N'", 'Ń'),
    // --- 【O】 ---
    ("O`", 'Ò'),
    ("O'", 'Ó'),
    ("O^", 'Ô'),
    ("O~", 'Õ'),
    ("O:", 'Ö'),
    ("O/", 'Ø'),
    ("O_", 'Ō'),
    // --- 【R】 ---
    ("R'", 'Ŕ'),
    // --- 【S】 ---
    ("S'", 'Ś'),
    ("S,", 'Ş'),
    ("S^", 'Ŝ'),
    // --- 【T】 ---
    ("T,", 'Ţ'),
    // --- 【U】 ---
    ("U`", 'Ù'),
    ("U'", 'Ú'),
    ("U^", 'Û'),
    ("U:", 'Ü'),
    ("U_", 'Ū'),
    ("U&", 'Ů'),
    ("U~", 'Ũ'),
    // --- 【Y】 ---
    ("Y'", 'Ý'),
    // --- 【Z】 ---
    ("Z'", 'Ź'),
];

/// ASCII characters that act as accent markers in the spec.
///
/// Kept as a `&[u8]` slice for downstream consumers that want to
/// enumerate the marker bytes; runtime membership checks go through
/// the `u128` bitmap `ACCENT_MARKER_MASK` instead, which lowers to a
/// single shift + AND.
pub const ACCENT_MARKERS: &[u8] = b"'`^:~&,/_";

/// 128-bit bitmap of [`ACCENT_MARKERS`] for branchless ASCII membership
/// testing. Bit `n` is 1 iff byte `n` is an accent marker. Computed at
/// compile time from [`ACCENT_MARKERS`] so the two stay in lockstep.
const ACCENT_MARKER_MASK: u128 = {
    let mut m: u128 = 0;
    let bs = ACCENT_MARKERS;
    let mut i = 0;
    while i < bs.len() {
        // All marker bytes are < 128 (ASCII). Compile-time-asserted by
        // the const block below.
        m |= 1u128 << bs[i];
        i += 1;
    }
    m
};

const _: () = {
    // Pin the marker set to ASCII; if a future spec edit adds a non-ASCII
    // marker the bitmap shape must change (no longer fits in u128).
    let bs = ACCENT_MARKERS;
    let mut i = 0;
    while i < bs.len() {
        assert!(bs[i] < 128, "ACCENT_MARKERS must stay ASCII-only");
        i += 1;
    }
};

/// Branchless membership test against [`ACCENT_MARKERS`].
///
/// Compiles to `(b < 128) & ((MASK >> b) & 1)` — one cmp, one shift,
/// one AND — no memory load, no loop, no branch. Replaces the prior
/// `ACCENT_MARKERS.contains(&b)` linear scan over 9 bytes.
#[inline]
#[must_use]
pub const fn is_accent_marker(b: u8) -> bool {
    // `b as u32` to avoid `1u128 << 200` overflow if a non-ASCII byte
    // were ever passed; the AND with the high mask is 0 there anyway,
    // but the shift itself UB without the guard.
    (b < 128) && ((ACCENT_MARKER_MASK >> b) & 1) != 0
}

/// 3-byte ligatures (ASCII keys → Latin char). Only four entries, so a
/// `match` beats `phf::Map` here: the compiler lowers it to a small
/// jump table, branch prediction nails the common ASCII miss path, and
/// the `match` keeps the keys inlined as immediates rather than
/// reaching out to a static array.
#[inline]
fn match_ligature(head: &[u8]) -> Option<char> {
    debug_assert_eq!(head.len(), 3, "match_ligature requires exactly 3 bytes");
    match head {
        b"ae&" => Some('æ'),
        b"AE&" => Some('Æ'),
        b"oe&" => Some('œ'),
        b"OE&" => Some('Œ'),
        _ => None,
    }
}

/// 2-byte digraphs as a compile-time perfect hash table. 110 entries,
/// `&[u8]` keys (the 2 ASCII bytes), `char` values. `phf::Map::get` is
/// O(1) and constant-comparison-bounded, replacing the 110-entry
/// linear scan that the old `ACCENT_TABLE` lookup used.
static ACCENT_DIGRAPHS: phf::Map<&'static [u8], char> = phf::phf_map! {
    // s& is grouped as a "ligature" on the spec page but is 2 bytes;
    // it lives here in the digraph map alongside the rest.
    b"s&" => 'ß',
    // --- 【a】 ---
    b"a`" => 'à', b"a'" => 'á', b"a^" => 'â', b"a~" => 'ã',
    b"a:" => 'ä', b"a&" => 'å', b"a_" => 'ā',
    // --- 【c】 ---
    b"c," => 'ç', b"c'" => 'ć', b"c^" => 'ĉ',
    // --- 【d】 ---
    b"d/" => 'đ',
    // --- 【e】 ---
    b"e`" => 'è', b"e'" => 'é', b"e^" => 'ê', b"e:" => 'ë',
    b"e_" => 'ē', b"e~" => 'ẽ',
    // --- 【g】 ---
    b"g^" => 'ĝ',
    // --- 【h】 ---
    b"h^" => 'ĥ', b"h/" => 'ħ',
    // --- 【i】 ---
    b"i`" => 'ì', b"i'" => 'í', b"i^" => 'î', b"i:" => 'ï',
    b"i_" => 'ī', b"i/" => 'ɨ', b"i~" => 'ĩ',
    // --- 【j】 ---
    b"j^" => 'ĵ',
    // --- 【l】 ---
    b"l/" => 'ł', b"l'" => 'ĺ',
    // --- 【m】 ---
    b"m'" => 'ḿ',
    // --- 【n】 ---
    b"n`" => 'ǹ', b"n~" => 'ñ', b"n'" => 'ń',
    // --- 【o】 ---
    b"o`" => 'ò', b"o'" => 'ó', b"o^" => 'ô', b"o~" => 'õ',
    b"o:" => 'ö', b"o/" => 'ø', b"o_" => 'ō',
    // --- 【r】 ---
    b"r'" => 'ŕ',
    // --- 【s】 ---
    b"s'" => 'ś', b"s," => 'ş', b"s^" => 'ŝ',
    // --- 【t】 ---
    b"t," => 'ţ',
    // --- 【u】 ---
    b"u`" => 'ù', b"u'" => 'ú', b"u^" => 'û', b"u:" => 'ü',
    b"u_" => 'ū', b"u&" => 'ů', b"u~" => 'ũ',
    // --- 【y】 ---
    b"y'" => 'ý', b"y:" => 'ÿ',
    // --- 【z】 ---
    b"z'" => 'ź',
    // --- 【A】 ---
    b"A`" => 'À', b"A'" => 'Á', b"A^" => 'Â', b"A~" => 'Ã',
    b"A:" => 'Ä', b"A&" => 'Å', b"A_" => 'Ā',
    // --- 【C】 ---
    b"C," => 'Ç', b"C'" => 'Ć', b"C^" => 'Ĉ',
    // --- 【D】 ---
    b"D/" => 'Đ',
    // --- 【E】 ---
    b"E`" => 'È', b"E'" => 'É', b"E^" => 'Ê', b"E:" => 'Ë',
    b"E_" => 'Ē', b"E~" => 'Ẽ',
    // --- 【G】 ---
    b"G^" => 'Ĝ',
    // --- 【H】 ---
    b"H^" => 'Ĥ',
    // --- 【I】 ---
    b"I`" => 'Ì', b"I'" => 'Í', b"I^" => 'Î', b"I:" => 'Ï',
    b"I_" => 'Ī', b"I~" => 'Ĩ',
    // --- 【J】 ---
    b"J^" => 'Ĵ',
    // --- 【L】 ---
    b"L/" => 'Ł', b"L'" => 'Ĺ',
    // --- 【M】 ---
    b"M'" => 'Ḿ',
    // --- 【N】 ---
    b"N`" => 'Ǹ', b"N~" => 'Ñ', b"N'" => 'Ń',
    // --- 【O】 ---
    b"O`" => 'Ò', b"O'" => 'Ó', b"O^" => 'Ô', b"O~" => 'Õ',
    b"O:" => 'Ö', b"O/" => 'Ø', b"O_" => 'Ō',
    // --- 【R】 ---
    b"R'" => 'Ŕ',
    // --- 【S】 ---
    b"S'" => 'Ś', b"S," => 'Ş', b"S^" => 'Ŝ',
    // --- 【T】 ---
    b"T," => 'Ţ',
    // --- 【U】 ---
    b"U`" => 'Ù', b"U'" => 'Ú', b"U^" => 'Û', b"U:" => 'Ü',
    b"U_" => 'Ū', b"U&" => 'Ů', b"U~" => 'Ũ',
    // --- 【Y】 ---
    b"Y'" => 'Ý',
    // --- 【Z】 ---
    b"Z'" => 'Ź',
};

const _: () = {
    // Pin runtime tables to canonical table size: 4 ligatures (in
    // `match_ligature`) + 110 digraphs = 114 spec entries. Compile-time
    // assert so a forgotten entry surfaces during build, not at the
    // first runtime test.
    assert!(
        ACCENT_DIGRAPHS.len() == 110,
        "ACCENT_DIGRAPHS must contain exactly 110 entries (114 spec − 4 ligatures)"
    );
};

/// Decompose Aozora accent digraphs anywhere inside `fragment`.
///
/// Call this on the **body of a `〔...〕` span** only; the transform is
/// restricted to that convention so English text (`isn't`, `text,`, `word's`)
/// doesn't false-match legitimate spec entries (`n'`=ń, `t,`=ţ, and friends).
///
/// Guarantees:
/// - Returns `Cow::Borrowed(fragment)` when no accent **marker byte** appears
///   (zero alloc on the common Japanese-only case).
/// - Greedy longest-match: ligatures (3-byte, e.g. `ae&` = æ) beat the 2-byte
///   digraphs that share a prefix (`a&` = å would otherwise apply).
/// - Byte length of the output can be up to 3 bytes per 2-byte digraph for the
///   few entries that land in U+1Exx (`m'` = ḿ, `e~` = ẽ). Most entries shrink
///   (3-byte ligature → 2-byte UTF-8). The invariant we do hold: the result
///   is always a valid UTF-8 string.
///
/// The implementation is linear in `fragment.len()`: we walk the byte stream
/// left-to-right, peek `<= 3` bytes at a time, and commit the longest match
/// that's in the table.
#[must_use]
pub fn decompose_fragment(fragment: &str) -> Cow<'_, str> {
    let bytes = fragment.as_bytes();
    // Early-out: if no accent marker byte appears at all, the output equals the
    // input bit-for-bit. Borrow to avoid allocation.
    //
    // The membership test goes through the [`ACCENT_MARKER_MASK`] u128
    // bitmap, which lowers to one cmp + shift + AND per byte — the
    // tightest path possible without SIMD. SIMD prefilter wouldn't help
    // here: aozora text is overwhelmingly Japanese (3-byte UTF-8 with
    // 0xE3 lead byte), so byte-level memchr-style searches don't reduce
    // the candidate set.
    if !bytes.iter().any(|b| is_accent_marker(*b)) {
        return Cow::Borrowed(fragment);
    }

    let mut out = String::with_capacity(fragment.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some((pat_len, ch)) = try_match(bytes, i) {
            out.push(ch);
            i += pat_len;
        } else {
            // Advance one UTF-8 scalar value. Every index we land on is a
            // valid char boundary because we only stride by `pat_len` (2 or 3
            // ASCII bytes) or by `ch.len_utf8()`. `.get(i..)` both avoids
            // `clippy::string_slice` and defends against the stride
            // invariant breaking: a misaligned index yields `None`, which
            // breaks the loop cleanly.
            let Some(ch) = fragment.get(i..).and_then(|s| s.chars().next()) else {
                break;
            };
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Cow::Owned(out)
}

/// Attempt to match a table entry starting at `bytes[i]`. Longest-first
/// (the spec rule): try 3-byte ligatures before 2-byte digraphs.
///
/// - **3-byte path**: a 4-arm `match` against the four ligatures
///   (`ae&`, `AE&`, `oe&`, `OE&`). `match_ligature` lowers to a tight
///   jump-table-or-direct-compares form.
/// - **2-byte path**: O(1) lookup in `ACCENT_DIGRAPHS`, a `phf::Map`
///   built at compile time over all 110 spec digraph entries.
///
/// Returns `(consumed_bytes, replacement_char)` on match.
#[inline]
fn try_match(bytes: &[u8], i: usize) -> Option<(usize, char)> {
    if i + 3 <= bytes.len()
        && let Some(ch) = match_ligature(&bytes[i..i + 3])
    {
        return Some((3, ch));
    }
    if i + 2 <= bytes.len()
        && let Some(&ch) = ACCENT_DIGRAPHS.get(&bytes[i..i + 2])
    {
        return Some((2, ch));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size_is_pinned_to_spec_count() {
        // Verified 2026-04-23 against <https://www.aozora.gr.jp/accent_separation.html>
        // (archived at docs/specs/aozora/accent_separation.html) by enumerating
        // every ASCII digraph and ligature in the 【a..z】, 【A..Z】, and 【合字】
        // groups. A drop below this number means a merge lost table entries;
        // a rise means the spec added entries and the table needs to grow.
        const EXPECTED: usize = 114;
        assert_eq!(
            ACCENT_TABLE.len(),
            EXPECTED,
            "spec count drift — see docs/specs/aozora/accent_separation.html"
        );
    }

    #[test]
    fn every_table_entry_is_representable_ascii_source() {
        for (pat, _) in ACCENT_TABLE {
            assert!(
                pat.is_ascii(),
                "digraph {pat:?} must be pure ASCII per spec"
            );
            assert!(
                pat.len() == 2 || pat.len() == 3,
                "digraph {pat:?} must be 2 or 3 bytes"
            );
        }
    }

    #[test]
    fn every_table_entry_has_unique_pattern() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for (pat, _) in ACCENT_TABLE {
            assert!(seen.insert(pat), "duplicate digraph {pat:?}");
        }
    }

    #[test]
    fn digraph_size_growth_stays_within_one_extra_byte() {
        // We don't claim byte-length non-growth (disproved by entries like
        // `m'` = ḿ U+1E3F which grows 2 → 3 bytes), but we DO pin that no entry
        // grows by more than one byte: callers budgeting diagnostic span
        // back-mapping need to allocate at most `input_len + count_of_digraphs`
        // output bytes.
        for (pat, ch) in ACCENT_TABLE {
            let out_len = ch.len_utf8();
            let in_len = pat.len();
            let growth = out_len.saturating_sub(in_len);
            assert!(
                growth <= 1,
                "digraph {pat:?} → {ch} grew by {growth} bytes (cap is 1)"
            );
        }
    }

    // --- Specific spec checkpoints (sample across groups to catch table drift) ---

    #[test]
    fn spec_point_e_grave() {
        assert_eq!(decompose_fragment("fune`bre"), "funèbre");
    }

    #[test]
    fn spec_point_acute_accents() {
        assert_eq!(decompose_fragment("ve'rite'"), "vérité");
    }

    #[test]
    fn spec_point_circumflex_and_cedilla_together() {
        assert_eq!(decompose_fragment("C,a va^"), "Ça vâ");
    }

    #[test]
    fn spec_point_all_vowel_graves() {
        assert_eq!(decompose_fragment("a` e` i` o` u`"), "à è ì ò ù");
    }

    #[test]
    fn spec_point_uppercase_accents() {
        assert_eq!(decompose_fragment("A` E' N~"), "À É Ñ");
    }

    #[test]
    fn spec_point_ligatures_beat_ring_above() {
        // `s&` = ß (eszett), NOT `s` + ring-above — longest-match ordering.
        assert_eq!(decompose_fragment("stras&e"), "straße");
        // Ligature over single-letter: ae& = æ, not a& + e.
        assert_eq!(decompose_fragment("ae&on"), "æon");
        assert_eq!(decompose_fragment("OE&uvre"), "Œuvre");
    }

    #[test]
    fn spec_point_stroke_and_macron() {
        assert_eq!(decompose_fragment("d/o_g"), "đōg");
    }

    #[test]
    fn input_without_any_marker_byte_is_borrowed() {
        // Must avoid every ASCII marker: ' ` ^ : ~ & , / _
        let input = "plain Japanese prose ここはテストです 春夏秋冬";
        let out = decompose_fragment(input);
        assert!(
            matches!(out, Cow::Borrowed(_)),
            "expected zero-alloc path for {input:?}"
        );
        assert_eq!(out, input);
    }

    #[test]
    fn isolated_markers_not_preceded_by_table_base_are_preserved() {
        // A marker that lands without a valid base letter preceding it stays
        // intact. The call site is the inside of a 〔〕 span, where
        // these cases represent author typos or genuine punctuation.
        assert_eq!(decompose_fragment("'tis"), "'tis"); // leading apostrophe
        assert_eq!(decompose_fragment("5^2"), "5^2"); // digit base not in spec
        assert_eq!(decompose_fragment("q^"), "q^"); // q not in spec table
    }

    #[test]
    fn markers_are_greedy_for_any_valid_preceding_base() {
        // Even when the user might have intended punctuation, the spec rule is
        // simple: `<base-letter><marker>` decomposes. Call sites must gate by
        // the 〔〕 wrapper to avoid false-positives on English text.
        assert_eq!(decompose_fragment("`hello`"), "`hellò"); // o` → ò
        assert_eq!(decompose_fragment("text,"), "texţ"); // t, → ţ
    }

    #[test]
    fn unknown_base_letters_stay_unchanged() {
        // f doesn't have entries in the spec; f' must stay.
        assert_eq!(decompose_fragment("f'x"), "f'x");
        // q also absent.
        assert_eq!(decompose_fragment("q^"), "q^");
    }

    #[test]
    fn mixed_japanese_and_accents_round_trip_on_japanese() {
        assert_eq!(
            decompose_fragment("ここは fune`bre です"),
            "ここは funèbre です"
        );
    }

    #[test]
    fn empty_input_is_borrowed() {
        let out = decompose_fragment("");
        assert!(matches!(out, Cow::Borrowed("")));
    }

    #[test]
    fn three_byte_ligatures_shrink_output_byte_length() {
        // 3-byte ASCII ligature → 2-byte UTF-8: strictly shorter.
        // `s&` = ß is NOT a 3-byte ligature; it's a 2-byte digraph → 2 UTF-8
        // bytes, so length is preserved. Covered separately below.
        for (input, expected) in [("ae&on", "æon"), ("OE&uvre", "Œuvre")] {
            let out = decompose_fragment(input);
            assert!(
                out.len() < input.len(),
                "3-byte ligature should shrink: {input:?} → {out:?}"
            );
            assert_eq!(out, expected);
        }
    }

    #[test]
    fn two_byte_eszett_preserves_output_byte_length() {
        // `s&` = ß is a 2-byte source → 2-byte UTF-8 output: neutral length.
        let out = decompose_fragment("stras&e");
        assert_eq!(out, "straße");
        assert_eq!(out.len(), "stras&e".len());
    }

    #[test]
    fn bmp_above_u1e00_digraphs_may_grow_output() {
        // `m'` → ḿ U+1E3F is 3 bytes; documented growth path.
        let out = decompose_fragment("m'a");
        assert_eq!(out, "ḿa");
        assert!(out.len() > "m'a".len());
    }

    #[test]
    fn property_all_table_entries_round_trip() {
        // Every table entry, when wrapped in benign context, decomposes to its
        // target char and only that char.
        for (pat, ch) in ACCENT_TABLE {
            let input = format!("_{pat}_");
            let out = decompose_fragment(&input);
            let expected: String = format!("_{ch}_");
            assert_eq!(*out, *expected, "pattern {pat:?} failed");
        }
    }
}
