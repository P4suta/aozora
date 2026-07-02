//! Non-canonical directive catalogue for the notation-hygiene lint.
//!
//! [`canonical_directive`] maps a `［＃…］` directive body that the parser
//! keeps as `DirectiveKind::Unknown` — because it is spelled as a *verified
//! near-miss* of a recognized construct (送り仮名 drift, a synonym, or a
//! malformed prefix / close) — to its canonical spelling. It returns `None`
//! for a genuine editorial Unknown, so the lint that consumes it fires only
//! on the closed, parser-verified catalogue below.
//!
//! Zero false positives by construction: the catalogue is a fixed map, not a
//! fuzzy matcher, and the `lint_catalogue` self-test in the `aozora` crate
//! pins the invariant that every suggested canonical parses
//! to a non-Unknown node while every catalogue key still parses to Unknown —
//! so the map can never rot into suggesting a form the parser rejects, nor
//! target a body the parser already recognizes.
//!
//! Single authority (the [`crate::accent`] precedent): the pipeline lint
//! (`aozora-pipeline`), the `aozora fmt --fix-notation` autofix
//! (`aozora-render`), and the LSP quick-fix all resolve the canonical form
//! here, on the trimmed body string, with no `DirectiveKind` dependency.

use std::borrow::Cow;

/// Literal whole-body variant → canonical maps.
const EXACT: &[(&str, &str)] = &[
    // 字下げ close: okurigana / 文字下げ / bare-with-N drift of ここで字下げ終わり.
    ("字下げ終わり", "ここで字下げ終わり"),
    ("字下げ終り", "ここで字下げ終わり"),
    ("字下げおわり", "ここで字下げ終わり"),
    ("文字下げ終わり", "ここで字下げ終わり"),
    ("二字下げ終わり", "ここで字下げ終わり"),
    // 表組 → 表 (block open / close).
    ("ここから表組", "ここから表"),
    ("ここで表組終わり", "ここで表終わり"),
    // Marker / synonym drift.
    ("黒丸傍点", "丸傍点"),
    ("中央寄せ", "中央揃え"),
    ("改行を挿入", "改行"),
    ("斜体字", "斜体"),
    ("中中見出し", "中見出し"),
    // Line font-size compound: gothic weight normalised to the sole recognised
    // line weight (太字). Lossy (gothic→bold) but faithful by Aozora convention.
    ("中文字、ゴシック体", "中文字、太字"),
    // Region-close synonyms — okurigana drift / 文字下げ / 横書き=横組み /
    // 横組みの表=表, all resolving to the canonical `ここで…終わり` close.
    ("ここで字下げおわり", "ここで字下げ終わり"),
    ("ここで字下げ終り", "ここで字下げ終わり"),
    ("ここで文字下げ終わり", "ここで字下げ終わり"),
    ("ここで横書き終わり", "ここで横組み終わり"),
    ("ここで左から右への横組み終わり", "ここで横組み終わり"),
    ("ここで横組みの表終わり", "ここで表終わり"),
    // Region-open synonyms — 地付きで/こ地付き=地付き, 横書き=横組み.
    ("地付きで", "地付き"),
    ("こ地付き", "地付き"),
    ("ここから横書き", "ここから横組み"),
];

/// Map a non-canonical `［＃…］` directive body to its canonical spelling, or
/// `None` for a genuine editorial Unknown. `body` is the trimmed inner text,
/// without the `［＃` / `］` delimiters.
#[must_use]
pub fn canonical_directive(body: &str) -> Option<Cow<'static, str>> {
    if let Some((_, canonical)) = EXACT.iter().find(|(variant, _)| *variant == body) {
        return Some(Cow::Borrowed(canonical));
    }
    parameterized(body)
        .or_else(|| forward_form(body))
        .map(Cow::Owned)
}

