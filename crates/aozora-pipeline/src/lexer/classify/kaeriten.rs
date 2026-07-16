//! Kaeriten (返り点) classification helpers.
//!
//! Ladder-family classification and the kana-prose heuristic used by
//! the classify-stage stream's end-of-document kaeriten pairing checks
//! (`ClassifyStream::finalize_kaeriten`).

use aozora_syntax::Span;

/// Family of a bracketed kaeriten ladder. Reading-order return marks
/// come in ordered families; a mark of rank `r` needs a same-family base
/// (`一` / `上` / `甲`) somewhere in the document (see
/// `Diagnostic::bracketed_kaeriten_no_pair`). `レ` (re-ten) and 送り仮名
/// `（X）` are standalone and never ladder.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum KaeritenFamily {
    /// `一` < `二` < `三` < `四` (and the `Xレ` compounds).
    Numeric,
    /// `上` < `中` < `下`.
    Jouge,
    /// `甲` < `乙` < `丙` < `丁`.
    Kouotsu,
    /// `レ`, 送り仮名, or any non-ladder mark.
    Other,
}

/// One bracketed kaeriten observed during classification, retained for
/// the end-of-document pairing / context checks run in
/// `ClassifyStream::finalize_kaeriten`.
#[derive(Clone, Copy)]
pub(super) struct KaeritenObs {
    pub(super) family: KaeritenFamily,
    /// 1-based ladder rank; `0` for non-ladder marks.
    pub(super) rank: u8,
    /// Whether this mark participates in ladder-base checks.
    pub(super) is_ladder: bool,
    /// Byte-range of the `［＃…］` directive in the sanitized source.
    pub(super) span: Span,
}

/// Classify a bracketed kaeriten body into `(family, rank, is_ladder)`.
/// `Xレ` compounds ladder by their base char `X`; `レ` alone and 送り仮名
/// `（X）` are non-ladder.
pub(super) fn classify_kaeriten_mark(mark: &str) -> (KaeritenFamily, u8, bool) {
    // 送り仮名 ［＃（X）］ — a kaeriten node but not a ladder mark.
    if mark.starts_with('（') {
        return (KaeritenFamily::Other, 0, false);
    }
    // Compound `Xレ` ladders by its base char; `レ` alone has empty base.
    let base = mark
        .strip_suffix('レ')
        .filter(|b| !b.is_empty())
        .unwrap_or(mark);
    match base {
        "一" => (KaeritenFamily::Numeric, 1, true),
        "二" => (KaeritenFamily::Numeric, 2, true),
        "三" => (KaeritenFamily::Numeric, 3, true),
        "四" => (KaeritenFamily::Numeric, 4, true),
        "上" => (KaeritenFamily::Jouge, 1, true),
        "中" => (KaeritenFamily::Jouge, 2, true),
        "下" => (KaeritenFamily::Jouge, 3, true),
        "甲" => (KaeritenFamily::Kouotsu, 1, true),
        "乙" => (KaeritenFamily::Kouotsu, 2, true),
        "丙" => (KaeritenFamily::Kouotsu, 3, true),
        "丁" => (KaeritenFamily::Kouotsu, 4, true),
        _ => (KaeritenFamily::Other, 0, false),
    }
}

/// Dense index of a ladder [`KaeritenFamily`] for the base-presence
/// table. `Other` never ladders so it shares the `Numeric` slot (read
/// but never marked present for non-ladder marks).
pub(super) fn family_index(fam: KaeritenFamily) -> usize {
    match fam {
        KaeritenFamily::Jouge => 1,
        KaeritenFamily::Kouotsu => 2,
        KaeritenFamily::Numeric | KaeritenFamily::Other => 0,
    }
}

/// Conservative "is this kana prose, not 漢文?" check for the lone-mark
/// outside-kanbun heuristic. Reads a small character window each side of
/// `span`: kana-dominant with almost no kanji ⇒ prose. Real 漢文 around a
/// genuine 返り点 is kanji-dense and never trips this.
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

/// Hiragana / katakana / half-width katakana.
fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9D}')
}

/// CJK unified ideographs (incl. Ext-A and compatibility) — "kanji".
fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_marks_pin_family_rank_and_ladder() {
        // Every ladder base char maps to its exact (family, rank, is_ladder);
        // 送り仮名 `（X）` and the lone re-ten `レ` are non-ladder `Other`. The
        // `Xレ` compound ladders by its base char `X`, not by the `レ`.
        // KaeritenFamily has no Debug, so compare by `==` under assert!.
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
            // Compound: strips the `レ` suffix and ladders on the non-empty base.
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
        // Numeric/Other share slot 0; Jouge is 1; Kouotsu is 2.
        assert_eq!(family_index(KaeritenFamily::Numeric), 0);
        assert_eq!(family_index(KaeritenFamily::Other), 0);
        assert_eq!(family_index(KaeritenFamily::Jouge), 1);
        assert_eq!(family_index(KaeritenFamily::Kouotsu), 2);
    }

    #[test]
    fn kana_prose_needs_kana_dominance_and_low_kanji() {
        // Kana-dominant window with no kanji ⇒ prose. "あいうえお": each char is
        // 3 bytes, so span(6,9) brackets "う"; both sides are pure kana.
        let src = "あいうえお";
        assert!(looks_like_kana_prose(src, Span::new(6, 9)));

        // No kana and no kanji ⇒ NOT prose (kana >= 2 is false). Distinguishes
        // `&&` from `||`: `||` would treat kanji <= 1 alone as sufficient.
        let ascii = "ABCDEFGHIJ";
        assert!(!looks_like_kana_prose(ascii, Span::new(4, 6)));

        // Kana present but two kanji in the window ⇒ NOT prose (kanji <= 1 is
        // false). "あい漢字" is 4 chars × 3 bytes; span(12,12) puts the whole
        // string in the `before` window: kana=2, kanji=2. If kanji were never
        // counted (is_kanji stub / `*= 1` on the counter) this would wrongly
        // report prose.
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
