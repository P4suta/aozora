//! Render-side extension contract between the forked comrak parser and afm-parser.
//!
//! See ADR-0008 for the architectural rationale. Since the ADR-0008 cutover
//! (E1), Aozora **parsing** happens entirely in `afm-lexer` + `afm-parser`'s
//! post-process AST walk — comrak sees only normalized CommonMark text with
//! PUA sentinels. The only hook that survives on comrak's side is the
//! **render** dispatch: comrak's HTML renderer encounters
//! `NodeValue::Aozora(_)` arms and needs a way to delegate rendering.
//!
//! The trait lives in `afm-syntax` (beside [`AozoraNode`]) rather than inside
//! comrak or afm-parser so that both can depend on it without creating a
//! dependency cycle:
//!
//! ```text
//!   comrak (fork) ──► afm-syntax ◄── afm-parser ──► comrak
//!         (trait + NodeValue arm)          (impl)
//! ```
//!
//! Implementations must be `Send + Sync + RefUnwindSafe`: comrak may run
//! rendering under `catch_unwind` and share the extension across threads
//! via `Arc`.
//!
//! A future commit (D2) converts this trait-object dispatch to a naked
//! `fn` pointer on `comrak::Options`; the trait is kept for one more
//! milestone so the upstream diff shrinks in two reviewable steps.
//!
//! [`ContainerKind`] is carried here (not the `AozoraNode` module) because
//! the lexer's Phase 3 classification and Phase 4 normalization both need
//! it, and `afm-syntax` is the common dependency.

use core::fmt;
use std::panic::RefUnwindSafe;

use crate::AozoraNode;

/// The kinds of Aozora container blocks the lexer classifies.
///
/// Carried on `afm-lexer::phase3_classify::SpanKind::{BlockOpen,
/// BlockClose}` and on `afm-lexer::PlaceholderRegistry`'s paired-container
/// entries. `afm-parser::post_process` reads these when splicing container
/// nodes back into the AST (F5 schema extension pending).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Render-side hook surface registered with comrak via
/// `comrak::ExtensionOptions::aozora: Option<Arc<dyn AozoraExtension + 'c>>`.
///
/// `render_html` takes `&self`; any state that must change during rendering
/// should be held behind `Mutex` or `AtomicXxx`. Implementations must uphold
/// the bounds listed at trait level — comrak may run the renderer under
/// `catch_unwind` and may share the extension across threads.
pub trait AozoraExtension: Send + Sync + RefUnwindSafe {
    /// Render a recognised [`AozoraNode`] to HTML. Called from comrak's
    /// renderer at the one `NodeValue::Aozora(_)` arm. The extension emits
    /// well-formed, correctly-escaped HTML; it must not write raw text that
    /// would break the surrounding structure.
    ///
    /// Uses [`core::fmt::Write`] (rather than `std::io::Write`) to match
    /// comrak's formatter-based output pipeline, avoiding an extra UTF-8
    /// bridge.
    ///
    /// # Errors
    ///
    /// Propagates formatter write errors.
    fn render_html(&self, node: &AozoraNode, writer: &mut dyn fmt::Write) -> fmt::Result;
}

impl fmt::Debug for dyn AozoraExtension + '_ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<dyn AozoraExtension>")
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
}
