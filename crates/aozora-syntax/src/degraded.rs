//! Tier2 degraded-form matcher for the notation-hygiene layers (ADR-0026).
//!
//! [`degraded_directive`] maps a `［＃…］` directive body that the parser keeps
//! as `DirectiveKind::Unknown` — and that Tier1 ([`crate::lint::canonical_directive`])
//! deliberately declines — to a directly parser-recognized spelling. Where
//! Tier1 is a *zero-false-positive* map of verified near-misses that lose no
//! meaning, Tier2 is the opt-in home for the reductions Tier1 must refuse
//! because they are **lossy** (a spelling or scope the parser preserves is
//! erased) or **judgment-laden** (two measurement vocabularies folded via a
//! typographic identity, not a spelling repair).
//!
//! Contract (looser input, identical output rigor to Tier1):
//! - Input: a degraded body Tier1's map refuses (`canonical_directive` is `None`).
//! - Output: a **directly** parser-recognized spelling — never a Tier1 *key*,
//!   because the opt-in renderer does a single serialize→lex pass
//!   ([`crate::lint`] doc), so a Tier1-key output would re-lex to Unknown and
//!   render inert.
//! - Disjoint from Tier1 and idempotent: `degraded_directive` is `None` for
//!   every Tier1 key and for its own outputs; `canonical_directive` is `None`
//!   for every Tier2 key.
//!
//! Invoked **only** by the opt-in renderer/interpreter
//! (`RenderOptions::directives == Degraded` / `aozora render --degraded`) — never
//! by the parser, the default lint, the default `fmt`, or `fmt --fix`.
//! Because `DirectiveNormalization::Degraded` is constructed at a single
//! ephemeral render site, a Tier2 misfire can reach only `--degraded` render
//! output; it never rewrites source. See ADR-0026.

use std::borrow::Cow;

use crate::lint::is_digit_run;

