//! AST type definitions for the aozora parser.
//!
//! # AST shape
//!
//! The **sole AST** is the borrowed-AST defined in [`borrowed`]:
//! arena-allocated, `Copy`-able, deduplicated through
//! [`borrowed::Interner`]. Public consumers (`aozora` meta crate,
//! FFI / WASM / Python drivers, CLI) parse via
//! `aozora::Document::parse()` and walk a `borrowed::Node<'_>`.
//!
//! # Top-level surface
//!
//! Only the **shared `Copy`-able payloads** referenced by the borrowed
//! AST (`BoutenKind`, `BoutenPosition`, `Indent`, `AlignEnd`,
//! `Container`, `ContainerKind`, `Framed`, `SectionKind`,
//! `HeadingKind`, `EmphasisKind`, `DirectiveKind`) live at the
//! top level. The
//! borrowed-AST node types live under `borrowed::`. The arena-backed
//! builder lives under `alloc::`.

#![forbid(unsafe_code)]

use miette::Diagnostic;
use thiserror::Error;

pub mod accent;
pub mod alloc;
pub mod borrowed;
mod extension;
pub mod node_kind;

pub use extension::{ContainerKind, IndentLayout};
pub use node_kind::NodeKind;

/// Byte-range span into the original source document.
///
/// Re-exported from [`aozora_spec::Span`] — see that module for the
/// canonical definition.
pub use aozora_spec::Span;

/// Paired block container payload: carries only the kind descriptor.
///
/// Children live in the AST as the container node's children
/// (the `post_process` paired-container splice reparents them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Container {
    /// Which container family this open marker begins.
    pub kind: ContainerKind,
}

/// Which 傍点 (emphasis dot) or 傍線 (sideline) mark decorates a run.
///
/// Carried by both the forward-reference [`borrowed::Bouten`] leaf and the
/// paired [`ContainerKind::BoutenRange`]. The 点 (dot) vs 線 (line) split —
/// see [`Self::is_line`] — is the family boundary the
/// `mismatched_bouten_container` diagnostic enforces. Each variant maps to a
/// canonical 青空文庫 keyword via [`Self::keyword`]; [`BOUTEN_KINDS`] is the
/// single declaration-order list the rest of the workspace derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenKind {
    /// ゴマ
    Goma,
    /// 白ゴマ
    WhiteSesame,
    /// 丸
    Circle,
    /// 白丸
    WhiteCircle,
    /// 二重丸
    DoubleCircle,
    /// 蛇の目
    Janome,
    /// ばつ
    Cross,
    /// 白三角
    WhiteTriangle,
    /// 波線
    WavyLine,
    /// 傍線
    UnderLine,
    /// 二重傍線
    DoubleUnderLine,
    /// 鎖線
    ChainLine,
    /// 破線
    DashedLine,
    /// 黒三角
    BlackTriangle,
}

impl BoutenKind {
    /// Whether this is a 傍線 (line) variant rather than a 傍点 (dot)
    /// variant. The 点/線 split is the *family* boundary used by
    /// `mismatched_bouten_container`: a `［＃傍点］` range closed by a
    /// `［＃傍線終わり］` (or vice-versa) is the mismatch the diagnostic
    /// reports.
    #[must_use]
    pub const fn is_line(self) -> bool {
        matches!(
            self,
            Self::WavyLine
                | Self::UnderLine
                | Self::DoubleUnderLine
                | Self::ChainLine
                | Self::DashedLine
        )
    }

    /// Stable family tag (`"傍点"` / `"傍線"`) for diagnostics that name a
    /// mismatched bouten range pair.
    #[must_use]
    pub const fn family_str(self) -> &'static str {
        if self.is_line() { "傍線" } else { "傍点" }
    }
}

