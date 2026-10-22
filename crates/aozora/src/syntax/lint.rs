use std::borrow::Cow;

const EXACT: &[(&str, &str)] = &[
    ("字下げ終わり", "ここで字下げ終わり"),
    ("字下げ終り", "ここで字下げ終わり"),
    ("字下げおわり", "ここで字下げ終わり"),
    ("文字下げ終わり", "ここで字下げ終わり"),
    ("二字下げ終わり", "ここで字下げ終わり"),
    ("ここから表組", "ここから表"),
    ("ここで表組終わり", "ここで表終わり"),
    ("黒丸傍点", "丸傍点"),
    ("中央寄せ", "中央揃え"),
    ("改行を挿入", "改行"),
    ("斜体字", "斜体"),
    ("中中見出し", "中見出し"),
    ("ゴチック", "ゴシック体"),
    ("ここからゴチック", "ここからゴシック体"),
    ("ここでゴチック終わり", "ここでゴシック体終わり"),
    ("傍点（白丸）", "白丸傍点"),
    ("傍点◎", "二重丸傍点"),
    ("中見出終わり", "中見出し終わり"),
    ("大見出終わり", "大見出し終わり"),
    ("小見出終わり", "小見出し終わり"),
    ("ここで中見出終わり", "ここで中見出し終わり"),
    ("ここで大見出終わり", "ここで大見出し終わり"),
    ("ここで小見出終わり", "ここで小見出し終わり"),
    ("ここで字下げおわり", "ここで字下げ終わり"),
    ("ここで字下げ終り", "ここで字下げ終わり"),
    ("ここで文字下げ終わり", "ここで字下げ終わり"),
    ("ここで横書き終わり", "ここで横組み終わり"),
    ("ここで左から右への横組み終わり", "ここで横組み終わり"),
    ("ここで横組みの表終わり", "ここで表終わり"),
    ("地付きで", "地付き"),
    ("こ地付き", "地付き"),
    ("ここから横書き", "ここから横組み"),
];

#[must_use]
pub(crate) fn canonical_directive(body: &str) -> Option<Cow<'static, str>> {
    if let Some((_, canonical)) = EXACT.iter().find(|(variant, _)| *variant == body) {
        return Some(Cow::Borrowed(canonical));
    }
    parameterized(body)
        .or_else(|| forward_form(body))
        .map(Cow::Owned)
}

pub(crate) fn is_digit_run(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

fn parameterized(body: &str) -> Option<String> {
    parameterized_font_size(body)
        .or_else(|| parameterized_indent_open(body))
        .or_else(|| parameterized_region_close(body))
        .or_else(|| parameterized_align(body))
}

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

fn parameterized_indent_open(body: &str) -> Option<String> {
    for bad in ["ここか", "ここより", "以下"] {
        if let Some(n) = body
            .strip_prefix(bad)
            .and_then(|rest| rest.strip_suffix("字下げ"))
            && is_digit_run(n)
        {
            return Some(format!("ここから{n}字下げ"));
        }
    }
    if let Some(n) = body
        .strip_prefix("ここから")
        .and_then(|r| r.strip_suffix("　字下げ"))
        && is_digit_run(n)
    {
        return Some(format!("ここから{n}字下げ"));
    }
    None
}

fn parameterized_region_close(body: &str) -> Option<String> {
    let inner = body.strip_prefix("ここで")?;
    if let Some(n) = inner.strip_suffix("字下げ終わり")
        && is_digit_run(n)
    {
        return Some("ここで字下げ終わり".to_owned());
    }
    for size in ["大きな", "小さな"] {
        if let Some(n) = inner.strip_suffix(&format!("段階{size}文字終わり"))
            && is_digit_run(n)
        {
            return Some(format!("ここで{size}文字終わり"));
        }
    }
    if let Some(head) = body.strip_suffix('」')
        && head.ends_with("終わり")
    {
        return Some(head.to_owned());
    }
    None
}

fn parameterized_align(body: &str) -> Option<String> {
    if let Some(n) = body.strip_suffix("字下げて")
        && is_digit_run(n)
    {
        return Some(format!("{n}字下げ"));
    }
    if let Some(n) = body
        .strip_prefix("この行")
        .and_then(|r| r.strip_suffix("字下げ"))
        && is_digit_run(n)
    {
        return Some(format!("{n}字下げ"));
    }
    None
}

fn forward_form(body: &str) -> Option<String> {
    if let Some(target) = body.strip_suffix("は縦中横")
        && target.starts_with('（')
        && target.ends_with('）')
    {
        return Some(format!("「{target}」は縦中横"));
    }

    if !body.starts_with('「') {
        return None;
    }

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
        ("は地付け", "は地付き"),
        ("はゴチック", "はゴシック体"),
        ("は枠囲み", "は罫囲み"),
        ("は枠囲い", "は罫囲み"),
        ("に枠囲み", "は罫囲み"),
        ("に枠囲い", "は罫囲み"),
        ("は横一列", "は縦中横"),
        ("に傍点（白丸）", "に白丸傍点"),
        ("に傍点◎", "に二重丸傍点"),
    ] {
        if let Some(head) = body.strip_suffix(variant_kw) {
            return Some(format!("{head}{canonical_kw}"));
        }
    }

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

    for variant_kw in ["は傍点", "の傍点", "傍点"] {
        if let Some(head) = body.strip_suffix(variant_kw)
            && head.ends_with('」')
        {
            return Some(format!("{head}に傍点"));
        }
    }
    None
}