/// Map a degraded `［＃…］` directive body to a recognized spelling, or `None`.
///
/// `body` is the trimmed inner text, without the `［＃` / `］` delimiters. See
/// the module docs for the Tier1/Tier2 contract. Each rule is here rather than
/// in Tier1 because it loses or re-derives meaning: admitting it there lets
/// `fmt --fix` rewrite source lossily and hides the loss behind a purely
/// syntactic self-test.
#[must_use]
pub fn degraded_directive(body: &str) -> Option<Cow<'static, str>> {
    // (The former D1 — line-scope 中文字、ゴシック体 → 中文字、太字 — was removed
    // in #435: ゴシック体 is now a first-class gothic construct distinct from
    // 太字, so folding it to 太字 is a meaning change, not a faithful render.
    // The rare 中文字、ゴシック体 size+gothic compound stays a verbatim Unknown.)

    // D2 — ここから最後まで{N}字下げ → ここから{N}字下げ. LOSSY: 最後まで marks an
    // indent that auto-closes at document/section end; the parser has no
    // until-end concept. As a render-only reduction this is faithful — the
    // EOF-drain closes the open block at document end, which is what 最後まで
    // means — but as a *source* rewrite it would strand an unclosed block, so
    // it must never reach fmt.
    if let Some(n) = body
        .strip_prefix("ここから最後まで")
        .and_then(|r| r.strip_suffix("字下げ"))
        && is_digit_run(n)
    {
        return Some(Cow::Owned(format!("ここから{n}字下げ")));
    }

    // D3/D4 — 地付き、地より{N}字アキ / 字あき → 地から{N}字上げ. JUDGMENT: folds
    // two measurement vocabularies (アキ = blank space from the bottom margin;
    // 上げ = raised from the bottom) via the typographic identity
    // "gap-from-edge ≡ raised-from-edge". Also a `、`-joined compound, which
    // Tier1 otherwise excludes.
    for tail in ["字アキ", "字あき"] {
        if let Some(n) = body
            .strip_prefix("地付き、地より")
            .and_then(|r| r.strip_suffix(tail))
            && is_digit_run(n)
        {
            return Some(Cow::Owned(format!("地から{n}字上げ")));
        }
    }

    // D5 — 行末から{N}字上で地付き → 地から{N}字上げ. JUDGMENT: re-projects a
    // line-end + up + geochi description onto the bottom-anchored raise leaf.
    if let Some(n) = body
        .strip_prefix("行末から")
        .and_then(|r| r.strip_suffix("字上で地付き"))
        && is_digit_run(n)
    {
        return Some(Cow::Owned(format!("地から{n}字上げ")));
    }

    // D6 — 下げて[、]地より{N}字あきで / 字アキで → 地から{N}字上げ. JUDGMENT/LOSSY,
    // sibling of D3/D4. The both-margin parser only anchors when a leading 字下げ
    // *count* is present, so the count-less 下げて head-indent has no needle and the
    // whole body falls to Unknown. The trailing 地より{N}字あきで is the D3/D4
    // gap-from-bottom, folded onto the bottom-anchored raise leaf via the same
    // アキ≡上げ identity; the unquantified head-indent is dropped — lossy, hence
    // render-only. Both the comma'd (下げて、地より) and bare (下げて地より) corpus
    // spellings resolve.
    for prefix in ["下げて、地より", "下げて地より"] {
        for tail in ["字あきで", "字アキで"] {
            if let Some(n) = body.strip_prefix(prefix).and_then(|r| r.strip_suffix(tail))
                && is_digit_run(n)
            {
                return Some(Cow::Owned(format!("地から{n}字上げ")));
            }
        }
    }

    // D7 — 「X」は縦中横、行右/左小書き → 「X」は縦中横. LOSSY: a single-target
    // compound whose primary 縦中横 axis renders faithfully while the secondary
    // 行右/左小書き annotation is dropped. The parser declines the whole compound
    // to a lossless Unknown rather than silently folding away the small-script
    // axis — that silent drop was the pre-#435 data-loss bug (ADR-0027 A5) — so
    // rendering it as bare 縦中横 is the faithful render-only approximation and
    // lives here, never in fmt. Anchored on the exact tail plus a bare 「X」 head,
    // so the two-target `…、「乙」は…` compounds (which style a *different* target
    // and have no single faithful reduction) never match.
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

/// Concrete degraded sample bodies, one per rule family above.
///
/// For the parse-round-trip + meaning self-tests in the `aozora` crate (which
/// alone can call the parser). Each must satisfy: `degraded_directive(sample)`
/// is `Some`, `canonical_directive(sample)` is `None` (disjoint from Tier1),
/// `［＃sample］` parses to Unknown, `［＃<output>］` parses to a non-Unknown
/// node, and `degraded_directive(output)` is `None` (idempotent).
pub const DEGRADED_SAMPLES: &[&str] = &[
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
        // D6 — both the comma'd and bare 下げて…字あきで spellings.
        assert_eq!(
            degraded_directive("下げて、地より2字あきで").as_deref(),
            Some("地から2字上げ")
        );
        assert_eq!(
            degraded_directive("下げて地より2字あきで").as_deref(),
            Some("地から2字上げ")
        );
        // D7 — single-target 縦中横 compound drops the 行右/左小書き axis.
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
        // A recognized-already form, an editorial note, and a plain typo the
        // catalogue must not touch.
        assert_eq!(degraded_directive("中文字、太字"), None);
        assert_eq!(degraded_directive("「甲」は「乙」の誤記か"), None);
        assert_eq!(degraded_directive("ここから3字下げ"), None);
        assert_eq!(degraded_directive("字下げ終わり"), None);
        // D7 refuses a two-target compound: `、「乙」は…` styles a different
        // target, so there is no single faithful reduction (stays Unknown).
        assert_eq!(
            degraded_directive("「甲」は縦中横、「乙」は上付き小書き"),
            None
        );
        // …and refuses a bare tail with no 「X」 head.
        assert_eq!(degraded_directive("は縦中横、行右小書き"), None);
    }
}
