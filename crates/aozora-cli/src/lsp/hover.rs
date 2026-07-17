//! `textDocument/hover` — gaiji (外字) reference resolution.
//!
//! When the cursor sits inside a `※［＃description、mencode］` (or the
//! `U+XXXX` variant) token, returns a Markdown block that shows the
//! resolved character (via `aozora::encoding::gaiji::lookup`), the raw
//! description, and the mencode. Misses (cursor not in a gaiji span,
//! malformed body) return `None` and the editor falls back to no hover.
//!
//! This intentionally re-parses a small surrounding window rather than
//! asking the full afm lexer: the hover handler runs on every cursor
//! move, and the full lex + splice is ~linear in document length. The
//! small textual scan here is `O(local window)` which matters for
//! long documents. The trade-off is that malformed input outside the
//! window is ignored — we'd rather show nothing than a wrong
//! resolution.

use std::ops::Range as ByteRange;

use crate::i18n::{self as i18n, LanguageIdentifier};
use aozora::encoding::gaiji;
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

const GAIJI_OPEN: &str = "※［＃";
const GAIJI_CLOSE: &str = "］";

/// Compute a hover, if any, at `position` in `source`, with the Markdown body
/// prose rendered in `lang`.
#[must_use]
pub(super) fn hover_at(
    source: &str,
    position: Position,
    lang: &LanguageIdentifier,
) -> Option<Hover> {
    let byte_offset = position_to_byte_offset(source, position)?;
    let span = find_gaiji_span(source, byte_offset)?;
    let body = &source[span.start + GAIJI_OPEN.len()..span.end - GAIJI_CLOSE.len()];
    let (description, mencode) = parse_gaiji_body(body);
    let resolved = gaiji::lookup(None, mencode.as_deref(), &description);
    let markdown = render_markdown(lang, &description, mencode.as_deref(), resolved);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(Range::new(
            byte_offset_to_position(source, span.start),
            byte_offset_to_position(source, span.end),
        )),
    })
}

/// Byte-range of a `※［＃…］` span that contains `byte_offset`, or
/// `None` if no such span exists around the cursor.
///
/// # Locality bound
///
/// A correct gaiji span is at most a few-hundred bytes long (the
/// description plus the mencode plus optional page-line ref). The
/// hover handler runs on every cursor move, so a full-document scan
/// here was `O(n)` per stroke — easily 1 MB walked on a long
/// translation. We bound the search to a window of
/// [`MAX_GAIJI_SPAN_LEN`] bytes either side of the cursor, snapped
/// to UTF-8 boundaries; that gives `O(1)` lookup independent of
/// document size while still covering the realistic span lengths
/// used by Aozora Bunko.
///
/// # Boundary correctness
///
/// The window extends forward by the same margin so a `※［＃` whose
/// start index equals the cursor is still caught (a prefix ending at
/// `byte_offset` wouldn't yet contain the trigram). The walk inside the
/// window is `O(window_size)`, i.e. constant.
const MAX_GAIJI_SPAN_LEN: usize = 512;

fn find_gaiji_span(source: &str, byte_offset: usize) -> Option<ByteRange<usize>> {
    if source.is_empty() {
        return None;
    }
    let win_start =
        snap_to_char_boundary_left(source, byte_offset.saturating_sub(MAX_GAIJI_SPAN_LEN));
    let win_end = snap_to_char_boundary_right(
        source,
        byte_offset
            .saturating_add(MAX_GAIJI_SPAN_LEN)
            .min(source.len()),
    );
    let window = &source[win_start..win_end];
    let win_offset = byte_offset.saturating_sub(win_start);

    for (start_in_win, _) in window.match_indices(GAIJI_OPEN) {
        let after_open = start_in_win + GAIJI_OPEN.len();
        let Some(end_rel) = window.get(after_open..).and_then(|s| s.find(GAIJI_CLOSE)) else {
            continue;
        };
        let end_in_win = after_open + end_rel + GAIJI_CLOSE.len();
        if (start_in_win..end_in_win).contains(&win_offset) {
            return Some((win_start + start_in_win)..(win_start + end_in_win));
        }
    }
    None
}