/// Every [`BoutenKind`] variant, in declaration order.
///
/// The single enumeration source the rest of the workspace derives from:
/// the parser's reverse keyword→kind lookup walks this list against
/// [`BoutenKind::keyword`] instead of hand-maintaining a second match,
/// and the render / spec slug tables are drift-checked against it. Adding
/// a bouten mark therefore means a new variant + its `keyword` arm + one
/// row here — nothing else can silently fall out of sync.
pub const BOUTEN_KINDS: &[BoutenKind] = &[
    BoutenKind::Goma,
    BoutenKind::WhiteSesame,
    BoutenKind::Circle,
    BoutenKind::WhiteCircle,
    BoutenKind::DoubleCircle,
    BoutenKind::Janome,
    BoutenKind::Cross,
    BoutenKind::WhiteTriangle,
    BoutenKind::WavyLine,
    BoutenKind::UnderLine,
    BoutenKind::DoubleUnderLine,
    BoutenKind::ChainLine,
    BoutenKind::DashedLine,
    BoutenKind::BlackTriangle,
];

/// Which side of the vertical-writing base text the bouten marks sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenPosition {
    /// Right of (in horizontal terms, above) the base text — the
    /// default side, the bare `［＃「X」に傍点］` form.
    #[default]
    Right,
    /// Left of (below) the base text — the `左に` modifier
    /// (`［＃「X」の左に傍点］`).
    Left,
}

/// Single-line indentation marker (`［＃N字下げ］`).
///
/// The one-line counterpart of the [`ContainerKind::Indent`] block range;
/// indents only the line it sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Indent {
    /// Number of full-width characters to indent by.
    pub amount: u8,
}

/// Single-line end-alignment marker (`［＃地付き］` / `［＃地から N字上げ］`).
///
/// Pushes the line to the foot (地, the bottom of the column / page) — the
/// one-line counterpart of the [`ContainerKind::AlignEnd`] block range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AlignEnd {
    /// Offset in chars from the right edge. `0` = 地付き, `n` = 地から n 字上げ.
    pub offset: u8,
}

/// Single-line centring marker (`［＃ページの左右中央］` / `［＃中央揃え］`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Center {
    /// `true` for `ページの左右中央` (page centre), `false` for `中央揃え`.
    pub page: bool,
}

/// Which side of the base text a ruby reading sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RubySide {
    /// Standard ruby — `｜base《reading》` (right of / above the base).
    #[default]
    Right,
    /// Left-side (below) ruby — `［＃「base」の左に「reading」のルビ］`, the
    /// saidoku-moji (再読文字) building block.
    Left,
}

/// Which annotation flavour a [`crate::borrowed::MarginNote`] carries.
///
/// 注記 and 傍記 share the `MarginNote` structure (a note attached to a
/// preceding run) but round-trip to distinct keywords, so the flavour is
/// preserved here even though both render the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MarginNoteKind {
    /// 注記 — `［＃「X」の左に「Y」の注記］`, a left-side editorial gloss.
    #[default]
    Gloss,
    /// 傍記 — `［＃「X」に「Y」の傍記］`, a redaction marker (典型的に ×)
    /// written beside X, used in censorship restoration.
    Marginal,
}

impl MarginNoteKind {
    /// The `(connector, suffix)` source literals that wrap the note text
    /// when a [`crate::borrowed::MarginNote`] of this flavour round-trips
    /// back to source as `base［＃「base{connector}note{suffix}`.
    ///
    /// Renderers call this instead of matching the (`non_exhaustive`)
    /// variants, so a future flavour must add its affixes here — keeping
    /// the round-trip vocabulary beside the variant definition.
    #[must_use]
    pub const fn serialize_affixes(self) -> (&'static str, &'static str) {
        match self {
            // 注記 normalises bare `に` input to the canonical `の左に…の注記`.
            Self::Gloss => ("」の左に「", "」の注記］"),
            // 傍記 keeps the bare `に` — there is no 左 in the source.
            Self::Marginal => ("」に「", "」の傍記］"),
        }
    }
}

/// Single-line 罫囲み (ruled box) marker (`［＃罫囲み］`).
///
/// A fieldless tag: it boxes the line it sits on. The multi-line range
/// form is the [`ContainerKind::Framed`] paired container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Framed;

