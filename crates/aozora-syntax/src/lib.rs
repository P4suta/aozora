//! AST types shared between `afm-parser`, `afm-encoding`, and `afm-cli`.
//!
//! Keeping the AST in its own crate (with no parser dep) lets downstream tools consume
//! afm's structured output without pulling in the full `CommonMark` engine.
//!
//! # Invariants
//!
//! - Every [`Span`] refers to byte offsets in the original UTF-8 source buffer. The
//!   offsets are guaranteed to fall on UTF-8 character boundaries; callers can slice
//!   the input with them safely.
//! - Every owned string field uses [`Box<str>`] rather than [`String`]: after parsing,
//!   nodes are immutable, so the capacity field of `String` is dead weight. A
//!   56 kB-annotated work like 『罪と罰』 saves roughly 2 k × 8 B = 16 kB per parse.
//! - Every public enum is `#[non_exhaustive]`. Adding a variant is not a breaking
//!   change; downstream match arms learned the `_` catch-all at compile time.
//!
//! # Classifier methods
//!
//! [`AozoraNode::is_block`], [`AozoraNode::contains_inlines`] and
//! [`AozoraNode::xml_node_name`] exist so comrak's fork can delegate the
//! single-line `NodeValue::Aozora(_)` arm to an AST-level classifier without knowing
//! the shape of individual variants.

#![forbid(unsafe_code)]

use miette::Diagnostic;
use thiserror::Error;

pub mod accent;
mod extension;
pub use extension::{AozoraExtension, BlockCtx, BlockMatch, ContainerKind, InlineCtx, InlineMatch};