#[cfg(test)]
pub(crate) const CATALOGUE_SAMPLES: &[&str] = &[
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
    "「文」の部分はイタリック体",
    "「AB」は横書き",
    "「12」の縦中横",
    "「ガ」は小書き",
    "「2」は下付き",
    "「2」は上付き",
    "「abc」はすべて下付き小文字",
    "「幕。」は地付け",
    "「GHQ」の小文字",
    "「強調」ゴシック体",
    "「語」は傍点",
    "「語」の傍点",
    "ゴチック",
    "ここでゴチック終わり",
    "傍点（白丸）",
    "傍点◎",
    "中見出終わり",
    "ここで中見出終わり",
    "「梅」はゴチック",
    "「梅」は枠囲み",
    "「梅」に枠囲い",
    "「!?」は横一列",
    "「意志」に傍点（白丸）",
    "「意志」に傍点◎",
    "「甫」に「ママ」注記",
    "「甫」にママの注記",
    "「甫」に「ママ」と注記",
    "（一）は縦中横",
    "ここで字下げおわり",
    "ここで文字下げ終わり",
    "ここで横書き終わり",
    "ここで左から右への横組み終わり",
    "ここで横組みの表終わり",
    "ここで2字下げ終わり",
    "ここで1段階小さな文字終わり",
    "ここで字下げ終わり」",
    "地付きで",
    "こ地付き",
    "ここから横書き",
    "以下2字下げ",
    "ここから2　字下げ",
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
        assert_eq!(canonical_directive("中文字、ゴシック体"), None);
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
            ("「幕。」は地付け", "「幕。」は地付き"),
            ("「GHQ」の小文字", "「GHQ」は小文字"),
            ("「強調」ゴシック体", "「強調」はゴシック体"),
            ("「語」は傍点", "「語」に傍点"),
            ("「語」の傍点", "「語」に傍点"),
        ] {
            assert_eq!(canonical_directive(v).as_deref(), Some(c), "variant {v:?}");
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
        assert_eq!(canonical_directive("「（一）」は縦中横"), None);
        assert_eq!(
            canonical_directive("（十一）は縦中横、「十一」は縦組み"),
            None
        );
    }

    #[test]
    fn small_letter_drift_excludes_editorial() {
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
            ("ここで1段階小さな文字終わり", "ここで小さな文字終わり"),
            ("ここで字下げ終わり」", "ここで字下げ終わり"),
            ("以下2字下げ", "ここから2字下げ"),
            ("ここから2　字下げ", "ここから2字下げ"),
            ("この行2字下げ", "2字下げ"),
        ] {
            assert_eq!(canonical_directive(v).as_deref(), Some(c), "variant {v:?}");
            assert_eq!(canonical_directive(c), None, "canonical {c:?} re-matched");
        }
    }

    #[test]
    fn region_numeric_excludes_editorial_and_compound() {
        assert_eq!(canonical_directive("この行は底本では３字下げ"), None);
        assert_eq!(canonical_directive("ここで小文字、字下げ終わり"), None);
    }

    #[test]
    fn genuine_editorial_unknown_returns_none() {
        assert!(canonical_directive("底本では「青空」").is_none());
        assert!(canonical_directive("入力者注").is_none());
        assert!(canonical_directive("「」は「」の「」").is_none());
        assert!(canonical_directive("未完").is_none());
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