/// Which section-break directive a [`borrowed::Node::SectionBreak`] carries —
/// the stronger page-structure breaks beyond the plain `［＃改ページ］`.
///
/// Each variant maps to its canonical keyword via [`Self::keyword`];
/// [`SECTION_KINDS`] is the declaration-order list the renderer derives its
/// class list from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SectionKind {
    /// `［＃改丁］`
    Kaicho,
    /// `［＃改段］`
    Kaidan,
    /// `［＃改見開き］`
    Kaimihiraki,
}

/// Heading *level* — the 大 / 中 / 小 outline rank.
///
/// Orthogonal to [`HeadingStyle`]; the two combine (同行中見出し is
/// `Medium` + `SameLine`, 窓小見出し is `Small` + `Window`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingKind {
    /// 大見出し — the top outline level (renders as `<h1>`).
    Large,
    /// 中見出し — the middle outline level (renders as `<h2>`).
    Medium,
    /// 小見出し — the lowest outline level (renders as `<h3>`).
    Small,
}

/// Heading *style* — standard, 同行 (same-line), or 窓 (window).
///
/// Orthogonal to [`HeadingKind`] (the 大 / 中 / 小 level): each style
/// pairs with any level. The 同行 style runs the title into the body on the
/// same line; 窓 is an inset title. 副見出し is **not** a real annotation (it
/// does not occur in the corpus) and is deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum HeadingStyle {
    /// Standard heading — no 同行 / 窓 prefix. The default.
    #[default]
    Standard,
    /// 同行見出し — the title runs into the body on the same line.
    SameLine,
    /// 窓見出し — an inset ("window") title.
    Window,
}

/// Inline typographic treatment carried by the forward-reference leaf
/// node [`borrowed::Emphasis`] (`X［＃「X」は…］`).
///
/// Covers text-weight / slant (太字 / 斜体), super- and sub-script
/// (上付き小文字 / 下付き小文字), and the vertical-writing small side
/// glyphs (行右小書き / 行左小書き). Distinct from [`BoutenKind`]
/// (傍点 / 傍線 decorative marks): emphasis is a typographic treatment of
/// a whole span, not a per-character mark. The weight range / block forms
/// (`［＃太字］…［＃太字終わり］`, `［＃ここから太字］…`) pair as
/// [`ContainerKind::Bold`] / [`ContainerKind::Italic`]; the script and
/// 小書き forms are forward-reference only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EmphasisKind {
    /// 太字 (bold / ゴシック).
    Bold,
    /// 斜体 (italic / イタリック).
    Italic,
    /// 上付き小文字 (superscript — exponents, ordinals; 横組み).
    SuperScript,
    /// 下付き小文字 (subscript — chemical formulae; 横組み).
    SubScript,
    /// 行右小書き (small glyph set to the line's right in vertical writing).
    SmallRight,
    /// 行左小書き (small glyph set to the line's left in vertical writing).
    SmallLeft,
    /// 文字サイズ変更 (`●段階大きな/小さな文字`) — a relative size shift of
    /// `steps` stages: positive enlarges (大きな), negative shrinks (小さな).
    /// Never zero.
    FontSize {
        /// Signed stage count; `+N` = `N段階大きな文字`, `-N` = `N段階小さな文字`.
        steps: i8,
    },
    /// 行中 罫囲み (`「X」は罫囲み`) — the inline forward-reference box, the
    /// span-level counterpart of the block [`ContainerKind::Framed`]
    /// (just as [`Bold`](Self::Bold) is the leaf counterpart of
    /// [`ContainerKind::Bold`]).
    KeigakomiInline,
    /// 横組み (`「X」は横組み`) — an inline run set horizontally inside vertical
    /// text, the span-level counterpart of the block
    /// [`ContainerKind::Horizontal`].
    HorizontalInline,
    /// キャプション (`「X」はキャプション`) — the inline forward-reference
    /// caption, the leaf counterpart of the block / range
    /// [`ContainerKind::Caption`].
    Caption,
}

