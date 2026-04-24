//! Extension-seam type that the forked comrak parser still needs.
//!
//! The only afm seam inside comrak is a render-side `fn` pointer on
//! `comrak::ExtensionOptions::render_aozora`. There is no trait object,
//! no extension trait, and no parse-side hook. The `AozoraNode` variant
//! on `NodeValue::Aozora` is carried by the lexer and
//! `afm-parser::post_process` splice step, and the render callback
//! simply writes an `AozoraNode` into a formatter.
//!
//! This module carries [`ContainerKind`] — the paired-container
//! classifier's tag — produced by the lexer's Phase 3 classification
//! and consumed by `afm-parser::post_process`'s paired-container
//! splice.

/// The kinds of Aozora container blocks the lexer classifies.
///
/// Carried on `afm-lexer::phase3_classify::SpanKind::{BlockOpen,
/// BlockClose}` and on `afm-lexer::PlaceholderRegistry`'s paired-container
/// entries. `afm-parser::post_process` reads these when wrapping
/// sibling blocks into an `AozoraNode::Container` node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ContainerKind {
    /// `［＃ここから N字下げ］`
    Indent { amount: u8 },
    /// `［＃割り注］ ... ［＃割り注終わり］` (when spanning multiple lines)
    Warichu,
    /// `［＃罫囲み］ ... ［＃罫囲み終わり］`
    Keigakomi,
    /// `［＃ここから地付き］` / `［＃ここから地から N 字上げ］`
    AlignEnd { offset: u8 },
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
}
