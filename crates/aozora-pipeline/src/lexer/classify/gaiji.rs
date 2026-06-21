//! Gaiji (外字) reference recognition.
//!
//! The `※［＃<description>、<mencode>］` glyph-reference form plus the
//! `mencode` / 底本ページ-行 validators. Extracted verbatim from the
//! classify-stage classifier; recognised nodes are built on the shared
//! [`RecogniseCtx`].

#[cfg(feature = "classify-instrument")]
use super::super::instrumentation::{Subsystem, SubsystemGuard};
use super::super::pair::{PairEvent, PairKind};
use super::super::token::TriggerKind;
use super::{BodyView, RecogniseCtx};
use aozora_encoding::gaiji as gaiji_resolve;
use aozora_syntax::{Span, borrowed};

/// Intermediate result of `recognize_gaiji`.
///
/// Holds the payload (`&'a borrowed::Gaiji<'a>`) rather than a wrapped
/// node so the caller can route it to either `alloc.gaiji(p)`
/// (top-level span) or `alloc.seg_gaiji(p)` (nested inside a body
/// content) without re-paying the description / mencode intern cost.
pub(super) struct GaijiMatch<'a> {
    pub(super) payload: &'a borrowed::Gaiji<'a>,
    pub(super) consume_start: u32,
    pub(super) consume_end: u32,
}

