#![expect(
    clippy::expect_used,
    reason = "matched accent grammar establishes character boundaries"
)]

//! Resolves accent notation through [`decompose_fragment`] and letter composition helpers.

use std::borrow::Cow;

const ASCII_BYTE_LIMIT: u8 = 128;
const LIGATURE_BYTE_LEN: usize = 3;
const DECIMAL_RADIX: usize = 10;

use crate::syntax::format::AccentMark;

#[cfg(test)]
pub(crate) const ACCENT_TABLE: &[(&str, char)] = &[
    ("ae&", 'æ'),
    ("AE&", 'Æ'),
    ("oe&", 'œ'),
    ("OE&", 'Œ'),
    ("s&", 'ß'),
    ("a`", 'à'),
    ("a'", 'á'),
    ("a^", 'â'),
    ("a~", 'ã'),
    ("a:", 'ä'),
    ("a&", 'å'),
    ("a_", 'ā'),
    ("c,", 'ç'),
    ("c'", 'ć'),
    ("c^", 'ĉ'),
    ("d/", 'đ'),
    ("e`", 'è'),
    ("e'", 'é'),
    ("e^", 'ê'),
    ("e:", 'ë'),
    ("e_", 'ē'),
    ("e~", 'ẽ'),
    ("g^", 'ĝ'),
    ("h^", 'ĥ'),
    ("h/", 'ħ'),
    ("i`", 'ì'),
    ("i'", 'í'),
    ("i^", 'î'),
    ("i:", 'ï'),
    ("i_", 'ī'),
    ("i/", 'ɨ'),
    ("i~", 'ĩ'),
    ("j^", 'ĵ'),
    ("l/", 'ł'),
    ("l'", 'ĺ'),
    ("m'", 'ḿ'),
    ("n`", 'ǹ'),
    ("n~", 'ñ'),
    ("n'", 'ń'),
    ("o`", 'ò'),
    ("o'", 'ó'),
    ("o^", 'ô'),
    ("o~", 'õ'),
    ("o:", 'ö'),
    ("o/", 'ø'),
    ("o_", 'ō'),
    ("r'", 'ŕ'),
    ("s'", 'ś'),
    ("s,", 'ş'),
    ("s^", 'ŝ'),
    ("t,", 'ţ'),
    ("u`", 'ù'),
    ("u'", 'ú'),
    ("u^", 'û'),
    ("u:", 'ü'),
    ("u_", 'ū'),
    ("u&", 'ů'),
    ("u~", 'ũ'),
    ("y'", 'ý'),
    ("y:", 'ÿ'),
    ("z'", 'ź'),
    ("A`", 'À'),
    ("A'", 'Á'),
    ("A^", 'Â'),
    ("A~", 'Ã'),
    ("A:", 'Ä'),
    ("A&", 'Å'),
    ("A_", 'Ā'),
    ("C,", 'Ç'),
    ("C'", 'Ć'),
    ("C^", 'Ĉ'),
    ("D/", 'Đ'),
    ("E`", 'È'),
    ("E'", 'É'),
    ("E^", 'Ê'),
    ("E:", 'Ë'),
    ("E_", 'Ē'),
    ("E~", 'Ẽ'),
    ("G^", 'Ĝ'),
    ("H^", 'Ĥ'),
    ("I`", 'Ì'),
    ("I'", 'Í'),
    ("I^", 'Î'),
    ("I:", 'Ï'),
    ("I_", 'Ī'),
    ("I~", 'Ĩ'),
    ("J^", 'Ĵ'),
    ("L/", 'Ł'),
    ("L'", 'Ĺ'),
    ("M'", 'Ḿ'),
    ("N`", 'Ǹ'),
    ("N~", 'Ñ'),
    ("N'", 'Ń'),
    ("O`", 'Ò'),
    ("O'", 'Ó'),
    ("O^", 'Ô'),
    ("O~", 'Õ'),
    ("O:", 'Ö'),
    ("O/", 'Ø'),
    ("O_", 'Ō'),
    ("R'", 'Ŕ'),
    ("S'", 'Ś'),
    ("S,", 'Ş'),
    ("S^", 'Ŝ'),
    ("T,", 'Ţ'),
    ("U`", 'Ù'),
    ("U'", 'Ú'),
    ("U^", 'Û'),
    ("U:", 'Ü'),
    ("U_", 'Ū'),
    ("U&", 'Ů'),
    ("U~", 'Ũ'),
    ("Y'", 'Ý'),
    ("Z'", 'Ź'),
];

