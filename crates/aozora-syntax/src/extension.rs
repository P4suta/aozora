//! Paired-container classifier tags.
//!
//! [`ContainerKind`] is the tag the lexer's classify phase emits on
//! every paired open / close marker (e.g. `［＃ここから2字下げ］ … ［＃ここで字下げ終わり］`).
//! The renderer reads it when wrapping the enclosed sibling nodes
//! into an `AozoraNode::Container`.

use crate::{AozoraHeadingKind, AozoraHeadingStyle, BoutenKind, BoutenPosition};

/// The kinds of Aozora container blocks the lexer classifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ContainerKind {
    /// `［＃ここから N字下げ］`, or the hanging-indent form
    /// `［＃ここから N字下げ、折り返して M字下げ］` where `wrap` is `Some(M)`
    /// (the wrapped continuation lines indent by `M`). `wrap` is `None`
    /// for a plain block indent and for the generic `字下げ終わり` closer.
    /// `center` is `true` for the combined `［＃ここから N字下げ、ページの左右
    /// 中央］` form — the indented block is also page-centred; it still closes
    /// with the shared `字下げ終わり`.
    /// `head_flush` is `true` for the 改行天付き hanging form
    /// `［＃ここから改行天付き、折り返して M字下げ］` — the first line is flush
    /// to the head (天付き, no indent: `amount` is `0`) and only the wrapped
    /// continuation lines indent by `wrap` (`M`). It also closes with the
    /// shared `字下げ終わり`.
    Indent {
        amount: u8,
        wrap: Option<u8>,
        center: bool,
        head_flush: bool,
    },
    /// `［＃割り注］ ... ［＃割り注終わり］` (when spanning multiple lines)
    Warichu,
    /// `［＃罫囲み］ ... ［＃罫囲み終わり］`
    Keigakomi,
    /// `［＃ここから地付き］` / `［＃ここから地から N 字上げ］`
    AlignEnd { offset: u8 },
    /// `［＃ここから N字詰め］ ... ［＃ここで字詰め終わり］` — line-width
    /// (字詰め): sets the number of full-width characters per line for the
    /// enclosed run. Block-only (no single-line form), so `is_inline` is
    /// `false` and the normalizer pads it like any other block container.
    /// `width` is the character count from the opener; the close marker
    /// re-emits `width: 0` as a placeholder (the open-side payload is
    /// authoritative when pairing, mirroring the generic `字下げ終わり`
    /// closer). Renders as
    /// `<div class="aozora-container aozora-container-line-width" data-width="N">`.
    LineWidth { width: u8 },
    /// 傍点 / 傍線 range form: `［＃傍点］ ... ［＃傍点終わり］`,
    /// `［＃二重傍線］ ... ［＃二重傍線終わり］`, `［＃左に傍線］ ...`, etc.
    /// The `kind` is the emphasis variant (its 点/線 family drives the
    /// `mismatched_bouten_container` check); `position` records a `左に`
    /// left-side modifier. The renderer wraps the run in
    /// `<em class="aozora-bouten-…">`.
    BoutenRange {
        kind: BoutenKind,
        position: BoutenPosition,
    },
    /// 太字 (bold) range / block. `block` distinguishes the bare inline
    /// range `［＃太字］ ... ［＃太字終わり］` (`false`, no `\n\n` padding,
    /// renders as inline `<b class="aozora-bold">`) from the block form
    /// `［＃ここから太字］ ... ［＃ここで太字終わり］` (`true`, padded,
    /// renders as a block `<div class="aozora-container aozora-container-bold">`
    /// so the wrapped paragraphs nest validly). A separate variant from
    /// [`Self::Italic`] so a 太字-open closed by a 斜体-close trips
    /// `mismatched_container_close` (different discriminant).
    Bold { block: bool },
    /// 斜体 (italic) range / block — the slant counterpart of
    /// [`Self::Bold`]. `［＃斜体］ ... ［＃斜体終わり］` (inline
    /// `<i class="aozora-italic">`) / `［＃ここから斜体］ ...
    /// ［＃ここで斜体終わり］` (block
    /// `<div class="aozora-container aozora-container-italic">`).
    Italic { block: bool },
    /// Delimited heading. The **paired** form `［＃窓中見出し］ ...
    /// ［＃窓中見出し終わり］` (`block: false`) and the **block** form
    /// `［＃ここから大見出し］ ... ［＃ここで大見出し終わり］` (`block: true`)
    /// wrap their enclosed run and render it as a heading — the container
    /// counterpart of the forward-reference [`crate::borrowed::AozoraHeading`]
    /// leaf. `kind` is the 大/中/小 level, `style` the standard / 同行 / 窓
    /// style. Its content is *phrasing* (rendered directly inside the
    /// `<hN>` / `<div>`, not wrapped in a `<p>`) — see [`Self::content_is_phrasing`].
    Heading {
        kind: AozoraHeadingKind,
        style: AozoraHeadingStyle,
        block: bool,
    },
    /// `［＃ここからN段組(み)］ ... ［＃ここで段組(み)終わり］` — a multi-column
    /// (段組) layout region. `count` is the number of columns. A layout
    /// container only: the enclosed content is plain text with no per-column
    /// markup. Renders as
    /// `<div class="aozora-container aozora-container-columns" data-columns="N">`.
    Columns { count: u8 },
    /// `［＃ここから表］ ... ［＃ここで表終わり］` — a table region. A layout
    /// container only: there is no cell / row / column markup, so the enclosed
    /// content is plain text. Renders as
    /// `<div class="aozora-container aozora-container-table">`.
    Table,
    /// `［＃ここから横組み］ ... ［＃ここで横組み終わり］` — a horizontal-writing
    /// (横組み) region inside an otherwise vertical document. A writing-mode
    /// container; the enclosed content is plain text. Renders as
    /// `<div class="aozora-container aozora-container-horizontal">`.
    Horizontal,
    /// `［＃ここからN段階大きな文字］ ... ［＃ここで大きな文字終わり］` (and the
    /// 小さな variant) — a block-level relative font-size shift. `steps` is the
    /// signed stage count (positive = 大きな, negative = 小さな); the close
    /// marker carries only the direction (its magnitude is a `±1` placeholder).
    /// Renders as
    /// `<div class="aozora-container aozora-container-font-larger" data-steps="N">`
    /// for the block form (`block: true`), or the inline
    /// `<span class="aozora-font-larger" data-steps="N">` for the bare range
    /// `［＃N段階大きな文字］ ... ［＃大きな文字終わり］` (`block: false`, no
    /// `ここから` / `ここで`) — the container counterpart of the
    /// forward-reference [`crate::EmphasisKind::FontSize`] leaf.
    FontSize { steps: i8, block: bool },
    /// 行右小書き / 行左小書き range / block — the small-glyph side script set
    /// to the line's right (`position: Right`) or left (`position: Left`) in
    /// vertical writing. The bare range `［＃行右小書き］ ... ［＃行右小書き
    /// 終わり］` (`block: false`, inline `<span class="aozora-koshogaki-right">`)
    /// is the container counterpart of the forward-reference
    /// [`crate::EmphasisKind::SmallRight`] / [`crate::EmphasisKind::SmallLeft`]
    /// leaf, just as [`Self::Bold`] is the counterpart of
    /// [`crate::EmphasisKind::Bold`]. `block` is reserved for a future
    /// `［＃ここから行右小書き］` form (unattested in the corpus today).
    SmallSide {
        position: BoutenPosition,
        block: bool,
    },
}

