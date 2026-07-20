//! `textDocument/hover` — gaiji (外字) reference resolution.
//!
//! When the cursor sits inside a `※［＃description、mencode］` (or the
//! `U+XXXX` variant) token, returns a Markdown block that shows the
//! resolved character, the raw
//! description, and the mencode. Misses (cursor not in a gaiji span,
//! malformed body) return `None` and the editor falls back to no hover.
//!
use crate::i18n::{self as i18n, LanguageIdentifier};
use aozora::Snapshot;
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use crate::lsp::position::{byte_offset_to_position, position_to_byte_offset};

/// Compute a hover, if any, at `position`, with the Markdown body
/// prose rendered in `lang`.
#[must_use]
pub(super) fn hover_at(
    snapshot: &Snapshot,
    position: Position,
    lang: &LanguageIdentifier,
) -> Option<Hover> {
    let source = snapshot.source();
    let byte_offset = position_to_byte_offset(source, position)?;
    let resolution = snapshot.gaiji_resolution_at(byte_offset)?;
    let markdown = render_markdown(
        lang,
        resolution.description(),
        resolution.mencode(),
        resolution.resolved(),
    );
    let span = resolution.span();
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(Range::new(
            byte_offset_to_position(source, span.start as usize),
            byte_offset_to_position(source, span.end as usize),
        )),
    })
}

fn render_markdown(
    lang: &LanguageIdentifier,
    description: &str,
    mencode: Option<&str>,
    resolved: Option<&str>,
) -> String {
    use core::fmt::Write as _;
    // Prose (header + labels) comes from the shared i18n catalog; the notation
    // glyphs, backticks and `U+XXXX` formatting are locale-neutral structure.
    let resolved_label = i18n::t(lang, "lsp-hover-resolved-label");
    let mut md = format!("{}\n\n", i18n::t(lang, "lsp-hover-gaiji-header"));
    match resolved {
        Some(value) if value.chars().count() == 1 => {
            let ch = value.chars().next().expect("one-character value");
            // `write!` into the existing buffer avoids the intermediate
            // `format!() -> String` allocation that the workspace
            // `format_push_string` lint flags.
            _ = writeln!(md, "- {resolved_label}: `{ch}` (U+{:04X})", ch as u32);
        }
        Some(s) => {
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

    fn hover_at(source: &str, position: Position) -> Option<Hover> {
        let document = aozora::parse(source).expect("test source is within parser limit");
        super::hover_at(&document.snapshot(), position, &lang("en"))
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
    fn single_scalar_resolution_uses_scalar_markdown() {
        assert_eq!(
            render_markdown(&lang("en"), "description", None, Some("枘")),
            "**Gaiji (外字)**\n\n\
             - Resolved: `枘` (U+6798)\n\
             - Description: `description`\n",
        );
    }

    #[test]
    fn multi_scalar_resolution_uses_composed_markdown() {
        assert_eq!(
            render_markdown(&lang("en"), "description", None, Some("か゚")),
            "**Gaiji (外字)**\n\n\
             - Resolved: `か゚` (composed sequence: U+304B + U+309A)\n\
             - Description: `description`\n",
        );
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
    #[test]
    fn hover_far_outside_span_returns_none() {
        let span = "※［＃「desc」、第3水準1-85-54］";
        let tail: String = "x".repeat(1_074);
        let src = format!("{span}{tail}");
        let cursor_byte = src.len();
        let pos = byte_offset_to_position(&src, cursor_byte);
        assert!(hover_at(&src, pos).is_none());
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
        let document = aozora::parse(src).expect("test source is within parser limit");
        let snapshot = document.snapshot();
        let body = |tag: &str| match super::hover_at(&snapshot, pos, &lang(tag))
            .expect("hover fires")
            .contents
        {
            HoverContents::Markup(m) => m.value,
            HoverContents::Scalar(_) | HoverContents::Array(_) => {
                unreachable!("markdown hover")
            }
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
}