/// Try to recognize a gaiji reference at `events[refmark_idx]`.
///
/// Shape: `※［＃<description>、<mencode>］` or `※［＃<description>］`.
/// The description may be wrapped in `「…」` (the common form) or
/// appear bare. `<mencode>` is the mencode reference (`第3水準1-85-54`,
/// `U+XXXX`, etc.) appearing after a `、` separator.
///
/// The UCS resolution column of `Gaiji` is populated by
/// `aozora_encoding::gaiji::lookup` before the recogniser returns, so
/// downstream consumers receive a resolved `Option<char>` without
/// having to re-probe the mencode table.
///
/// Event preconditions (checked):
/// * `events[refmark_idx]` is `Solo(RefMark)` [done by caller]
/// * `events[refmark_idx + 1]` is `PairOpen(Bracket)` [done by caller]
/// * `events[refmark_idx + 2]` is `Solo(Hash)` [checked here]
///
/// Consume range is from `refmark_span.start` to the bracket close's
/// end — i.e. the `※` and the entire following `［＃…］` fold into
/// one Aozora span.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    pub(super) fn recognize_gaiji(
        &mut self,
        view: BodyView<'_>,
        refmark_span: Span,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch<'a>> {
        self.recognize_gaiji_core(view, refmark_span.start, false, bracket_open_idx)
    }

    /// Core recogniser shared by the `※`-prefixed [`Self::recognize_gaiji`]
    /// and the no-refmark standalone (#122) form. `consume_start` is the
    /// byte the gaiji span folds back to (`※` start for the refmark form,
    /// `［` start for standalone); `standalone` records whether the source
    /// carried a `※` so the serializer can round-trip the bracket verbatim.
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "a flat gaiji body recogniser (quoted form, bare form, validation, \
                  resolution) over four independent inputs; splitting it would scatter \
                  the I3-idempotency reasoning"
    )]
    pub(super) fn recognize_gaiji_core(
        &mut self,
        view: BodyView<'_>,
        consume_start: u32,
        standalone: bool,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch<'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Gaiji);
        let events = view.events;
        let &PairEvent::PairOpen {
            kind: PairKind::Bracket,
            ..
        } = events.get(bracket_open_idx)?
        else {
            return None;
        };
        let bracket_close_link = *view.links.get(bracket_open_idx)?;
        if bracket_close_link == u32::MAX {
            return None;
        }
        let bracket_close_idx = bracket_close_link as usize;
        let hash_end = match events.get(bracket_open_idx + 1)? {
            PairEvent::Solo {
                kind: TriggerKind::Hash,
                span,
            } => span.end,
            _ => return None,
        };
        let &PairEvent::PairClose {
            span: bracket_close_span,
            ..
        } = events.get(bracket_close_idx)?
        else {
            return None;
        };

        // Try the quoted-description form first: `「DESC」、MENCODE`. Two
        // events after open: PairOpen(Quote).
        let quote_open_idx = bracket_open_idx + 2;
        let quoted = events.get(quote_open_idx).and_then(|ev| match *ev {
            PairEvent::PairOpen {
                kind: PairKind::Quote,
                span: qos,
            } => {
                let qci_link = *view.links.get(quote_open_idx)?;
                if qci_link == u32::MAX {
                    return None;
                }
                let qci = qci_link as usize;
                if qci >= bracket_close_idx {
                    return None;
                }
                let PairEvent::PairClose { span: qcs, .. } = *events.get(qci)? else {
                    return None;
                };
                let desc = &self.source[qos.end as usize..qcs.start as usize];
                if desc.is_empty() {
                    return None;
                }
                // A quoted body whose inner content itself carries `「」`
                // structure is an outer-wrapped composed / 正字 form
                // (`「…）、「柿」の正字」、mencode`). Stripping the outer `「…」`
                // here would lose it on re-serialize: `emit_gaiji` writes a
                // quote-bearing description verbatim (no wrapper). Decline so
                // the bare extractor captures the whole `「…」`-wrapped body
                // verbatim and the round-trip stays a fixed point.
                if desc.contains(['「', '」']) {
                    return None;
                }
                let tail = self.source[qcs.end as usize..bracket_close_span.start as usize].trim();
                // The simple quoted form is `「desc」` optionally followed by
                // `、mencode`. If extra structure follows the first quote
                // (e.g. the composed-glyph form `「X」の「Y」に代えて「Z」、
                // mencode`), this is not a one-quote gaiji — decline so the
                // bare extractor captures the whole verbatim description
                // instead of silently dropping it (a round-trip data loss).
                if !tail.is_empty() && !tail.starts_with('、') {
                    return None;
                }
                let mencode = tail.strip_prefix('、').map(str::trim);
                Some((desc.to_owned(), mencode.map(str::to_owned)))
            }
            _ => None,
        });

        let (description, mencode) = if let Some(pair) = quoted {
            pair
        } else {
            self.extract_bare_gaiji_body(hash_end, bracket_close_span.start)?
        };

        if description.is_empty()
            || !gaiji_description_serializable(&description, mencode.is_some())
        {
            // A non-serializable description (stray quote imbalance, an
            // embedded `［＃`, …) falls through to `Directive{Unknown}`,
            // which round-trips byte-identical. See
            // [`gaiji_description_serializable`].
            return None;
        }

        // Resolve the Unicode scalar at lex time via the static table in
        // aozora-encoding so the downstream AST / renderer never has to
        // re-probe. `None` stays `None` when the mencode has no mapping
        // entry and no `U+XXXX` shape matches — the renderer falls back
        // to escaping the raw `description`. A trailing 底本ページ-行 suffix is
        // stripped for resolution (the men-ku-ten still maps the glyph) while
        // the full mencode is stored verbatim for round-trip.
        let ucs = gaiji_resolve::lookup(
            None,
            mencode.as_deref().map(mencode_resolution_token),
            &description,
        );

        let payload = self
            .alloc
            .make_gaiji(&description, ucs, mencode.as_deref(), standalone);
        Some(GaijiMatch {
            payload,
            consume_start,
            consume_end: bracket_close_span.end,
        })
    }

    /// Bare-form `※［＃DESC、MENCODE］` body extraction. Kept to
    /// accommodate the historical form `※［＃二の字点、1-2-23］` (no
    /// `「…」` quotes) that some Aozora source uses, plus the composed-glyph
    /// (`「X」の「Y」に代えて「Z」`) / 正字 / 屋号 forms the quoted extractor
    /// declines, tightened so the I3 idempotency property holds.
    ///
    /// The body is split on `、` and the tokens are walked **from the right**:
    /// the maximal trailing run that are each a JIS men-ku-ten / `U+XXXX`
    /// (`is_mencode_shaped`) or a 底本ページ-行 (`is_page_line_shaped`) token
    /// is the `mencode` (kept verbatim, joined with `、`); the text before it
    /// is the `description`. Scanning from the right is what lets a `、` that
    /// belongs to the *description* (`…面から一、二画目をとったもの`,
    /// `…）、「柿」の正字`, `…読みは「はく」`, `…、屋号を示す記号`) stay in the
    /// description instead of cutting it at the wrong `、`.
    ///
    /// Guarantees (round-trip / idempotency):
    ///   - the run must be non-empty (a real JIS / U+ / page-line anchor) and
    ///     there must be a description before it, else the bracket falls
    ///     through to `Directive::Unknown` and round-trips byte-identical;
    ///   - the last description token is non-shaped by construction, so
    ///     re-parsing the serialised `DESC、MENCODE` re-splits at the same
    ///     boundary (the trailing run is maximal and self-consistent).
    ///
    /// The 底本ページ-行 suffix is kept inside `mencode` verbatim; resolution
    /// strips it via [`mencode_resolution_token`] (the men-ku-ten still maps
    /// the glyph), so `小書き片仮名ヲ、5-下-3` stays `ucs = None` while
    /// `…、第3水準1-85-57` resolves.
    fn extract_bare_gaiji_body(
        &self,
        hash_end: u32,
        bracket_close_start: u32,
    ) -> Option<(String, Option<String>)> {
        let body = self.source[hash_end as usize..bracket_close_start as usize].trim();
        let shaped = |t: &str| is_mencode_shaped(t) || is_page_line_shaped(t);
        // Byte offsets of every `、` separator, and the trimmed tokens between
        // them (trimming only affects shape-testing; the description / mencode
        // are sliced from `body` so internal spacing round-trips).
        let commas: Vec<usize> = body.match_indices('、').map(|(i, _)| i).collect();
        let tokens: Vec<&str> = body.split('、').map(str::trim).collect();
        // First token index (from the left) of the maximal trailing shaped run.
        let mut run_start = tokens.len();
        while run_start > 0 && shaped(tokens[run_start - 1]) {
            run_start -= 1;
        }
        // Need a non-empty mencode run *and* a description before it.
        if run_start == tokens.len() || run_start == 0 {
            return None;
        }
        // The separator before `tokens[run_start]` is the `run_start`-th `、`.
        let boundary = commas[run_start - 1];
        let description = body[..boundary].trim();
        let mencode = body[boundary + '、'.len_utf8()..].trim();
        Some((description.to_owned(), Some(mencode.to_owned())))
    }
}

