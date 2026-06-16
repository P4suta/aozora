//! AST type definitions for the aozora parser.
//!
//! # AST shape
//!
//! The **sole AST** is the borrowed-AST defined in [`borrowed`]:
//! arena-allocated, `Copy`-able, deduplicated through
//! [`borrowed::Interner`]. Public consumers (`aozora` meta crate,
//! FFI / WASM / Python drivers, CLI) parse via
//! `aozora::Document::parse()` and walk a `borrowed::AozoraNode<'_>`.
//!
//! # Top-level surface
//!
//! Only the **shared `Copy`-able payloads** referenced by the borrowed
//! AST (`BoutenKind`, `BoutenPosition`, `Indent`, `AlignEnd`,
//! `Container`, `ContainerKind`, `Keigakomi`, `SectionKind`,
//! `AozoraHeadingKind`, `EmphasisKind`, `AnnotationKind`) live at the
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

pub use extension::ContainerKind;
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
    pub kind: ContainerKind,
}

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

/// Which side of the vertical-writing base text the bouten marks sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenPosition {
    #[default]
    Right,
    Left,
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

/// Single-line centring marker (`［＃ページの左右中央］` / `［＃中央揃え］`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Center {
    /// `true` for `ページの左右中央` (page centre), `false` for `中央揃え`.
    pub page: bool,
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

/// Heading *level* — the 大 / 中 / 小 outline rank.
///
/// Orthogonal to [`AozoraHeadingStyle`]; the two combine (同行中見出し is
/// `Medium` + `SameLine`, 窓小見出し is `Small` + `Window`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraHeadingKind {
    /// 大見出し — the top outline level (renders as `<h1>`).
    Large,
    /// 中見出し — the middle outline level (renders as `<h2>`).
    Medium,
    /// 小見出し — the lowest outline level (renders as `<h3>`).
    Small,
}

/// Heading *style* — standard, 同行 (same-line), or 窓 (window).
///
/// Orthogonal to [`AozoraHeadingKind`] (the 大 / 中 / 小 level): each style
/// pairs with any level. The 同行 style runs the title into the body on the
/// same line; 窓 is an inset title. 副見出し is **not** a real annotation (it
/// does not occur in the corpus) and is deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraHeadingStyle {
    /// Standard heading — no 同行 / 窓 prefix. The default.
    #[default]
    Standard,
    /// 同行見出し — the title runs into the body on the same line.
    SameLine,
    /// 窓見出し — an inset ("window") title.
    Window,
}

/// Text-weight / slant emphasis: 太字 (bold) or 斜体 (italic).
///
/// Distinct from [`BoutenKind`] (傍点 / 傍線 decorative marks): emphasis
/// is a typographic weight/slant, not a per-character mark. Carried by
/// the forward-reference leaf node [`borrowed::Emphasis`]
/// (`X［＃「X」は太字］`); the range / block forms
/// (`［＃太字］…［＃太字終わり］`, `［＃ここから太字］…`) pair as
/// [`ContainerKind::Bold`] / [`ContainerKind::Italic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EmphasisKind {
    /// 太字 (bold / ゴシック).
    Bold,
    /// 斜体 (italic / イタリック).
    Italic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AnnotationKind {
    /// The parser recognised the notation as Aozora-shaped but not registered.
    Unknown,
    /// `［＃「」」はママ］`-style editorial as-is marker.
    AsIs,
    /// Source-text divergence note (`［＃「X」は底本では「Y」］`).
    TextualNote,
    /// A ruby span that couldn't be parsed cleanly.
    InvalidRubySpan,
    /// Inline warichu opener — `［＃割り注］`.
    WarichuOpen,
    /// Inline warichu closer — `［＃割り注終わり］`.
    WarichuClose,
}

/// Parse- and render-time error surface for `aozora-syntax` consumers.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum SyntaxError {
    #[error("未知のノード種別です: {kind}")]
    #[diagnostic(code(aozora::syntax::unknown_kind))]
    UnknownKind { kind: Box<str> },
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
}
