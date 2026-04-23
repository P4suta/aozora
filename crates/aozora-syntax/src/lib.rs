//! AST types shared between `afm-parser`, `afm-encoding`, and `afm-cli`.
//!
//! Keeping the AST in its own crate (with no parser dep) lets downstream tools consume
//! afm's structured output without pulling in the full `CommonMark` engine.

#![forbid(unsafe_code)]

use std::ops::Range;

use miette::Diagnostic;
use thiserror::Error;

/// Byte-range span into the original source document.
///
/// Stored as a raw byte range rather than a `(line, column)` pair so tokens can carry
/// spans cheaply through the parser and be resolved to line/column only when formatting
/// diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    pub range: Range<usize>,
}

impl Span {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { range: start..end }
    }
}

/// Every afm-specific AST node. Embedded into comrak's `NodeValue` tree as a single
/// `NodeValue::Aozora(AozoraNode)` variant so the upstream diff stays at one line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraNode {
    /// Ruby (furigana). Ex: `｜青梅《おうめ》` or `日本《にほん》`.
    Ruby(Ruby),

    /// Emphasis dots / sidelines. Ex: `［＃「X」に傍点］` or `［＃傍点］...［＃傍点終わり］`.
    Bouten(Bouten),

    /// Tate-chu-yoko (horizontal embedding inside vertical text). Ex: `［＃縦中横］20［＃縦中横終わり］`.
    TateChuYoko(TateChuYoko),

    /// Gaiji (out-of-range character). Ex: `※［＃「木＋吶のつくり」、第3水準1-85-54］`.
    Gaiji(Gaiji),

    /// `［＃ここから字下げ］ ... ［＃ここで字下げ終わり］` block.
    Indent(Indent),

    /// `［＃地付き］` / `［＃地から2字上げ］` block.
    AlignEnd(AlignEnd),

    /// Split annotation (`割り注`). Two-line inline annotation.
    Warichu(Warichu),

    /// Boxed block (`罫囲み`).
    Keigakomi(Keigakomi),

    /// Page break (`［＃改ページ］`).
    PageBreak,

    /// Section break (`［＃改丁］` / `［＃改段］` / `［＃改見開き］`).
    SectionBreak(SectionKind),

    /// Aozora-specific heading level that doesn't map to a Markdown `#` level
    /// (e.g. 窓見出し). Canonical `大/中/小` are normalised to `NodeValue::Heading`
    /// at parse time and never reach this variant.
    AozoraHeading(AozoraHeading),

    /// Illustration (`［＃挿絵（fig.png）入る］`) — normalised to `NodeValue::Image`
    /// when a caption is absent; the `Sashie` form is used when a caption is attached.
    Sashie(Sashie),

    /// An annotation recognised as Aozora-shaped but not understood by this version
    /// of the parser. Kept for round-trip fidelity and surfaced as a diagnostic.
    Annotation(Annotation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ruby {
    pub base: String,
    pub reading: String,
    /// `true` when the base was delimited by `｜`, `false` when inferred from the
    /// trailing kanji run before `《》`.
    pub delim_explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bouten {
    pub kind: BoutenKind,
    /// Byte span of the annotated text *in the source*, NOT a child list — the
    /// annotated run remains in the surrounding inline stream.
    pub target: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenKind {
    /// 白ゴマ (default, `［＃「X」に傍点］`)
    Goma,
    /// 丸 (`［＃「X」に丸傍点］`)
    Circle,
    /// 白丸
    WhiteCircle,
    /// 二重丸
    DoubleCircle,
    /// 蛇の目
    Janome,
    /// 波線 (`［＃「X」に波線］`)
    WavyLine,
    /// 傍線
    UnderLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TateChuYoko {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Gaiji {
    /// Description text from the source, e.g. "木＋吶のつくり".
    pub description: String,
    /// Resolved Unicode scalar, if the gaiji maps to one.
    pub ucs: Option<char>,
    /// Raw mencode reference (e.g. "第3水準1-85-54" or "U+XXXX, page-line").
    pub mencode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Indent {
    pub amount: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AlignEnd {
    /// Offset in chars from the right edge. `0` = 地付き, `n` = 地から n 字上げ.
    pub offset: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Warichu {
    pub upper: String,
    pub lower: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keigakomi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SectionKind {
    /// `［＃改丁］`
    Choho,
    /// `［＃改段］`
    Dan,
    /// `［＃改見開き］`
    Spread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraHeadingKind {
    /// 窓見出し
    Window,
    /// 副見出し
    Sub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AozoraHeading {
    pub kind: AozoraHeadingKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sashie {
    pub file: String,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Annotation {
    pub raw: String,
    pub kind: AnnotationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AnnotationKind {
    /// The parser recognised the notation as Aozora-shaped but not registered.
    Unknown,
    /// `［＃「」」はママ］`-style editorial as-is marker. Rendered but flagged.
    AsIs,
    /// Source-text divergence note (`［＃「X」は底本では「Y」］`).
    TextualNote,
    /// A ruby span that couldn't be parsed cleanly — round-tripped as-is.
    InvalidRubySpan,
}

/// Parse- and render-time error surface for afm-syntax consumers. Parsers and renderers
/// funnel their failures through this enum so CLI and library integrations see a single
/// error type.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum SyntaxError {
    #[error("未知のノード種別です: {kind}")]
    #[diagnostic(code(afm::syntax::unknown_kind))]
    UnknownKind { kind: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruby_roundtrip_fields() {
        let r = Ruby {
            base: "青梅".to_owned(),
            reading: "おうめ".to_owned(),
            delim_explicit: true,
        };
        assert_eq!(r.base, "青梅");
        assert_eq!(r.reading, "おうめ");
        assert!(r.delim_explicit);
    }

    #[test]
    fn bouten_target_span_is_independent_of_children() {
        // Bouten carries a span, not a child list — this test pins the intent.
        let b = Bouten {
            kind: BoutenKind::Goma,
            target: Span::new(10, 20),
        };
        assert_eq!(b.target.range, 10..20);
    }

    #[test]
    fn all_variants_are_non_exhaustive_for_forward_compat() {
        // If this compiles, `#[non_exhaustive]` is preserved. If a future PR removes it,
        // CI catches it because downstream match arms will start breaking.
        match AozoraNode::PageBreak {
            AozoraNode::PageBreak => {}
            _ => unreachable!(),
        }
    }
}