const fn snap_to_char_boundary_left(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

const fn snap_to_char_boundary_right(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    while idx < len && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Split a gaiji body (`「description」、mencode[、page-line]`) into
/// `(description, mencode?)`. The Aozora annotation manual attaches
/// optional trailing fields after the mencode (page-line references
/// for `U+XXXX` mencodes); they are informational only, so we
/// pick the first comma-separated entry after the description.
fn parse_gaiji_body(body: &str) -> (String, Option<String>) {
    let body = body.trim();
    let (description, rest) = body.find('「').map_or_else(
        || (body.to_owned(), ""),
        |open_idx| {
            let after_open = &body[open_idx + '「'.len_utf8()..];
            after_open.find('」').map_or_else(
                || (body.to_owned(), ""),
                |close_rel| {
                    let desc = after_open[..close_rel].to_owned();
                    let rest = &after_open[close_rel + '」'.len_utf8()..];
                    (desc, rest)
                },
            )
        },
    );
    let rest = rest.trim_start_matches('、').trim();
    let mencode = rest
        .split('、')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    (description, mencode)
}

fn render_markdown(
    lang: &LanguageIdentifier,
    description: &str,
    mencode: Option<&str>,
    resolved: Option<gaiji::Resolved>,
) -> String {
    use core::fmt::Write as _;
    // Prose (header + labels) comes from the shared i18n catalog; the notation
    // glyphs, backticks and `U+XXXX` formatting are locale-neutral structure.
    let resolved_label = i18n::t(lang, "lsp-hover-resolved-label");
    let mut md = format!("{}\n\n", i18n::t(lang, "lsp-hover-gaiji-header"));
    match resolved {
        Some(gaiji::Resolved::Char(ch)) => {
            // `write!` into the existing buffer avoids the intermediate
            // `format!() -> String` allocation that the workspace
            // `format_push_string` lint flags.
            _ = writeln!(md, "- {resolved_label}: `{ch}` (U+{:04X})", ch as u32);
        }
        Some(gaiji::Resolved::Multi(s)) => {
            // Multi-codepoint cells render their full sequence plus
            // the explicit list of constituent scalars so the user
            // can see the composition (`か゚` = U+304B + U+309A).
            let codepoints: Vec<String> =
                s.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
            _ = writeln!(
                md,
                "- {resolved_label}: `{s}` ({}: {})",
                i18n::t(lang, "lsp-hover-composed-seq-label"),
                codepoints.join(" + ")
            );
        }
        None => {
            _ = writeln!(
                md,
                "- {resolved_label}: {}",
                i18n::t(lang, "lsp-hover-unresolved")
            );
        }
    }
    _ = writeln!(
        md,
        "- {}: `{description}`",
        i18n::t(lang, "lsp-hover-description-label")
    );
    if let Some(m) = mencode {
        _ = writeln!(md, "- mencode: `{m}`");
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    /// Shim mirroring the pre-i18n `hover_at(source, position)` signature,
    /// pinned to the canonical English catalog so the structural assertions
    /// below stay locale-stable regardless of the host `LANG`.
    fn hover_at(source: &str, position: Position) -> Option<Hover> {
        super::hover_at(source, position, &lang("en"))
    }

    #[test]
    fn hover_on_gaiji_returns_markdown_with_resolved_char() {
        let src = "語※［＃「木＋吶のつくり」、第3水準1-85-54］で";
        // byte offset 6 は「※」の次、gaiji span 内
        let pos = byte_offset_to_position(src, 6);
        let hover = hover_at(src, pos).expect("hover should resolve");
        let md = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markdown hover"),
        };
        assert!(md.contains("外字"), "hover missing 外字 header: {md}");
        assert!(
            md.contains("木＋吶のつくり"),
            "hover missing description: {md}"
        );
        // JIS X 0213:2004 plane 1 row 85 cell 54 = 枘 (U+6798).
        // (`木＋吶のつくり` = 木+内 = 枘.)
        assert!(
            md.contains("枘") || md.contains("6798"),
            "hover missing resolved character U+6798 (枘): {md}",
        );
    }

    #[test]
    fn hover_on_u_plus_form_resolves_codepoint() {
        let src = "※［＃「description」、U+01F5］";
        let pos = byte_offset_to_position(src, 3);
        let hover = hover_at(src, pos).expect("hover on U+ form");
        let md = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!(),
        };
        assert!(md.contains("01F5") || md.contains('\u{01F5}'));
    }

    #[test]
    fn hover_outside_gaiji_returns_none() {
        let src = "ただの文です";
        let pos = Position::new(0, 2);
        assert!(hover_at(src, pos).is_none());
    }

    #[test]
    fn hover_before_gaiji_returns_none() {
        // `abc` 部分にカーソル (offset 0-2) があれば None
        let src = "abc※［＃「木＋吶のつくり」、第3水準1-85-54］で";
        let pos = byte_offset_to_position(src, 1);
        assert!(hover_at(src, pos).is_none());
    }

    #[test]
    fn hover_on_unresolved_gaiji_still_returns_markdown() {
        // 辞書未登録の mencode は character=None で返るが、hover 自体は出す
        let src = "※［＃「未知字」、第9水準9-99-99］";
        let pos = byte_offset_to_position(src, 3);
        let hover = hover_at(src, pos).expect("hover should fire even if unresolved");
        let md = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!(),
        };
        assert!(md.contains("no dictionary match"));
        assert!(md.contains("未知字"));
    }

    /// Cursor exactly on the leading `※` byte must still resolve the
    /// containing gaiji span. Earlier `rfind`-based detection missed
    /// this boundary because the prefix ending at the cursor didn't
    /// yet contain the trigram. Pin the boundary explicitly.
    #[test]
    fn hover_on_leading_kome_byte_resolves_span() {
        let src = "前※［＃「desc」、第3水準1-85-54］後";
        let kome_byte = src.find('※').unwrap();
        let pos = byte_offset_to_position(src, kome_byte);
        assert!(
            hover_at(src, pos).is_some(),
            "cursor on the leading ※ must still hover the span",
        );
    }
    /// Cursor on the closing `］` byte resolves the same span — tests
    /// the inclusive-end side of the window scan.
    #[test]
    fn hover_on_closing_bracket_byte_resolves_span() {
        let src = "前※［＃「desc」、第3水準1-85-54］後";
        let close_byte = src.rfind('］').unwrap();
        let pos = byte_offset_to_position(src, close_byte);
        assert!(
            hover_at(src, pos).is_some(),
            "cursor on the closing ］ must still hover the span",
        );
    }
    /// Hover on a span far past the locality window (a pathological
    /// `MAX_GAIJI_SPAN_LEN` distance) returns None — the window
    /// bound is what keeps hover O(1) regardless of doc length, so
    /// pin that the window cap is honoured.
    #[test]
    fn hover_far_outside_window_returns_none() {
        // Place a gaiji span at the start, then cursor deep into a
        // long tail. The cursor's window is [cursor - MAX, cursor + MAX],
        // so spans before that window must NOT resolve.
        let span = "※［＃「desc」、第3水準1-85-54］";
        let tail: String = "x".repeat(MAX_GAIJI_SPAN_LEN * 2 + 50);
        let src = format!("{span}{tail}");
        let cursor_byte = src.len();
        let pos = byte_offset_to_position(&src, cursor_byte);
        assert!(
            hover_at(&src, pos).is_none(),
            "span sits before the cursor's hover window, must not resolve",
        );
    }

    /// Empty source must not panic on a hover call. Defensive guard
    /// pin — the early `source.is_empty()` short-circuit is what
    /// prevents the window-snap math from going through `&""[0..0]`
    /// arithmetic that some prior versions miscomputed.
    #[test]
    fn hover_on_empty_source_returns_none_without_panic() {
        assert!(hover_at("", Position::new(0, 0)).is_none());
        assert!(hover_at("", Position::new(99, 99)).is_none());
    }

    /// The gaiji hover header + labels come from the shared i18n catalog, so
    /// each locale gets its own prose; `en` is the default asserted throughout
    /// via the shim.
    #[test]
    fn hover_prose_localizes_by_lang() {
        let src = "※［＃「木＋吶のつくり」、第3水準1-85-54］";
        let pos = byte_offset_to_position(src, 3);
        let body = |tag: &str| match super::hover_at(src, pos, &lang(tag))
            .expect("hover fires")
            .contents
        {
            HoverContents::Markup(m) => m.value,
            HoverContents::Scalar(_) | HoverContents::Array(_) => unreachable!("markdown hover"),
        };
        let ja = body("ja");
        assert!(ja.contains("外字 (gaiji)"), "ja header: {ja}");
        assert!(
            ja.contains("解決") && ja.contains("記述"),
            "ja labels: {ja}"
        );
        let zh = body("zh");
        assert!(
            zh.contains("解析") && zh.contains("描述"),
            "zh labels: {zh}"
        );
    }

    // --- direct helper pins (mutation kills) ---------------------------

    /// `snap_to_char_boundary_left` walks an index left until it lands on
    /// a UTF-8 boundary (or 0). Pinning both interior and on-boundary
    /// inputs against exact offsets kills the loop-guard and step
    /// mutations.
    #[test]
    fn snap_left_lands_on_prior_char_boundary() {
        // "aあ": 'a' at byte 0, 'あ' (U+3042, 3 bytes) at bytes 1..4.
        // Char boundaries: {0, 1, 4}; bytes 2 and 3 are interior.
        let s = "aあ";
        // Interior bytes snap back to the start of 'あ' (byte 1). This is
        // the single most load-bearing assertion: a body replaced with
        // `0` returns 0; `>`→`<`/`==` never enters the loop and returns 2;
        // `&&`→`||` over-decrements to 0; `-=`→`+=` overshoots to 4;
        // `-=`→`/=` spins forever (timeout). Real answer is 1.
        assert_eq!(snap_to_char_boundary_left(s, 2), 1);
        assert_eq!(snap_to_char_boundary_left(s, 3), 1);
        // On-boundary and terminal indices are returned unchanged.
        assert_eq!(snap_to_char_boundary_left(s, 1), 1);
        assert_eq!(snap_to_char_boundary_left(s, 4), 4);
        assert_eq!(snap_to_char_boundary_left(s, 0), 0);
    }

    /// `snap_to_char_boundary_right` walks an index right to the next
    /// UTF-8 boundary (or `len`). Mirror pins for its loop guard and step.
    #[test]
    fn snap_right_lands_on_next_char_boundary() {
        // "aあ": boundaries {0, 1, 4}; len 4; interior bytes 2 and 3.
        let s = "aあ";
        // Interior bytes advance to the end of 'あ' (byte 4). Kills
        // `<`→`==`/`>` (never enters, returns 2), `+=`→`-=` (retreats to
        // 1), `+=`→`*=` (spins forever, timeout).
        assert_eq!(snap_to_char_boundary_right(s, 2), 4);
        assert_eq!(snap_to_char_boundary_right(s, 3), 4);
        // A real boundary below `len` must be returned untouched: this is
        // what distinguishes `&&`→`||` (would run to 4) and the deleted
        // `!` (would advance past the boundary to the next interior stop,
        // 2) from correct behaviour.
        assert_eq!(snap_to_char_boundary_right(s, 1), 1);
        assert_eq!(snap_to_char_boundary_right(s, 0), 0);
        assert_eq!(snap_to_char_boundary_right(s, 4), 4);
    }

    /// The span returned by `find_gaiji_span` must be the *absolute* byte
    /// range `win_start + start_in_win ..`, not a product. Pushing the
    /// cursor past `MAX_GAIJI_SPAN_LEN` forces a non-zero `win_start` so
    /// `+`→`*` diverges (`98 + 502 = 600` vs `98 * 502 = 49196`).
    #[test]
    fn find_gaiji_span_returns_absolute_range_past_window_origin() {
        let pad = "x".repeat(600);
        let span = "※［＃「desc」、第3水準1-85-54］";
        let src = format!("{pad}{span}");
        let kome = src.find('※').unwrap(); // == 600, > MAX_GAIJI_SPAN_LEN
        let cursor = kome + 10; // inside the span, on an ASCII boundary
        assert_eq!(find_gaiji_span(&src, cursor), Some(kome..src.len()));
    }

    /// `parse_gaiji_body` slices the description starting at
    /// `open_idx + '「'.len_utf8()`. Leading text makes `open_idx` = 2,
    /// so `+`→`*` shifts the slice start from byte 5 to byte 6 and drops
    /// the first description character (`desc` → `esc`).
    #[test]
    fn parse_gaiji_body_slices_description_after_open_quote() {
        let (description, mencode) = parse_gaiji_body("xy「desc」、mc");
        assert_eq!(description, "desc");
        assert_eq!(mencode.as_deref(), Some("mc"));
    }
}