/// The JIS / U+ token of a `mencode`, dropping any trailing 底本ページ-行
/// suffix (`第3水準1-84-27、144-上-9` → `第3水準1-84-27`, `U+74FC、372-10` →
/// `U+74FC`) so the resolver sees a clean men-ku-ten / codepoint.
fn mencode_resolution_token(mencode: &str) -> &str {
    mencode
        .split_once('、')
        .map_or(mencode, |(token, _)| token.trim())
}

/// Whether `s` is a 底本ページ-行 reference — the trailing column in the
/// corpus' 外字 凡例 (`144-上-9`, `372-10`, `1-13-25`): `-`-joined parts, each
/// a run of ASCII / full-width digits, a 上 / 中 / 下 column marker, or a volume
/// marker (`上巻` / `下巻` / `7巻` in multi-volume 底本 — `上巻-34-18`,
/// `7巻-42-下-10`).
fn is_page_line_shaped(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(is_page_line_part)
}

/// One `-`-separated component of a 底本ページ-行 reference.
fn is_page_line_part(p: &str) -> bool {
    // A volume marker keeps the column / numbered prefix before `巻`
    // (`上巻` → `上`, `7巻` → `7`).
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    matches!(p, "上" | "中" | "下") || is_digit_run(p)
}

/// A non-empty run of ASCII or full-width decimal digits.
fn is_digit_run(p: &str) -> bool {
    !p.is_empty()
        && p.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

/// Whether a gaiji `description` can be kept (it both serializes and
/// round-trips); otherwise the bracket falls through to `Directive{Unknown}`.
///
/// Rejects:
///   - a description embedding `［＃` (a nested annotation opener would leak a
///     bare `［＃` outside the `aozora-directive` wrapper, violating the Tier A
///     canary), and
///   - a description carrying structural `「…」` quotes, *except* the
///     composed-glyph / 正字 / 屋号 forms (`「X」の「Y」に代えて「Z」`,
///     `「…）、「柿」の正字」`, `「…」、屋号を示す記号`; corpus §6 external-character
///     forms): balanced quotes anchored by a trailing `、mencode`. `emit_gaiji`
///     writes such a description verbatim (no `「…」` wrapper), so it round-trips
///     even when it carries an internal `、` — the recogniser's right-to-left
///     mencode scan (`extract_bare_gaiji_body`) re-splits at the same
///     boundary. A quote-bearing description without a mencode anchor stays
///     rejected (the serializer's wrapper would unbalance it).
fn gaiji_description_serializable(description: &str, has_mencode: bool) -> bool {
    if description.contains("［＃") {
        return false;
    }
    if description.contains(['「', '」']) {
        let balanced = description.matches('「').count() == description.matches('」').count();
        return balanced && has_mencode;
    }
    true
}

/// Loose validator for a bare-form gaiji mencode field.
///
/// Accepts:
///   - `N-N-N` (plain JIS X 0213 row-cell, digits + ASCII hyphens)
///   - `第N水準N-N-N` (same with the explicit `水準` label)
///   - `U+XXXX` (1–6 ASCII hex digits)
///
/// Anything else — random kanji, half-/full-width punctuation, etc.
/// — is rejected so the bare-description bracket falls through to
/// `Directive::Unknown` instead of getting serializer-promoted into
/// a quoted Gaiji. See `recognize_gaiji` for the I3-idempotency
/// rationale.
fn is_mencode_shaped(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("U+") {
        return !hex.is_empty() && hex.len() <= 6 && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // Optional `第N水準` prefix: skip past the digits + `水準` token
    // if present, then validate the remainder as `N-N-N` (digits and
    // ASCII hyphens only, must contain at least one digit).
    let rest = s
        .strip_prefix('第')
        .and_then(|after_dai| {
            let nondigit = after_dai.find(|c: char| !c.is_ascii_digit())?;
            let (_digits, tail) = after_dai.split_at(nondigit);
            tail.strip_prefix("水準")
        })
        .unwrap_or(s);
    !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '-')
        && rest.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod is_mencode_shaped_tests {
    use super::{is_mencode_shaped, is_page_line_shaped, mencode_resolution_token};

    #[test]
    fn page_line_shaped_accepts_corpus_forms() {
        assert!(is_page_line_shaped("372-10"));
        assert!(is_page_line_shaped("144-上-9"));
        assert!(is_page_line_shaped("1-13-25"));
        assert!(is_page_line_shaped("１４４-下-９"));
        // Multi-volume 底本 carry a 巻 marker before the page/line.
        assert!(is_page_line_shaped("上巻-34-18"));
        assert!(is_page_line_shaped("下巻-68-4"));
        assert!(is_page_line_shaped("7巻-42-下-10"));
        assert!(!is_page_line_shaped(""));
        assert!(!is_page_line_shaped("漢字"));
        assert!(!is_page_line_shaped("U+304B"));
        // `巻` alone (no column / number prefix) is not a page-line part.
        assert!(!is_page_line_shaped("巻-3-4"));
    }

    #[test]
    fn mencode_token_drops_trailing_page_line() {
        assert_eq!(
            mencode_resolution_token("第3水準1-84-27、144-上-9"),
            "第3水準1-84-27"
        );
        assert_eq!(mencode_resolution_token("U+74FC、372-10"), "U+74FC");
        // No page-line suffix — returned unchanged.
        assert_eq!(mencode_resolution_token("第3水準1-85-54"), "第3水準1-85-54");
    }

    #[test]
    fn jis_x_0213_row_cell_passes() {
        assert!(is_mencode_shaped("1-2-23"));
        assert!(is_mencode_shaped("1-85-54"));
    }

    #[test]
    fn dai_n_suijun_prefix_passes() {
        assert!(is_mencode_shaped("第3水準1-85-54"));
        assert!(is_mencode_shaped("第4水準2-1-2"));
    }

    #[test]
    fn u_plus_codepoint_passes() {
        assert!(is_mencode_shaped("U+304B"));
        assert!(is_mencode_shaped("U+1F94A"));
    }

    #[test]
    fn random_garbage_rejected() {
        assert!(!is_mencode_shaped("》"));
        assert!(!is_mencode_shaped("abc"));
        assert!(!is_mencode_shaped("漢字"));
        assert!(!is_mencode_shaped(""));
        assert!(!is_mencode_shaped("U+"));
        assert!(!is_mencode_shaped("U+ZZZZ"));
        assert!(!is_mencode_shaped("第3水準"));
    }
}