/// Byte-range span into the original source document.
///
/// `u32` (rather than `usize`) caps the addressable source at 4 GiB, which is
/// roughly 4 000× the largest plausible Aozora Bunko work — and halves span size on
/// 64-bit targets, which compounds across the thousands of nodes a long novel
/// produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Slice the source buffer by this span. Assumes `self` was produced by the
    /// parser and therefore sits on UTF-8 boundaries.
    #[must_use]
    pub fn slice(self, source: &str) -> &str {
        &source[self.start as usize..self.end as usize]
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

impl AozoraNode {
    /// Whether this node occupies a block (paragraph-level) position in the tree.
    ///
    /// Inline-only variants (`Ruby`, `Bouten`, `TateChuYoko`, `Gaiji`) return `false`.
    /// Block variants (`Indent`, `AlignEnd`, `Warichu`, `Keigakomi`, `PageBreak`,
    /// `SectionBreak`, `AozoraHeading`, `Sashie`) return `true`.
    /// `Annotation` is inline — it stands in for a span of text that the parser
    /// couldn't classify.
    #[must_use]
    pub const fn is_block(&self) -> bool {
        matches!(
            self,
            Self::Indent(_)
                | Self::AlignEnd(_)
                | Self::Warichu(_)
                | Self::Keigakomi(_)
                | Self::PageBreak
                | Self::SectionBreak(_)
                | Self::AozoraHeading(_)
                | Self::Sashie(_)
        )
    }

    /// Whether children of this node (if any) are inline content. Block variants that
    /// wrap an indented run of paragraphs answer `true`; leaf blocks answer `false`.
    #[must_use]
    pub const fn contains_inlines(&self) -> bool {
        matches!(
            self,
            Self::AozoraHeading(_)
                | Self::AlignEnd(_)
                | Self::Warichu(_)
                | Self::Keigakomi(_)
                | Self::Indent(_)
        )
    }

    /// Name used by comrak's XML pretty-printer for nodes of this kind. Stable across
    /// versions; appears in CI fixtures and user-visible diagnostics.
    #[must_use]
    pub const fn xml_node_name(&self) -> &'static str {
        match self {
            Self::Ruby(_) => "aozora_ruby",
            Self::Bouten(_) => "aozora_bouten",
            Self::TateChuYoko(_) => "aozora_tcy",
            Self::Gaiji(_) => "aozora_gaiji",
            Self::Indent(_) => "aozora_indent",
            Self::AlignEnd(_) => "aozora_align_end",
            Self::Warichu(_) => "aozora_warichu",
            Self::Keigakomi(_) => "aozora_keigakomi",
            Self::PageBreak => "aozora_page_break",
            Self::SectionBreak(_) => "aozora_section_break",
            Self::AozoraHeading(_) => "aozora_heading",
            Self::Sashie(_) => "aozora_sashie",
            Self::Annotation(_) => "aozora_annotation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ruby {
    pub base: Box<str>,
    pub reading: Box<str>,
    /// `true` when the base was delimited by `｜`, `false` when inferred from the
    /// trailing kanji run before `《》`.
    pub delim_explicit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub text: Box<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Gaiji {
    /// Description text from the source, e.g. "木＋吶のつくり".
    pub description: Box<str>,
    /// Resolved Unicode scalar, if the gaiji maps to one.
    pub ucs: Option<char>,
    /// Raw mencode reference (e.g. "第3水準1-85-54" or "U+XXXX, page-line").
    pub mencode: Option<Box<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub upper: Box<str>,
    pub lower: Box<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub text: Box<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sashie {
    pub file: Box<str>,
    pub caption: Option<Box<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Annotation {
    pub raw: Box<str>,
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
    UnknownKind { kind: Box<str> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruby_roundtrip_fields() {
        let r = Ruby {
            base: "青梅".into(),
            reading: "おうめ".into(),
            delim_explicit: true,
        };
        assert_eq!(&*r.base, "青梅");
        assert_eq!(&*r.reading, "おうめ");
        assert!(r.delim_explicit);
    }

    #[test]
    fn bouten_target_span_is_independent_of_children() {
        let b = Bouten {
            kind: BoutenKind::Goma,
            target: Span::new(10, 20),
        };
        assert_eq!(b.target.start, 10);
        assert_eq!(b.target.end, 20);
        assert_eq!(b.target.len(), 10);
        assert!(!b.target.is_empty());
    }

    #[test]
    fn empty_span_is_empty_and_zero_length() {
        let s = Span::new(42, 42);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn span_slices_source_buffer() {
        let source = "hello world";
        let s = Span::new(6, 11);
        assert_eq!(s.slice(source), "world");
    }

    #[test]
    fn inline_variants_are_not_block() {
        assert!(
            !AozoraNode::Ruby(Ruby {
                base: "a".into(),
                reading: "a".into(),
                delim_explicit: false,
            })
            .is_block()
        );
        assert!(!AozoraNode::TateChuYoko(TateChuYoko { text: "12".into() }).is_block());
    }

    #[test]
    fn block_variants_are_block() {
        assert!(AozoraNode::PageBreak.is_block());
        assert!(AozoraNode::Indent(Indent { amount: 2 }).is_block());
        assert!(AozoraNode::SectionBreak(SectionKind::Choho).is_block());
    }

    #[test]
    fn containers_report_contains_inlines() {
        assert!(AozoraNode::Indent(Indent { amount: 2 }).contains_inlines());
        assert!(
            AozoraNode::Warichu(Warichu {
                upper: "a".into(),
                lower: "b".into(),
            })
            .contains_inlines()
        );
    }

    #[test]
    fn leaf_blocks_do_not_contain_inlines() {
        assert!(!AozoraNode::PageBreak.contains_inlines());
        assert!(!AozoraNode::SectionBreak(SectionKind::Choho).contains_inlines());
    }

    #[test]
    fn xml_node_names_are_stable_and_unique() {
        use std::collections::BTreeSet;
        let samples: [AozoraNode; 13] = [
            AozoraNode::Ruby(Ruby {
                base: "".into(),
                reading: "".into(),
                delim_explicit: false,
            }),
            AozoraNode::Bouten(Bouten {
                kind: BoutenKind::Goma,
                target: Span::new(0, 0),
            }),
            AozoraNode::TateChuYoko(TateChuYoko { text: "".into() }),
            AozoraNode::Gaiji(Gaiji {
                description: "".into(),
                ucs: None,
                mencode: None,
            }),
            AozoraNode::Indent(Indent { amount: 0 }),
            AozoraNode::AlignEnd(AlignEnd { offset: 0 }),
            AozoraNode::Warichu(Warichu {
                upper: "".into(),
                lower: "".into(),
            }),
            AozoraNode::Keigakomi(Keigakomi),
            AozoraNode::PageBreak,
            AozoraNode::SectionBreak(SectionKind::Choho),
            AozoraNode::AozoraHeading(AozoraHeading {
                kind: AozoraHeadingKind::Window,
                text: "".into(),
            }),
            AozoraNode::Sashie(Sashie {
                file: "".into(),
                caption: None,
            }),
            AozoraNode::Annotation(Annotation {
                raw: "".into(),
                kind: AnnotationKind::Unknown,
            }),
        ];
        let names: BTreeSet<&'static str> = samples.iter().map(AozoraNode::xml_node_name).collect();
        assert_eq!(names.len(), samples.len(), "xml node names must be unique");
        for name in &names {
            assert!(
                name.starts_with("aozora_"),
                "xml name '{name}' missing aozora_ prefix"
            );
        }
    }

    #[test]
    fn all_variants_are_non_exhaustive_for_forward_compat() {
        match AozoraNode::PageBreak {
            AozoraNode::PageBreak => {}
            _ => unreachable!(),
        }
    }
}