pub(crate) const ACCENT_MARKERS: &[u8] = b"'`^:~&,/_";

const ACCENT_MARKER_MASK: u128 = {
    let mut m: u128 = 0;
    let bs = ACCENT_MARKERS;
    let mut i = 0;
    while i < bs.len() {
        m |= 1u128 << bs[i];
        i += 1;
    }
    m
};

/// `mutants::skip`: the fixed marker data satisfies these assertions without observable runtime behavior.
#[cfg_attr(test, mutants::skip)]
const fn assert_markers_ascii(bs: &[u8]) {
    let mut i = 0;
    while i < bs.len() {
        assert!(
            bs[i] < ASCII_BYTE_LIMIT,
            "ACCENT_MARKERS must stay ASCII-only"
        );
        i += 1;
    }
}
const _: () = assert_markers_ascii(ACCENT_MARKERS);

#[inline]
#[must_use]
pub(crate) const fn is_accent_marker(b: u8) -> bool {
    (b < ASCII_BYTE_LIMIT) && ((ACCENT_MARKER_MASK >> b) & 1) != 0
}

#[inline]
fn match_ligature(head: &[u8]) -> Option<char> {
    debug_assert_eq!(
        head.len(),
        LIGATURE_BYTE_LEN,
        "match_ligature requires exactly 3 bytes"
    );
    match head {
        b"ae&" => Some('æ'),
        b"AE&" => Some('Æ'),
        b"oe&" => Some('œ'),
        b"OE&" => Some('Œ'),
        _ => None,
    }
}

static ACCENT_DIGRAPHS: phf::Map<&'static [u8], char> = phf::phf_map! {
    b"s&" => 'ß',
    b"a`" => 'à', b"a'" => 'á', b"a^" => 'â', b"a~" => 'ã',
    b"a:" => 'ä', b"a&" => 'å', b"a_" => 'ā',
    b"c," => 'ç', b"c'" => 'ć', b"c^" => 'ĉ',
    b"d/" => 'đ',
    b"e`" => 'è', b"e'" => 'é', b"e^" => 'ê', b"e:" => 'ë',
    b"e_" => 'ē', b"e~" => 'ẽ',
    b"g^" => 'ĝ',
    b"h^" => 'ĥ', b"h/" => 'ħ',
    b"i`" => 'ì', b"i'" => 'í', b"i^" => 'î', b"i:" => 'ï',
    b"i_" => 'ī', b"i/" => 'ɨ', b"i~" => 'ĩ',
    b"j^" => 'ĵ',
    b"l/" => 'ł', b"l'" => 'ĺ',
    b"m'" => 'ḿ',
    b"n`" => 'ǹ', b"n~" => 'ñ', b"n'" => 'ń',
    b"o`" => 'ò', b"o'" => 'ó', b"o^" => 'ô', b"o~" => 'õ',
    b"o:" => 'ö', b"o/" => 'ø', b"o_" => 'ō',
    b"r'" => 'ŕ',
    b"s'" => 'ś', b"s," => 'ş', b"s^" => 'ŝ',
    b"t," => 'ţ',
    b"u`" => 'ù', b"u'" => 'ú', b"u^" => 'û', b"u:" => 'ü',
    b"u_" => 'ū', b"u&" => 'ů', b"u~" => 'ũ',
    b"y'" => 'ý', b"y:" => 'ÿ',
    b"z'" => 'ź',
    b"A`" => 'À', b"A'" => 'Á', b"A^" => 'Â', b"A~" => 'Ã',
    b"A:" => 'Ä', b"A&" => 'Å', b"A_" => 'Ā',
    b"C," => 'Ç', b"C'" => 'Ć', b"C^" => 'Ĉ',
    b"D/" => 'Đ',
    b"E`" => 'È', b"E'" => 'É', b"E^" => 'Ê', b"E:" => 'Ë',
    b"E_" => 'Ē', b"E~" => 'Ẽ',
    b"G^" => 'Ĝ',
    b"H^" => 'Ĥ',
    b"I`" => 'Ì', b"I'" => 'Í', b"I^" => 'Î', b"I:" => 'Ï',
    b"I_" => 'Ī', b"I~" => 'Ĩ',
    b"J^" => 'Ĵ',
    b"L/" => 'Ł', b"L'" => 'Ĺ',
    b"M'" => 'Ḿ',
    b"N`" => 'Ǹ', b"N~" => 'Ñ', b"N'" => 'Ń',
    b"O`" => 'Ò', b"O'" => 'Ó', b"O^" => 'Ô', b"O~" => 'Õ',
    b"O:" => 'Ö', b"O/" => 'Ø', b"O_" => 'Ō',
    b"R'" => 'Ŕ',
    b"S'" => 'Ś', b"S," => 'Ş', b"S^" => 'Ŝ',
    b"T," => 'Ţ',
    b"U`" => 'Ù', b"U'" => 'Ú', b"U^" => 'Û', b"U:" => 'Ü',
    b"U_" => 'Ū', b"U&" => 'Ů', b"U~" => 'Ũ',
    b"Y'" => 'Ý',
    b"Z'" => 'Ź',
};