/// A digit run: one or more ASCII `0-9` or full-width `０-９` characters.
fn is_digit_run(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

/// Digit-preserving rules — the `{N}` run is copied verbatim. Grouped by the
/// scope each family normalises (font-size / region-open / region-close /
/// alignment); the first group to claim the body wins.
fn parameterized(body: &str) -> Option<String> {
    parameterized_font_size(body)
        .or_else(|| parameterized_indent_open(body))
        .or_else(|| parameterized_region_close(body))
        .or_else(|| parameterized_align(body))
}

/// `{N}回り大きな/小さな文字` → `{N}段階…文字`.
fn parameterized_font_size(body: &str) -> Option<String> {
    for size in ["大きな文字", "小さな文字"] {
        if let Some(n) = body
            .strip_suffix(size)
            .and_then(|head| head.strip_suffix("回り"))
            && is_digit_run(n)
        {
            return Some(format!("{n}段階{size}"));
        }
    }
    None
}

/// Region-open indent synonyms → canonical `ここから{N}字下げ`.
fn parameterized_indent_open(body: &str) -> Option<String> {
    // ここか / ここより / 以下 {N}字下げ (malformed or 以下-prefixed open).
    for bad in ["ここか", "ここより", "以下"] {
        if let Some(n) = body
            .strip_prefix(bad)
            .and_then(|rest| rest.strip_suffix("字下げ"))
            && is_digit_run(n)
        {
            return Some(format!("ここから{n}字下げ"));
        }
    }
    // ここから最後まで{N}字下げ (drop 最後まで) / ここから{N}　字下げ (drop the
    // stray full-width space).
    if let Some(n) = body
        .strip_prefix("ここから最後まで")
        .and_then(|r| r.strip_suffix("字下げ"))
        .filter(|n| is_digit_run(n))
        .or_else(|| {
            body.strip_prefix("ここから")
                .and_then(|r| r.strip_suffix("　字下げ"))
                .filter(|n| is_digit_run(n))
        })
    {
        return Some(format!("ここから{n}字下げ"));
    }
    None
}

/// Region-close okurigana / magnitude / stray-bracket drift → `ここで…終わり`.
/// Tail-anchored and digit-run guarded, so `、`-bearing compound closes (which
/// carry a second axis) never match.
fn parameterized_region_close(body: &str) -> Option<String> {
    let inner = body.strip_prefix("ここで")?;
    // ここで{N}字下げ終わり → ここで字下げ終わり (drop the redundant N).
    if let Some(n) = inner.strip_suffix("字下げ終わり")
        && is_digit_run(n)
    {
        return Some("ここで字下げ終わり".to_owned());
    }
    // ここで{N}段組[み]終わり → ここで段組終わり.
    for tail in ["段組終わり", "段組み終わり"] {
        if let Some(n) = inner.strip_suffix(tail)
            && is_digit_run(n)
        {
            return Some("ここで段組終わり".to_owned());
        }
    }
    // ここで{N}段階(大きな|小さな)文字終わり → ここで(大きな|小さな)文字終わり.
    for size in ["大きな", "小さな"] {
        if let Some(n) = inner.strip_suffix(&format!("段階{size}文字終わり"))
            && is_digit_run(n)
        {
            return Some(format!("ここで{size}文字終わり"));
        }
    }
    // ここで…終わり」 → drop a stray trailing `」`.
    if let Some(head) = body.strip_suffix('」')
        && head.ends_with("終わり")
    {
        return Some(head.to_owned());
    }
    None
}

/// 字上げ / 字下げ numeric spellings → the recognised alignment / indent leaf.
fn parameterized_align(body: &str) -> Option<String> {
    // {N}字下げて → {N}字下げ.
    if let Some(n) = body.strip_suffix("字下げて")
        && is_digit_run(n)
    {
        return Some(format!("{n}字下げ"));
    }
    // 地付き、地より{N}字アキ / 字あき → 地から{N}字上げ (bottom-align with gap).
    for tail in ["字アキ", "字あき"] {
        if let Some(n) = body
            .strip_prefix("地付き、地より")
            .and_then(|r| r.strip_suffix(tail))
            && is_digit_run(n)
        {
            return Some(format!("地から{n}字上げ"));
        }
    }
    // 行末から{N}字上で地付き → 地から{N}字上げ.
    if let Some(n) = body
        .strip_prefix("行末から")
        .and_then(|r| r.strip_suffix("字上で地付き"))
        && is_digit_run(n)
    {
        return Some(format!("地から{n}字上げ"));
    }
    // この行{N}字下げ → {N}字下げ (the digit-run guard excludes editorial
    // `この行は…N字下げ` prose).
    if let Some(n) = body
        .strip_prefix("この行")
        .and_then(|r| r.strip_suffix("字下げ"))
        && is_digit_run(n)
    {
        return Some(format!("{n}字下げ"));
    }
    None
}

/// Forward-reference forms `「X」…` — substitute the trailing keyword while
/// preserving the `「X」` target. Most rules are guarded on a leading `「`; the
/// sole exception is the bare-parenthesised 縦中横 target, whose `（…）` operand
/// is unquoted and so is normalised (by quoting it) before that guard.
fn forward_form(body: &str) -> Option<String> {
    // `（X）は縦中横` → `「（X）」は縦中横`: the tate-chu-yoko target is an
    // unquoted full-width `（…）` run; quote it so the forward directive
    // resolves. Anchored on the exact tail, so the COMPOUND
    // `（十一）は縦中横、…` form (trailing clause) never matches.
    if let Some(target) = body.strip_suffix("は縦中横")
        && target.starts_with('（')
        && target.ends_with('）')
    {
        return Some(format!("「{target}」は縦中横"));
    }

    if !body.starts_with('「') {
        return None;
    }

    // Keyword drift after the target: swap the trailing variant keyword. Each
    // suffix is distinct and anchored at the body end, so the already-canonical
    // forms (e.g. `は下付き小文字`) cannot re-match (idempotent).
    for (variant_kw, canonical_kw) in [
        ("に黒丸傍点", "に丸傍点"),
        ("は斜体字", "は斜体"),
        ("は中中見出し", "は中見出し"),
        ("の部分はイタリック体", "は斜体"),
        ("は横書き", "は横組み"),
        ("の縦中横", "は縦中横"),
        ("は小書き", "は小文字"),
        ("は下付き", "は下付き小文字"),
        ("は上付き", "は上付き小文字"),
        ("はすべて下付き小文字", "は下付き小文字"),
    ] {
        if let Some(head) = body.strip_suffix(variant_kw) {
            return Some(format!("{head}{canonical_kw}"));
        }
    }

    // Sic-marker annotation notes → the recognised `に「Y」の注記` marginNote
    // form. Bare `ママ` is quoted (scoped to that literal token); a `と注記` or
    // missing-`の` tail gains the `の`. All preserve the `「X」`/`「Y」` targets.
    if let Some(head) = body.strip_suffix("にママの注記") {
        return Some(format!("{head}に「ママ」の注記"));
    }
    if let Some(head) = body.strip_suffix("と注記")
        && head.ends_with('」')
    {
        return Some(format!("{head}の注記"));
    }
    if let Some(head) = body.strip_suffix("注記")
        && head.ends_with('」')
    {
        return Some(format!("{head}の注記"));
    }

    // Missing `は` before `ゴシック体`; `の小文字` drift (only when the head is a
    // bare `「X」` target — excludes the editorial `…ローマ数字の小文字`).
    if let Some(head) = body.strip_suffix("ゴシック体")
        && head.ends_with('」')
    {
        return Some(format!("{head}はゴシック体"));
    }
    if let Some(head) = body.strip_suffix("の小文字")
        && head.ends_with('」')
    {
        return Some(format!("{head}は小文字"));
    }

    // Missing / drifted particle before `傍点` → `に傍点`.
    for variant_kw in ["は傍点", "の傍点", "傍点"] {
        if let Some(head) = body.strip_suffix(variant_kw)
            && head.ends_with('」')
        {
            return Some(format!("{head}に傍点"));
        }
    }
    None
}

/// Concrete sample bodies covering every catalogue tier.
///
/// For the parse-round-trip self-test in the `aozora` crate (which alone can
/// call the parser). Each must satisfy: `canonical_directive(sample)` is
/// `Some`, `［＃sample］` parses to Unknown, and `［＃<canonical>］` parses to a
/// non-Unknown node.
pub const CATALOGUE_SAMPLES: &[&str] = &[
    "字下げ終わり",
    "字下げ終り",
    "字下げおわり",
    "文字下げ終わり",
    "二字下げ終わり",
    "ここから表組",
    "ここで表組終わり",
    "黒丸傍点",
    "中央寄せ",
    "改行を挿入",
    "斜体字",
    "中中見出し",
    "３回り大きな文字",
    "ここか２字下げ",
    "ここより２字下げ",
    "２字下げて",
    "「梅」に黒丸傍点",
    "「梅」は斜体字",
    "「梅」傍点",
    // Line font-size compound (EXACT).
    "中文字、ゴシック体",
    // Forward keyword / particle drift.
    "「文」の部分はイタリック体",
    "「AB」は横書き",
    "「12」の縦中横",
    "「ガ」は小書き",
    "「2」は下付き",
    "「2」は上付き",
    "「abc」はすべて下付き小文字",
    "「GHQ」の小文字",
    "「強調」ゴシック体",
    "「語」は傍点",
    "「語」の傍点",
    // Sic-marker annotation notes.
    "「甫」に「ママ」注記",
    "「甫」にママの注記",
    "「甫」に「ママ」と注記",
    // Bare parenthesised 縦中横 target.
    "（一）は縦中横",
    // Region-close synonyms (EXACT + parameterized).
    "ここで字下げおわり",
    "ここで文字下げ終わり",
    "ここで横書き終わり",
    "ここで左から右への横組み終わり",
    "ここで横組みの表終わり",
    "ここで2字下げ終わり",
    "ここで2段組終わり",
    "ここで1段階小さな文字終わり",
    "ここで字下げ終わり」",
    // Region-open synonyms.
    "地付きで",
    "こ地付き",
    "ここから横書き",
    "以下2字下げ",
    "ここから最後まで2字下げ",
    "ここから2　字下げ",
    // 字上げ / 字下げ numeric.
    "地付き、地より3字アキ",
    "行末から2字上で地付き",
    "この行2字下げ",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_maps_resolve() {
        assert_eq!(canonical_directive("斜体字").as_deref(), Some("斜体"));
        assert_eq!(canonical_directive("中央寄せ").as_deref(), Some("中央揃え"));
        assert_eq!(canonical_directive("改行を挿入").as_deref(), Some("改行"));
        assert_eq!(
            canonical_directive("字下げおわり").as_deref(),
            Some("ここで字下げ終わり")
        );
        assert_eq!(
            canonical_directive("中文字、ゴシック体").as_deref(),
            Some("中文字、太字")
        );
    }

    #[test]
    fn parameterized_preserves_n() {
        assert_eq!(
            canonical_directive("３回り大きな文字").as_deref(),
            Some("３段階大きな文字")
        );
        assert_eq!(
            canonical_directive("ここか２字下げ").as_deref(),
            Some("ここから２字下げ")
        );
        assert_eq!(
            canonical_directive("ここより10字下げ").as_deref(),
            Some("ここから10字下げ")
        );
        assert_eq!(canonical_directive("2字下げて").as_deref(), Some("2字下げ"));
    }

    #[test]
    fn forward_form_preserves_target() {
        assert_eq!(
            canonical_directive("「梅」に黒丸傍点").as_deref(),
            Some("「梅」に丸傍点")
        );
        assert_eq!(
            canonical_directive("「梅」は斜体字").as_deref(),
            Some("「梅」は斜体")
        );
        assert_eq!(
            canonical_directive("「梅」傍点").as_deref(),
            Some("「梅」に傍点")
        );
    }

    #[test]
    fn forward_keyword_particle_drift() {
        for (v, c) in [
            ("「文」の部分はイタリック体", "「文」は斜体"),
            ("「AB」は横書き", "「AB」は横組み"),
            ("「12」の縦中横", "「12」は縦中横"),
            ("「ガ」は小書き", "「ガ」は小文字"),
            ("「2」は下付き", "「2」は下付き小文字"),
            ("「2」は上付き", "「2」は上付き小文字"),
            ("「abc」はすべて下付き小文字", "「abc」は下付き小文字"),
            ("「GHQ」の小文字", "「GHQ」は小文字"),
            ("「強調」ゴシック体", "「強調」はゴシック体"),
            ("「語」は傍点", "「語」に傍点"),
            ("「語」の傍点", "「語」に傍点"),
        ] {
            assert_eq!(canonical_directive(v).as_deref(), Some(c), "variant {v:?}");
            // Idempotent: the emitted canonical is not itself a catalogue key.
            assert_eq!(canonical_directive(c), None, "canonical {c:?} re-matched");
        }
    }

    #[test]
    fn sic_annotation_notes_normalise() {
        for v in [
            "「甫」に「ママ」注記",
            "「甫」にママの注記",
            "「甫」に「ママ」と注記",
        ] {
            assert_eq!(
                canonical_directive(v).as_deref(),
                Some("「甫」に「ママ」の注記"),
                "note variant {v:?}"
            );
        }
        assert_eq!(canonical_directive("「甫」に「ママ」の注記"), None);
    }

    #[test]
    fn bare_paren_tcy_is_quoted() {
        assert_eq!(
            canonical_directive("（一）は縦中横").as_deref(),
            Some("「（一）」は縦中横")
        );
        // Already-quoted and compound-tail forms are left alone.
        assert_eq!(canonical_directive("「（一）」は縦中横"), None);
        assert_eq!(
            canonical_directive("（十一）は縦中横、「十一」は縦組み"),
            None
        );
    }

    #[test]
    fn small_letter_drift_excludes_editorial() {
        // `の小文字` only fires when the head is a bare 「…」 target, so the
        // editorial `…ローマ数字の小文字` prose is never rewritten.
        assert_eq!(
            canonical_directive("「xxxiii」は33を表すローマ数字の小文字"),
            None
        );
    }

    #[test]
    fn region_synonyms_resolve() {
        for (v, c) in [
            ("ここで字下げおわり", "ここで字下げ終わり"),
            ("ここで横書き終わり", "ここで横組み終わり"),
            ("ここで横組みの表終わり", "ここで表終わり"),
            ("地付きで", "地付き"),
            ("こ地付き", "地付き"),
            ("ここから横書き", "ここから横組み"),
        ] {
            assert_eq!(canonical_directive(v).as_deref(), Some(c), "variant {v:?}");
            assert_eq!(canonical_directive(c), None, "canonical {c:?} re-matched");
        }
    }

    #[test]
    fn region_numeric_parameterized() {
        for (v, c) in [
            ("ここで2字下げ終わり", "ここで字下げ終わり"),
            ("ここで3段組終わり", "ここで段組終わり"),
            ("ここで1段階小さな文字終わり", "ここで小さな文字終わり"),
            ("ここで字下げ終わり」", "ここで字下げ終わり"),
            ("以下2字下げ", "ここから2字下げ"),
            ("ここから最後まで4字下げ", "ここから4字下げ"),
            ("ここから2　字下げ", "ここから2字下げ"),
            ("地付き、地より3字アキ", "地から3字上げ"),
            ("地付き、地より3字あき", "地から3字上げ"),
            ("行末から2字上で地付き", "地から2字上げ"),
            ("この行2字下げ", "2字下げ"),
        ] {
            assert_eq!(canonical_directive(v).as_deref(), Some(c), "variant {v:?}");
            assert_eq!(canonical_directive(c), None, "canonical {c:?} re-matched");
        }
    }

    #[test]
    fn region_numeric_excludes_editorial_and_compound() {
        // digit-run guard: editorial `この行は…N字下げ` prose never matches.
        assert_eq!(canonical_directive("この行は底本では３字下げ"), None);
        // A `、`-bearing compound close keeps its second axis (not folded away).
        assert_eq!(canonical_directive("ここで小文字、字下げ終わり"), None);
    }

    #[test]
    fn genuine_editorial_unknown_returns_none() {
        // Real editorial notes must never match — the zero-FP anchor.
        assert!(canonical_directive("底本では「青空」").is_none());
        assert!(canonical_directive("入力者注").is_none());
        assert!(canonical_directive("「」は「」の「」").is_none());
        assert!(canonical_directive("未完").is_none());
        // Already-canonical forms are not our business (never Unknown anyway).
        assert!(canonical_directive("ここで字下げ終わり").is_none());
        assert!(canonical_directive("地より2字上げ").is_none());
    }

    #[test]
    fn every_sample_resolves() {
        for &sample in CATALOGUE_SAMPLES {
            assert!(
                canonical_directive(sample).is_some(),
                "catalogue sample {sample:?} did not resolve"
            );
        }
    }
}