/// Every [`SectionKind`] variant in declaration order.
///
/// Drives the renderer's class-list derivation (and any codegen) so a new
/// section break flows in without a hand-maintained parallel — mirrors
/// [`BOUTEN_KINDS`].
pub const SECTION_KINDS: &[SectionKind] = &[
    SectionKind::Kaicho,
    SectionKind::Kaidan,
    SectionKind::Kaimihiraki,
];

/// Every [`HeadingKind`] outline level in declaration order. See
/// [`BOUTEN_KINDS`].
pub const HEADING_KINDS: &[HeadingKind] =
    &[HeadingKind::Large, HeadingKind::Medium, HeadingKind::Small];

/// Every [`HeadingStyle`] in declaration order. See [`BOUTEN_KINDS`].
pub const HEADING_STYLES: &[HeadingStyle] = &[
    HeadingStyle::Standard,
    HeadingStyle::SameLine,
    HeadingStyle::Window,
];

/// Every [`EmphasisKind`] variant in declaration order.
///
/// `FontSize` appears once with a representative magnitude — the
/// renderer's larger/smaller split is driven by the sign, exercised
/// explicitly where it matters. See [`BOUTEN_KINDS`].
pub const EMPHASIS_KINDS: &[EmphasisKind] = &[
    EmphasisKind::Bold,
    EmphasisKind::Italic,
    EmphasisKind::SuperScript,
    EmphasisKind::SubScript,
    EmphasisKind::SmallRight,
    EmphasisKind::SmallLeft,
    EmphasisKind::KeigakomiInline,
    EmphasisKind::HorizontalInline,
    EmphasisKind::Caption,
    EmphasisKind::FontSize { steps: 1 },
];

// --- enum → canonical 青空文庫 keyword ---------------------------------------
//
// The single source of truth for the Japanese keyword each render-bearing
// enum maps to (e.g. `BoutenKind::WhiteSesame` → "白ゴマ傍点"). Both the
// serializer (AST → annotation text) and the renderers key on these, and
// `aozora_spec::roman_slug` turns the keyword into the romaji CSS slug — so
// the keyword lives here once instead of being copied per crate.

impl BoutenKind {
    /// Canonical 青空文庫 keyword (the body of `［＃「…」に〈keyword〉］`).
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::WhiteSesame => "白ゴマ傍点",
            Self::Circle => "丸傍点",
            Self::WhiteCircle => "白丸傍点",
            Self::DoubleCircle => "二重丸傍点",
            Self::Janome => "蛇の目傍点",
            Self::Cross => "ばつ傍点",
            Self::WhiteTriangle => "白三角傍点",
            Self::WavyLine => "波線",
            Self::UnderLine => "傍線",
            Self::DoubleUnderLine => "二重傍線",
            Self::ChainLine => "鎖線",
            Self::DashedLine => "破線",
            Self::BlackTriangle => "黒三角傍点",
            // Goma (無印) and any future kind default to the bare 傍点.
            _ => "傍点",
        }
    }
}

impl EmphasisKind {
    /// Canonical 青空文庫 keyword for the emphasis treatment. `FontSize`
    /// is serialized separately (it carries a magnitude) and falls through
    /// to the 太字 default here.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Italic => "斜体",
            Self::SuperScript => "上付き小文字",
            Self::SubScript => "下付き小文字",
            Self::SmallRight => "行右小書き",
            Self::SmallLeft => "行左小書き",
            Self::KeigakomiInline => "罫囲み",
            Self::HorizontalInline => "横組み",
            Self::Caption => "キャプション",
            // Bold, FontSize, and any future weight default to 太字.
            _ => "太字",
        }
    }
}

impl SectionKind {
    /// Canonical 青空文庫 keyword for the section break. Matched
    /// exhaustively: adding a variant is a compile error here until its
    /// keyword is supplied, rather than silently falling through a
    /// `#[non_exhaustive]` `_` arm.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Kaicho => "改丁",
            Self::Kaidan => "改段",
            Self::Kaimihiraki => "改見開き",
        }
    }
}