const _: () = {
    assert!(
        ACCENT_DIGRAPHS.len() == 110,
        "ACCENT_DIGRAPHS must contain exactly 110 entries (114 spec − 4 ligatures)"
    );
};

/// Decomposes a `〔…〕` body and may change its byte length.
#[must_use]
pub(crate) fn decompose_fragment(fragment: &str) -> Cow<'_, str> {
    let bytes = fragment.as_bytes();
    if !bytes.iter().any(|b| is_accent_marker(*b)) {
        return Cow::Borrowed(fragment);
    }

    let mut out = String::with_capacity(fragment.len());
    let mut i = 0;
    while let Some(rest) = fragment.get(i..).filter(|rest| !rest.is_empty()) {
        let previous = i;
        if let Some((pat_len, ch)) = try_match(bytes, i) {
            out.push(ch);
            i = i
                .checked_add(pat_len)
                .expect("matched accent stays inside fragment");
        } else {
            let ch = rest.chars().next().expect("rest is non-empty");
            out.push(ch);
            i = i
                .checked_add(ch.len_utf8())
                .expect("source scalar stays inside fragment");
        }
        assert!(i > previous, "accent decomposition must advance");
    }
    Cow::Owned(out)
}

#[must_use]
#[cfg(test)]
pub(crate) fn decompose_fragment_edits(fragment: &str) -> Vec<(usize, usize, usize)> {
    let bytes = fragment.as_bytes();
    let mut edits = Vec::new();
    if !bytes.iter().any(|b| is_accent_marker(*b)) {
        return edits;
    }
    let mut i = 0;
    while let Some(rest) = fragment.get(i..).filter(|rest| !rest.is_empty()) {
        let previous = i;
        if let Some((pat_len, ch)) = try_match(bytes, i) {
            let out_len = ch.len_utf8();
            if pat_len != out_len {
                edits.push((i, pat_len, out_len));
            }
            i = i
                .checked_add(pat_len)
                .expect("matched accent stays inside fragment");
        } else {
            let ch = rest.chars().next().expect("rest is non-empty");
            i = i
                .checked_add(ch.len_utf8())
                .expect("source scalar stays inside fragment");
        }
        assert!(i > previous);
    }
    edits
}

#[inline]
fn try_match(bytes: &[u8], i: usize) -> Option<(usize, char)> {
    if i + LIGATURE_BYTE_LEN <= bytes.len()
        && let Some(ch) = match_ligature(&bytes[i..i + LIGATURE_BYTE_LEN])
    {
        return Some((LIGATURE_BYTE_LEN, ch));
    }
    if i + 2 <= bytes.len()
        && let Some(&ch) = ACCENT_DIGRAPHS.get(&bytes[i..i + 2])
    {
        return Some((2, ch));
    }
    None
}

