//! Paired-container classifier tags.
//!
//! [`ContainerKind`] is the tag the lexer's classify phase emits on
//! every paired open / close marker (e.g. `［＃ここから2字下げ］ … ［＃ここで字下げ終わり］`).
//! The renderer reads it when wrapping the enclosed sibling nodes
//! into an `AozoraNode::Container`.

use crate::{BoutenKind, BoutenPosition};

/// The kinds of Aozora container blocks the lexer classifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ContainerKind {
    /// `［＃ここから N字下げ］`, or the hanging-indent form
    /// `［＃ここから N字下げ、折り返して M字下げ］` where `wrap` is `Some(M)`
    /// (the wrapped continuation lines indent by `M`). `wrap` is `None`
    /// for a plain block indent and for the generic `字下げ終わり` closer.
    Indent { amount: u8, wrap: Option<u8> },
    /// `［＃割り注］ ... ［＃割り注終わり］` (when spanning multiple lines)
    Warichu,
    /// `［＃罫囲み］ ... ［＃罫囲み終わり］`
    Keigakomi,
    /// `［＃ここから地付き］` / `［＃ここから地から N 字上げ］`
    AlignEnd { offset: u8 },
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
            Self::BoutenRange { .. } => "bouten-range",
            Self::Bold { .. } => "bold",
            Self::Italic { .. } => "italic",
        }
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
            Self::BoutenRange { .. } | Self::Bold { block: false } | Self::Italic { block: false }
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
        assert!(size_of::<ContainerKind>() <= 4);
    }

    #[test]
    fn kind_str_ignores_payload_and_covers_every_family() {
        assert_eq!(
            ContainerKind::Indent {
                amount: 2,
                wrap: None
            }
            .kind_str(),
            "indent"
        );
        assert_eq!(
            ContainerKind::Indent {
                amount: 0,
                wrap: None
            }
            .kind_str(),
            "indent"
        );
        assert_eq!(
            ContainerKind::Indent {
                amount: 2,
                wrap: Some(4)
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
    }
}