/// Classifies a generic [`borrowed::Directive`] annotation that no more
/// specific node recogniser claimed.
///
/// [`Unknown`](Self::Unknown) is the catch-all for Aozora-shaped `［＃…］`
/// notation the parser does not model; the remaining variants tag the
/// handful of annotations kept as raw `Directive`s (sic markers, warichu
/// delimiters, the header 凡例 `［＃］`, …) so consumers can act on them
/// without re-parsing the raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DirectiveKind {
    /// The parser recognised the notation as Aozora-shaped but not registered.
    Unknown,
    /// `［＃「」」はママ］`-style editorial *sic* marker (text reproduced as in the source).
    Sic,
    /// Source-text divergence note (`［＃「X」は底本では「Y」］`).
    BaseTextVariant,
    /// A ruby span that couldn't be parsed cleanly.
    InvalidRubySpan,
    /// Inline warichu opener — `［＃割り注］`.
    WarichuOpen,
    /// Inline warichu closer — `［＃割り注終わり］`.
    WarichuClose,
    /// An empty directive `［＃］` (or whitespace-only `［＃　］`). Not an
    /// unrecognised notation: it is the de-facto-standard symbol used in the
    /// file-header 凡例 line `［＃］：入力者注…` that prefixes essentially every
    /// 青空文庫 work. Typed distinctly so it leaves the `Unknown` bucket while
    /// still round-tripping its raw bytes.
    Empty,
}