#[must_use]
pub(crate) fn compose_accent(letter: char, mark: AccentMark) -> Option<char> {
    let base = u8::try_from(letter).ok().filter(u8::is_ascii_alphabetic)?;
    let marker = match mark {
        AccentMark::Acute => b'\'',
        AccentMark::Umlaut => b':',
        AccentMark::Grave => b'`',
    };
    ACCENT_DIGRAPHS.get(&[base, marker][..]).copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DotPosition {
    Above,
    Below,
}

/// The `aozorabunko_text` corpus attests these letter and dot combinations.
pub(crate) const ACCENT_DOT_TABLE: &[(char, DotPosition, char)] = &[
    ('m', DotPosition::Above, 'ṁ'),
    ('n', DotPosition::Above, 'ṅ'),
    ('m', DotPosition::Below, 'ṃ'),
    ('n', DotPosition::Below, 'ṇ'),
    ('s', DotPosition::Below, 'ṣ'),
    ('t', DotPosition::Below, 'ṭ'),
    ('h', DotPosition::Below, 'ḥ'),
    ('r', DotPosition::Below, 'ṛ'),
    ('d', DotPosition::Below, 'ḍ'),
    ('T', DotPosition::Below, 'Ṭ'),
    ('R', DotPosition::Below, 'Ṛ'),
];

const _: () = {
    assert!(
        ACCENT_DOT_TABLE.len() == 11,
        "ACCENT_DOT_TABLE must contain exactly 11 corpus-attested entries"
    );
};

#[must_use]
pub(crate) fn compose_dotted(base: char, pos: DotPosition) -> Option<char> {
    ACCENT_DOT_TABLE
        .iter()
        .find(|&&(b, p, _)| b == base && p == pos)
        .map(|&(_, _, glyph)| glyph)
}

/// Counts letter occurrences without regard to case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occ {
    First,
    /// One-based occurrence number.
    Nth(usize),
    Last,
}

#[derive(Debug, Clone, Copy)]
struct DotOp {
    letter: char,
    pos: DotPosition,
    occ: Occ,
}

fn parse_leading_number(s: &str) -> Option<(usize, &str)> {
    let mut n: usize = 0;
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        let digit = match ch {
            '0'..='9' => ch as usize - '0' as usize,
            '０'..='９' => ch as usize - '０' as usize,
            _ => break,
        };
        n = n.checked_mul(DECIMAL_RADIX)?.checked_add(digit)?;
        end = i + ch.len_utf8();
    }
    (end != 0).then(|| (n, &s[end..]))
}

fn parse_selector(sel: &str) -> Option<(Occ, &str)> {
    if let Some(rest) = sel.strip_prefix("最後の") {
        return Some((Occ::Last, rest));
    }
    if let Some(rest) = sel.strip_prefix("前の") {
        return Some((Occ::First, rest));
    }
    if let Some(rest) = sel.strip_prefix("後の") {
        return Some((Occ::Last, rest));
    }
    if let Some((n, rest)) = parse_leading_number(sel) {
        let letters = rest.strip_prefix("つめの")?;
        return Some((Occ::Nth(n), letters));
    }
    Some((Occ::First, sel))
}

fn parse_accent_clause(clause: &str) -> Option<Vec<DotOp>> {
    let after_letters = clause.strip_suffix("ドット付き")?;
    let (selector, tail) = after_letters.split_once('は')?;
    let posword = tail
        .strip_prefix("ともに")
        .or_else(|| tail.strip_prefix("それぞれ"))
        .unwrap_or(tail);
    let pos = match posword {
        "上" => DotPosition::Above,
        "下" => DotPosition::Below,
        _ => return None,
    };
    let (occ, letters) = parse_selector(selector)?;
    if letters.is_empty() || !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let chars: Vec<char> = letters.chars().collect();
    if chars.len() > 1 && !matches!(occ, Occ::First) {
        return None;
    }
    let mut ops = Vec::with_capacity(chars.len());
    for &letter in &chars {
        compose_dotted(letter, pos)?;
        ops.push(DotOp { letter, pos, occ });
    }
    Some(ops)
}

fn parse_accent_dot_body(body: &str) -> Option<Vec<DotOp>> {
    let mut ops = Vec::new();
    for clause in body.split(['。', '、']) {
        ops.extend(parse_accent_clause(clause)?);
    }
    (!ops.is_empty()).then_some(ops)
}

#[must_use]
pub(crate) fn compose_accent_dots(run: &str, body: &str) -> Option<String> {
    let ops = parse_accent_dot_body(body)?;
    let mut subs: Vec<(usize, char, usize)> = Vec::with_capacity(ops.len());
    for op in &ops {
        let idx = resolve_occurrence(run, op.letter, op.pos, op.occ)?;
        let base = run[idx..].chars().next()?;
        let glyph = compose_dotted(base, op.pos)?;
        subs.push((idx, glyph, base.len_utf8()));
    }
    subs.sort_by_key(|&(idx, _, _)| idx);
    let mut out = String::with_capacity(run.len());
    let mut last = 0;
    for (idx, glyph, base_len) in subs {
        if idx < last {
            return None;
        }
        out.push_str(&run[last..idx]);
        out.push(glyph);
        last = idx + base_len;
    }
    out.push_str(&run[last..]);
    Some(out)
}