impl ContainerKind {
    /// Stable lowercase tag naming the container *family*, ignoring the
    /// `amount` / `offset` payload (so `Indent { amount: 2 }` and
    /// `Indent { amount: 0 }` both report `"indent"`).
    ///
    /// Used by human-facing diagnostics — e.g.
    /// [`aozora_spec::Diagnostic::mismatched_container_close`] — that need
    /// to name a mismatched open/close pair. (The `aozora` wire format
    /// keeps its own camelCase mapping for the machine contract.)
    #[must_use]
    pub const fn kind_str(self) -> &'static str {
        match self {
            Self::Indent { .. } => "indent",
            Self::Warichu => "warichu",
            Self::Keigakomi => "keigakomi",
            Self::AlignEnd { .. } => "align-end",
            Self::LineWidth { .. } => "line-width",
            Self::BoutenRange { .. } => "bouten-range",
            Self::Bold { .. } => "bold",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns { .. } => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize { .. } => "font-size",
            Self::SmallSide { .. } => "small-side",
        }
    }

    /// Whether this container's content is *phrasing* (inline) — rendered
    /// directly inside the block element rather than wrapped in `<p>`
    /// paragraphs.
    ///
    /// Only [`Self::Heading`] is phrasing: a heading element (`<hN>` / inset
    /// `<div>`) holds its title text directly, so `<h1><p>…</p></h1>` would be
    /// invalid. Every other block container wraps flow content in paragraphs.
    /// The block walker reads this to suppress paragraph emission inside the
    /// heading while still flushing the surrounding paragraph (the heading is
    /// block-level, just with phrasing content).
    #[must_use]
    pub const fn content_is_phrasing(self) -> bool {
        matches!(self, Self::Heading { .. })
    }

    /// Whether this container renders *inline* (within the current
    /// paragraph) rather than as a block wrapper.
    ///
    /// The 傍点 / 傍線 range (`<em>`) and the bare-range 太字 / 斜体 forms
    /// (`<b>` / `<i>`, `block: false`) are inline — every corpus
    /// occurrence sits within a line. Every other container (字下げ,
    /// 罫囲み, the ここから-block emphasis forms) is block-level: it gets
    /// `\n\n` padding from the normalizer and a `<div>` wrapper from the
    /// renderer. Centralised here so the lexer's padding decision and the
    /// renderer's paragraph decision cannot drift apart.
    #[must_use]
    pub const fn is_inline(self) -> bool {
        matches!(
            self,
            Self::BoutenRange { .. }
                | Self::Bold { block: false }
                | Self::Italic { block: false }
                | Self::SmallSide { block: false, .. }
                | Self::FontSize { block: false, .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_kind_is_copy_and_fits_in_a_word() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<ContainerKind>();
        // u8 + discriminant, must fit in a few bytes so downstream
        // vector entries stay tight.
        // The combined 字下げ＋ページ左右中央 form adds a `center` flag to
        // `Indent`, nudging the widest variant past 4 bytes; 8 keeps the tag
        // register-friendly while leaving room for the payload.
        assert!(size_of::<ContainerKind>() <= 8);
    }

    #[test]
    fn kind_str_ignores_payload_and_covers_every_family() {
        assert_eq!(
            ContainerKind::Indent {
                amount: 2,
                wrap: None,
                center: false,
                head_flush: false,
            }
            .kind_str(),
            "indent"
        );
        assert_eq!(
            ContainerKind::Indent {
                amount: 0,
                wrap: None,
                center: false,
                head_flush: false,
            }
            .kind_str(),
            "indent"
        );
        assert_eq!(
            ContainerKind::Indent {
                amount: 2,
                wrap: Some(4),
                center: false,
                head_flush: false,
            }
            .kind_str(),
            "indent"
        );
        assert_eq!(
            ContainerKind::Indent {
                amount: 0,
                wrap: Some(2),
                center: false,
                head_flush: true,
            }
            .kind_str(),
            "indent"
        );
        assert_eq!(ContainerKind::Warichu.kind_str(), "warichu");
        assert_eq!(ContainerKind::Keigakomi.kind_str(), "keigakomi");
        assert_eq!(
            ContainerKind::AlignEnd { offset: 0 }.kind_str(),
            "align-end"
        );
        assert_eq!(
            ContainerKind::AlignEnd { offset: 3 }.kind_str(),
            "align-end"
        );
        assert_eq!(
            ContainerKind::BoutenRange {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
            .kind_str(),
            "bouten-range"
        );
        assert_eq!(ContainerKind::Bold { block: false }.kind_str(), "bold");
        assert_eq!(ContainerKind::Bold { block: true }.kind_str(), "bold");
        assert_eq!(ContainerKind::Italic { block: false }.kind_str(), "italic");
        assert_eq!(ContainerKind::Italic { block: true }.kind_str(), "italic");
        assert_eq!(
            ContainerKind::SmallSide {
                position: BoutenPosition::Right,
                block: false,
            }
            .kind_str(),
            "small-side"
        );
        assert_eq!(
            ContainerKind::SmallSide {
                position: BoutenPosition::Left,
                block: false,
            }
            .kind_str(),
            "small-side"
        );
    }

    #[test]
    fn small_side_range_is_inline() {
        assert!(
            ContainerKind::SmallSide {
                position: BoutenPosition::Right,
                block: false,
            }
            .is_inline()
        );
    }
}
