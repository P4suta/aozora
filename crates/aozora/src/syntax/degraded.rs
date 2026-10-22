// ADR-0026 confines lossy reductions to temporary rendering to preserve source meaning.

use std::borrow::Cow;

use crate::syntax::lint::is_digit_run;

#[must_use]
pub(crate) fn degraded_directive(body: &str) -> Option<Cow<'static, str>> {
    // ADR-0026 keeps this end-of-document closure out of persistent source to avoid an unclosed block.
    if let Some(n) = body
        .strip_prefix("ここから最後まで")
        .and_then(|r| r.strip_suffix("字下げ"))
        && is_digit_run(n)
    {
        return Some(Cow::Owned(format!("ここから{n}字下げ")));
    }

    // ADR-0026 equates the bottom-margin gap with the lift from that margin.
    for tail in ["字アキ", "字あき"] {
        if let Some(n) = body
            .strip_prefix("地付き、地より")
            .and_then(|r| r.strip_suffix(tail))
            && is_digit_run(n)
        {
            return Some(Cow::Owned(format!("地から{n}字上げ")));
        }
    }

    if let Some(n) = body
        .strip_prefix("行末から")
        .and_then(|r| r.strip_suffix("字上で地付き"))
        && is_digit_run(n)
    {
        return Some(Cow::Owned(format!("地から{n}字上げ")));
    }

    // ADR-0026 confines this reduction to rendering because it drops an indent without a specified width.
    for prefix in ["下げて、地より", "下げて地より"] {
        for tail in ["字あきで", "字アキで"] {
            if let Some(n) = body.strip_prefix(prefix).and_then(|r| r.strip_suffix(tail))
                && is_digit_run(n)
            {
                return Some(Cow::Owned(format!("地から{n}字上げ")));
            }
        }
    }

    // ADR-0027 preserves small-script information in source, so only temporary rendering may omit it.
    for tail in ["は縦中横、行右小書き", "は縦中横、行左小書き"] {
        if let Some(head) = body.strip_suffix(tail)
            && head.starts_with('「')
            && head.ends_with('」')
        {
            return Some(Cow::Owned(format!("{head}は縦中横")));
        }
    }

    None
}

#[cfg(test)]
pub(crate) const DEGRADED_SAMPLES: &[&str] = &[
    "ここから最後まで3字下げ",
    "地付き、地より3字アキ",
    "地付き、地より3字あき",
    "行末から2字上で地付き",
    "下げて、地より3字あきで",
    "下げて、地より3字アキで",
    "下げて地より3字あきで",
    "「甲」は縦中横、行右小書き",
    "「乙」は縦中横、行左小書き",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_families_reduce_to_their_canonical_spelling() {
        assert_eq!(
            degraded_directive("ここから最後まで3字下げ").as_deref(),
            Some("ここから3字下げ")
        );
        assert_eq!(
            degraded_directive("地付き、地より3字アキ").as_deref(),
            Some("地から3字上げ")
        );
        assert_eq!(
            degraded_directive("地付き、地より3字あき").as_deref(),
            Some("地から3字上げ")
        );
        assert_eq!(
            degraded_directive("行末から2字上で地付き").as_deref(),
            Some("地から2字上げ")
        );
        assert_eq!(
            degraded_directive("下げて、地より2字あきで").as_deref(),
            Some("地から2字上げ")
        );
        assert_eq!(
            degraded_directive("下げて地より2字あきで").as_deref(),
            Some("地から2字上げ")
        );
        assert_eq!(
            degraded_directive("「１）」は縦中横、行右小書き").as_deref(),
            Some("「１）」は縦中横")
        );
        assert_eq!(
            degraded_directive("「甲」は縦中横、行左小書き").as_deref(),
            Some("「甲」は縦中横")
        );
    }

    #[test]
    fn outputs_are_idempotent() {
        for sample in DEGRADED_SAMPLES {
            let out = degraded_directive(sample).expect("sample reduces");
            assert_eq!(
                degraded_directive(&out),
                None,
                "degraded output {out} must not re-reduce"
            );
        }
    }

    #[test]
    fn genuine_unknown_returns_none() {
        assert_eq!(degraded_directive("中文字、太字"), None);
        assert_eq!(degraded_directive("「甲」は「乙」の誤記か"), None);
        assert_eq!(degraded_directive("ここから3字下げ"), None);
        assert_eq!(degraded_directive("字下げ終わり"), None);
        assert_eq!(
            degraded_directive("「甲」は縦中横、「乙」は上付き小書き"),
            None
        );
        assert_eq!(degraded_directive("は縦中横、行右小書き"), None);
    }
}