/// One-based occurrence number.
fn resolve_occurrence(run: &str, letter: char, pos: DotPosition, occ: Occ) -> Option<usize> {
    let target = letter.to_ascii_lowercase();
    let composable = |idx: usize| {
        run[idx..]
            .chars()
            .next()
            .and_then(|c| compose_dotted(c, pos))
            .is_some()
    };
    let mut hits = run
        .char_indices()
        .filter(|(_, c)| c.to_ascii_lowercase() == target)
        .map(|(i, _)| i);
    match occ {
        Occ::First => hits.find(|&i| composable(i)),
        Occ::Last => hits.rfind(|&i| composable(i)),
        Occ::Nth(n) => n
            .checked_sub(1)
            .and_then(|k| hits.nth(k))
            .filter(|&i| composable(i)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size_is_pinned_to_spec_count() {
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
        assert_eq!(decompose_fragment("stras&e"), "straße");
        assert_eq!(decompose_fragment("ae&on"), "æon");
        assert_eq!(decompose_fragment("OE&uvre"), "Œuvre");
    }

    #[test]
    fn spec_point_stroke_and_macron() {
        assert_eq!(decompose_fragment("d/o_g"), "đōg");
    }

    #[test]
    fn input_without_any_marker_byte_is_borrowed() {
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
        assert_eq!(decompose_fragment("'tis"), "'tis");
        assert_eq!(decompose_fragment("5^2"), "5^2");
        assert_eq!(decompose_fragment("q^"), "q^");
    }

    #[test]
    fn markers_are_greedy_for_any_valid_preceding_base() {
        assert_eq!(decompose_fragment("`hello`"), "`hellò");
        assert_eq!(decompose_fragment("text,"), "texţ");
    }

    #[test]
    fn unknown_base_letters_stay_unchanged() {
        assert_eq!(decompose_fragment("f'x"), "f'x");
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
        let out = decompose_fragment("stras&e");
        assert_eq!(out, "straße");
        assert_eq!(out.len(), "stras&e".len());
    }

    #[test]
    fn bmp_above_u1e00_digraphs_may_grow_output() {
        let out = decompose_fragment("m'a");
        assert_eq!(out, "ḿa");
        assert!(out.len() > "m'a".len());
    }

    #[test]
    fn property_all_table_entries_round_trip() {
        for (pat, ch) in ACCENT_TABLE {
            let input = format!("_{pat}_");
            let out = decompose_fragment(&input);
            let expected: String = format!("_{ch}_");
            assert_eq!(*out, *expected, "pattern {pat:?} failed");
        }
    }

    #[test]
    fn compose_accent_maps_corpus_pairs() {
        assert_eq!(compose_accent('e', AccentMark::Acute), Some('é'));
        assert_eq!(compose_accent('o', AccentMark::Umlaut), Some('ö'));
        assert_eq!(compose_accent('a', AccentMark::Umlaut), Some('ä'));
        assert_eq!(compose_accent('e', AccentMark::Grave), Some('è'));
        assert_eq!(compose_accent('E', AccentMark::Acute), Some('É'));
    }

    #[test]
    fn compose_accent_declines_uncomposable() {
        assert_eq!(compose_accent('q', AccentMark::Acute), None);
        assert_eq!(compose_accent('あ', AccentMark::Acute), None);
        assert_eq!(compose_accent('1', AccentMark::Umlaut), None);
        assert_eq!(compose_accent('g', AccentMark::Umlaut), None);
    }

    #[test]
    fn dot_table_composes_case_preserving() {
        assert_eq!(compose_dotted('m', DotPosition::Above), Some('ṁ'));
        assert_eq!(compose_dotted('s', DotPosition::Below), Some('ṣ'));
        assert_eq!(compose_dotted('T', DotPosition::Below), Some('Ṭ'));
        assert_eq!(compose_dotted('S', DotPosition::Below), None);
        assert_eq!(compose_dotted('m', DotPosition::Below), Some('ṃ'));
    }

    #[test]
    fn accent_dot_single_clause_bare() {
        assert_eq!(
            compose_accent_dots("Sam", "mは上ドット付き").as_deref(),
            Some("Saṁ")
        );
        assert_eq!(
            compose_accent_dots("Sas", "sは下ドット付き").as_deref(),
            Some("Saṣ")
        );
    }

    #[test]
    fn accent_dot_reclaims_tortoise_span_verbatim_brackets() {
        assert_eq!(
            compose_accent_dots("〔Mīhr〕", "hは下ドット付き").as_deref(),
            Some("〔Mīḥr〕")
        );
    }

    #[test]
    fn accent_dot_ordinal_counts_case_insensitively() {
        assert_eq!(
            compose_accent_dots("Sisa", "２つめのsは下ドット付き").as_deref(),
            Some("Siṣa")
        );
        assert_eq!(
            compose_accent_dots("mama", "最後のmは上ドット付き").as_deref(),
            Some("maṁa")
        );
    }

    #[test]
    fn accent_dot_cluster_with_set_adverb() {
        assert_eq!(
            compose_accent_dots("Visnu", "snはともに下ドット付き").as_deref(),
            Some("Viṣṇu")
        );
    }

    #[test]
    fn accent_dot_multi_clause_composes() {
        assert_eq!(
            compose_accent_dots("Samsa", "mは上ドット付き。２つめのsは下ドット付き").as_deref(),
            Some("Saṁṣa")
        );
    }

    #[test]
    fn accent_dot_former_latter_pair() {
        assert_eq!(
            compose_accent_dots("Konkana", "前のnは上ドット付き、後のnは下ドット付き").as_deref(),
            Some("Koṅkaṇa")
        );
    }

    #[test]
    fn accent_dot_declines_word_qualified_and_dangyou() {
        assert_eq!(compose_accent_dots("simha", "simhaのmは上ドット付き"), None);
        assert_eq!(
            compose_accent_dots("Sinha", "７段目、Sinhaのnは上ドット付き"),
            None
        );
    }

    #[test]
    fn is_accent_marker_matches_spec_bytes_and_rejects_the_rest() {
        for &b in ACCENT_MARKERS {
            assert!(is_accent_marker(b), "0x{b:02X} is a spec accent marker");
        }
        for b in [0u8, b'a', b'Z', b'0', 127, 128, 129, 200, 255] {
            assert!(!is_accent_marker(b), "0x{b:02X} is not an accent marker");
        }
    }

    #[test]
    fn decompose_fragment_edits_reports_exact_length_deltas() {
        assert_eq!(decompose_fragment_edits("ae&on"), vec![(0, 3, 2)]);
        assert_eq!(decompose_fragment_edits("m'a"), vec![(0, 2, 3)]);
        assert!(decompose_fragment_edits("stras&e").is_empty());
        assert!(decompose_fragment_edits("plain 日本語").is_empty());
        assert_eq!(
            decompose_fragment_edits("ae&m'"),
            vec![(0, 3, 2), (3, 2, 3)],
        );
    }

    #[test]
    fn accent_dot_ascii_ordinal_counts_like_fullwidth() {
        assert_eq!(
            compose_accent_dots("Sisa", "2つめのsは下ドット付き").as_deref(),
            Some("Siṣa")
        );
        assert_eq!(compose_accent_dots("sss", "10つめのsは下ドット付き"), None);
    }

    #[test]
    fn accent_dot_declines_empty_letter_clause_in_a_multi_clause_body() {
        assert_eq!(
            compose_accent_dots("Sam", "mは上ドット付き、最後のは下ドット付き"),
            None
        );
    }

    #[test]
    fn accent_dot_declines_ordinal_applied_to_a_letter_cluster() {
        assert_eq!(
            compose_accent_dots("mnmn", "２つめのmnは上ドット付き"),
            None
        );
    }

    #[test]
    fn accent_dot_declines_absent_or_uncomposable_occurrence() {
        assert_eq!(compose_accent_dots("abc", "mは上ドット付き"), None);
        assert_eq!(compose_accent_dots("Sam", "２つめのmは上ドット付き"), None);
        assert_eq!(compose_accent_dots("Sax", "sは下ドット付き"), None);
    }
}
