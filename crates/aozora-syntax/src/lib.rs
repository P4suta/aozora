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
//! `AozoraHeadingKind`, `AnnotationKind`) live at the top level. The
//! borrowed-AST node types live under `borrowed::`. The arena-backed
//! builder lives under `alloc::`.

#![forbid(unsafe_code)]

use miette::Diagnostic;
use thiserror::Error;

pub mod accent;
pub mod alloc;
pub mod borrowed;
mod extension;

pub use extension::ContainerKind;

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