/// Parse- and render-time error surface for `aozora-syntax` consumers.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum SyntaxError {
    /// A node-kind tag string did not resolve to a known node kind. The
    /// offending tag is carried verbatim in [`kind`](Self::UnknownKind::kind)
    /// and echoed in the `未知のノード種別です` message; the diagnostic code is
    /// `aozora::syntax::unknown_kind`.
    #[error("未知のノード種別です: {kind}")]
    #[diagnostic(code(aozora::syntax::unknown_kind))]
    UnknownKind {
        /// The unrecognised tag string, as received.
        kind: Box<str>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bouten_position_defaults_to_right() {
        assert_eq!(BoutenPosition::default(), BoutenPosition::Right);
    }

    #[test]
    fn ruby_side_and_heading_style_defaults() {
        assert_eq!(RubySide::default(), RubySide::Right);
        assert_eq!(HeadingStyle::default(), HeadingStyle::Standard);
    }

    #[test]
    fn bouten_keyword_is_exhaustive_and_stable() {
        // Every named variant keys its canonical 青空文庫 keyword; the
        // bare 傍点 (Goma) flows through the `_` default arm.
        let cases = [
            (BoutenKind::Goma, "傍点"),
            (BoutenKind::WhiteSesame, "白ゴマ傍点"),
            (BoutenKind::Circle, "丸傍点"),
            (BoutenKind::WhiteCircle, "白丸傍点"),
            (BoutenKind::DoubleCircle, "二重丸傍点"),
            (BoutenKind::Janome, "蛇の目傍点"),
            (BoutenKind::Cross, "ばつ傍点"),
            (BoutenKind::WhiteTriangle, "白三角傍点"),
            (BoutenKind::WavyLine, "波線"),
            (BoutenKind::UnderLine, "傍線"),
            (BoutenKind::DoubleUnderLine, "二重傍線"),
            (BoutenKind::ChainLine, "鎖線"),
            (BoutenKind::DashedLine, "破線"),
            (BoutenKind::BlackTriangle, "黒三角傍点"),
        ];
        for (kind, kw) in cases {
            assert_eq!(kind.keyword(), kw, "keyword mismatch for {kind:?}");
        }
    }

    #[test]
    fn bouten_is_line_splits_dot_from_line_family() {
        // 線 family.
        for kind in [
            BoutenKind::WavyLine,
            BoutenKind::UnderLine,
            BoutenKind::DoubleUnderLine,
            BoutenKind::ChainLine,
            BoutenKind::DashedLine,
        ] {
            assert!(kind.is_line(), "{kind:?} should be a 線 variant");
            assert_eq!(kind.family_str(), "傍線", "family_str for {kind:?}");
        }
        // 点 family.
        for kind in [
            BoutenKind::Goma,
            BoutenKind::WhiteSesame,
            BoutenKind::Circle,
            BoutenKind::WhiteCircle,
            BoutenKind::DoubleCircle,
            BoutenKind::Janome,
            BoutenKind::Cross,
            BoutenKind::WhiteTriangle,
            BoutenKind::BlackTriangle,
        ] {
            assert!(!kind.is_line(), "{kind:?} should be a 点 variant");
            assert_eq!(kind.family_str(), "傍点", "family_str for {kind:?}");
        }
    }

    #[test]
    fn emphasis_keyword_is_exhaustive_and_stable() {
        let cases = [
            (EmphasisKind::Bold, "太字"),
            (EmphasisKind::Italic, "斜体"),
            (EmphasisKind::SuperScript, "上付き小文字"),
            (EmphasisKind::SubScript, "下付き小文字"),
            (EmphasisKind::SmallRight, "行右小書き"),
            (EmphasisKind::SmallLeft, "行左小書き"),
            (EmphasisKind::KeigakomiInline, "罫囲み"),
            (EmphasisKind::HorizontalInline, "横組み"),
            (EmphasisKind::Caption, "キャプション"),
            // FontSize carries a magnitude and falls through to 太字.
            (EmphasisKind::FontSize { steps: 3 }, "太字"),
            (EmphasisKind::FontSize { steps: -2 }, "太字"),
        ];
        for (kind, kw) in cases {
            assert_eq!(kind.keyword(), kw, "keyword mismatch for {kind:?}");
        }
    }

    #[test]
    fn section_keyword_is_exhaustive() {
        assert_eq!(SectionKind::Kaicho.keyword(), "改丁");
        assert_eq!(SectionKind::Kaidan.keyword(), "改段");
        assert_eq!(SectionKind::Kaimihiraki.keyword(), "改見開き");
    }

    #[test]
    fn heading_kind_and_style_are_orthogonal_copies() {
        // Cheap structural smoke: every level pairs with every style and
        // the payload structs are Copy + Eq as the AST relies on.
        let heading = |kind, style| Container {
            kind: ContainerKind::Heading {
                kind,
                style,
                block: true,
            },
        };
        assert_eq!(
            heading(HeadingKind::Medium, HeadingStyle::Window),
            heading(HeadingKind::Medium, HeadingStyle::Window),
            "equal Heading containers compare equal"
        );
        assert_ne!(
            heading(HeadingKind::Medium, HeadingStyle::Window),
            heading(HeadingKind::Small, HeadingStyle::Window),
            "different level ⇒ not equal"
        );
        let indent = Indent { amount: 4 };
        assert_eq!(indent, Indent { amount: 4 });
        let align = AlignEnd { offset: 0 };
        assert_eq!(align.offset, 0);
        let center = Center { page: true };
        assert!(center.page, "page-centre flag round-trips");
    }

    #[test]
    fn bouten_kinds_are_complete_and_distinct() {
        assert_eq!(BOUTEN_KINDS.len(), 14, "every BoutenKind variant listed");
        for (i, a) in BOUTEN_KINDS.iter().enumerate() {
            for b in &BOUTEN_KINDS[i + 1..] {
                assert_ne!(a, b, "duplicate variant in BOUTEN_KINDS");
                assert_ne!(a.keyword(), b.keyword(), "duplicate bouten keyword");
            }
        }
    }

    #[test]
    fn every_bouten_kind_has_a_render_slug() {
        // The spec's RENDER_SLUGS must carry a romaji slug for every bouten
        // kind so the renderer's `aozora-bouten-<slug>` class never falls
        // back. Drift-guards the syntax↔spec bouten tables against the
        // single `BOUTEN_KINDS` source.
        for k in BOUTEN_KINDS {
            assert!(
                aozora_spec::roman_slug(k.keyword()).is_some(),
                "RENDER_SLUGS missing a slug for bouten keyword {:?}",
                k.keyword()
            );
        }
    }
}
