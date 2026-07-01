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

/// Digit-preserving rules — the `{N}` run is copied verbatim.
fn parameterized(body: &str) -> Option<String> {
    // {N}回り大きな/小さな文字 → {N}段階…文字.
    for size in ["大きな文字", "小さな文字"] {
        if let Some(n) = body
            .strip_suffix(size)
            .and_then(|head| head.strip_suffix("回り"))
            && is_digit_run(n)
        {
            return Some(format!("{n}段階{size}"));
        }
    }
    // ここか{N}字下げ / ここより{N}字下げ → ここから{N}字下げ.
    for bad in ["ここか", "ここより"] {
        if let Some(n) = body
            .strip_prefix(bad)
            .and_then(|rest| rest.strip_suffix("字下げ"))
            && is_digit_run(n)
        {
            return Some(format!("ここから{n}字下げ"));
        }
    }
    // {N}字下げて → {N}字下げ.
    if let Some(n) = body.strip_suffix("字下げて")
        && is_digit_run(n)
    {
        return Some(format!("{n}字下げ"));
    }
    None
}

/// Forward-reference forms `「X」…` — substitute the trailing keyword while
/// preserving the `「X」` target. Guarded on a leading `「` so a bare body
/// never matches.
fn forward_form(body: &str) -> Option<String> {
    if !body.starts_with('「') {
        return None;
    }
    // Keyword drift after the target: replace the trailing variant keyword.
    for (variant_kw, canonical_kw) in [
        ("に黒丸傍点", "に丸傍点"),
        ("は斜体字", "は斜体"),
        ("は中中見出し", "は中見出し"),
    ] {
        if let Some(head) = body.strip_suffix(variant_kw) {
            return Some(format!("{head}{canonical_kw}"));
        }
    }
    // Missing に particle: `「X」傍点` → `「X」に傍点`.
    if let Some(head) = body.strip_suffix("傍点")
        && head.ends_with('」')
    {
        return Some(format!("{head}に傍点"));
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
