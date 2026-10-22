//! Supports return-mark pairing checks during classification.

use crate::syntax::Span;

const THIRD_LADDER_RANK: u8 = 3;
const FOURTH_LADDER_RANK: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum KaeritenFamily {
    Numeric,
    Jouge,
    Kouotsu,
    Other,
}

#[derive(Clone, Copy)]
pub(super) struct KaeritenObs {
    pub(super) family: KaeritenFamily,
    /// One-based ladder rank, or zero for a standalone mark.
    pub(super) rank: u8,
    pub(super) is_ladder: bool,
    /// Directive span in sanitized source.
    pub(super) span: Span,
}

pub(super) fn classify_kaeriten_mark(mark: &str) -> (KaeritenFamily, u8, bool) {
    if mark.starts_with('（') {
        return (KaeritenFamily::Other, 0, false);
    }
    let base = mark
        .strip_suffix('レ')
        .filter(|b| !b.is_empty())
        .unwrap_or(mark);
    match base {
        "一" => (KaeritenFamily::Numeric, 1, true),
        "二" => (KaeritenFamily::Numeric, 2, true),
        "三" => (KaeritenFamily::Numeric, THIRD_LADDER_RANK, true),
        "四" => (KaeritenFamily::Numeric, FOURTH_LADDER_RANK, true),
        "上" => (KaeritenFamily::Jouge, 1, true),
        "中" => (KaeritenFamily::Jouge, 2, true),
        "下" => (KaeritenFamily::Jouge, THIRD_LADDER_RANK, true),
        "甲" => (KaeritenFamily::Kouotsu, 1, true),
        "乙" => (KaeritenFamily::Kouotsu, 2, true),
        "丙" => (KaeritenFamily::Kouotsu, THIRD_LADDER_RANK, true),
        "丁" => (KaeritenFamily::Kouotsu, FOURTH_LADDER_RANK, true),
        _ => (KaeritenFamily::Other, 0, false),
    }
}

pub(super) fn family_index(fam: KaeritenFamily) -> usize {
    match fam {
        KaeritenFamily::Jouge => 1,
        KaeritenFamily::Kouotsu => 2,
        KaeritenFamily::Numeric | KaeritenFamily::Other => 0,
    }
}

pub(super) fn looks_like_kana_prose(source: &str, span: Span) -> bool {
    const WIN: usize = 12;
    let before = source.get(..span.start as usize).unwrap_or("");
    let after = source.get(span.end as usize..).unwrap_or("");
    let (mut kana, mut kanji) = (0u32, 0u32);
    for c in before
        .chars()
        .rev()
        .take(WIN)
        .chain(after.chars().take(WIN))
    {
        if is_kana(c) {
            kana += 1;
        } else if is_kanji(c) {
            kanji += 1;
        }
    }
    kana >= 2 && kanji <= 1
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9D}')
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_marks_pin_family_rank_and_ladder() {
        let cases: &[(&str, KaeritenFamily, u8, bool)] = &[
            ("（キ）", KaeritenFamily::Other, 0, false),
            ("レ", KaeritenFamily::Other, 0, false),
            ("一", KaeritenFamily::Numeric, 1, true),
            ("二", KaeritenFamily::Numeric, 2, true),
            ("三", KaeritenFamily::Numeric, 3, true),
            ("四", KaeritenFamily::Numeric, 4, true),
            ("上", KaeritenFamily::Jouge, 1, true),
            ("中", KaeritenFamily::Jouge, 2, true),
            ("下", KaeritenFamily::Jouge, 3, true),
            ("甲", KaeritenFamily::Kouotsu, 1, true),
            ("乙", KaeritenFamily::Kouotsu, 2, true),
            ("丙", KaeritenFamily::Kouotsu, 3, true),
            ("丁", KaeritenFamily::Kouotsu, 4, true),
            ("一レ", KaeritenFamily::Numeric, 1, true),
            ("上レ", KaeritenFamily::Jouge, 1, true),
        ];
        for &(mark, fam, rank, ladder) in cases {
            assert!(
                classify_kaeriten_mark(mark) == (fam, rank, ladder),
                "classify_kaeriten_mark({mark:?}) returned the wrong (family, rank, ladder)"
            );
        }
    }

    #[test]
    fn family_index_is_dense_per_ladder_family() {
        assert_eq!(family_index(KaeritenFamily::Numeric), 0);
        assert_eq!(family_index(KaeritenFamily::Other), 0);
        assert_eq!(family_index(KaeritenFamily::Jouge), 1);
        assert_eq!(family_index(KaeritenFamily::Kouotsu), 2);
    }

    #[test]
    fn kana_prose_needs_kana_dominance_and_low_kanji() {
        let src = "あいうえお";
        assert!(looks_like_kana_prose(src, Span::new(6, 9)));

        let ascii = "ABCDEFGHIJ";
        assert!(!looks_like_kana_prose(ascii, Span::new(4, 6)));

        let mixed = "あい漢字";
        assert!(!looks_like_kana_prose(mixed, Span::new(12, 12)));
    }

    #[test]
    fn kanji_and_kana_predicates_discriminate_scripts() {
        assert!(is_kanji('漢'));
        assert!(!is_kanji('あ'));
        assert!(is_kana('あ'));
        assert!(is_kana('ア'));
        assert!(!is_kana('漢'));
    }
}
