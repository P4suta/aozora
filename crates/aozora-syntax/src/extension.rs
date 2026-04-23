//! Extension contract between the forked comrak parser and afm-parser.
//!
//! See ADR-0003 for the architectural rationale. In summary: comrak owns all parser
//! and container-stack state; the afm extension trait supplies three pure functions
//! that comrak calls during its normal inline / block / render passes.
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
//! Implementations must be `Send + Sync + RefUnwindSafe`: comrak may run parsing
//! under `catch_unwind` and share the extension across threads via `Arc`.

use core::num::NonZeroUsize;
use std::panic::RefUnwindSafe;

use crate::AozoraNode;

/// Context passed to [`AozoraExtension::try_parse_inline`].
///
/// Carries a read-only view of the inline cursor. The extension examines
/// `input[pos..]`, optionally consulting `preceding` for implicit-delimiter ruby,
/// and returns `Some(InlineMatch)` to claim bytes or `None` to let comrak continue
/// its default character dispatch.
///
/// `Copy` so callers can freely pass it by value; no allocation occurs at the seam.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct InlineCtx<'a> {
    /// Full inline source available to the current dispatch. The extension must not
    /// look outside this slice.
    pub input: &'a str,

    /// Current byte offset into `input`. `input[pos..]` is the head of the stream
    /// the extension should classify.
    pub pos: usize,

    /// Text the parser has already committed on the current inline run (used by
    /// implicit-delimiter ruby to recover the kanji base). Empty at the start of
    /// the stream.
    pub preceding: &'a str,
}

impl<'a> InlineCtx<'a> {
    /// Constructor for out-of-crate callers (comrak fork) since the struct is
    /// `#[non_exhaustive]` to allow additive fields without a breaking change.
    #[must_use]
    pub const fn new(input: &'a str, pos: usize, preceding: &'a str) -> Self {
        Self {
            input,
            pos,
            preceding,
        }
    }
}

/// Result of a successful inline match.
///
/// `consumed` is `NonZeroUsize` so the type system enforces that a match actually
/// advanced the cursor; returning zero would put the parser in an infinite loop.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InlineMatch {
    pub node: AozoraNode,
    pub consumed: NonZeroUsize,
}

impl InlineMatch {
    /// Construct an `InlineMatch` from a node and a byte-count that must be
    /// positive. Returns `None` if `consumed == 0`, which the type system then
    /// ensures the caller handles by returning `None` from the hook — fatal
    /// infinite-loop hazards can never be committed.
    #[must_use]
    pub fn new(node: AozoraNode, consumed: usize) -> Option<Self> {
        NonZeroUsize::new(consumed).map(|consumed| Self { node, consumed })
    }
}

/// Context passed to [`AozoraExtension::try_start_block`].
///
/// `line` is the logical line content with any comrak-level indent already stripped
/// (mirroring the framing used by comrak's other block starters).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct BlockCtx<'a> {
    /// Current line content with comrak's block-level indent stripped.
    pub line: &'a str,

    /// Byte offset of `line[0]` in the source buffer, for diagnostic spans.
    pub source_offset: u32,
}

impl<'a> BlockCtx<'a> {
    /// Constructor for out-of-crate callers (comrak fork).
    #[must_use]
    pub const fn new(line: &'a str, source_offset: u32) -> Self {
        Self {
            line,
            source_offset,
        }
    }
}

/// Classification result for a line the extension inspected.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BlockMatch {
    /// The line does not start an Aozora block; comrak should keep trying its
    /// default starters (thematic break, heading, list, …).
    NotOurs,

    /// A leaf block (e.g. `［＃改ページ］`, `※［＃…］`). Comrak emits the node and
    /// moves on to the next line.
    Leaf(AozoraNode),

    /// Open a container block (e.g. `［＃ここから字下げ］`). Comrak pushes the
    /// corresponding frame onto its container stack; subsequent lines are parsed as
    /// children until a matching [`BlockMatch::CloseContainer`].
    OpenContainer(ContainerKind),

    /// Close the topmost currently-open Aozora container. Paired with a prior
    /// `OpenContainer`.
    CloseContainer,
}

/// The kinds of Aozora container blocks comrak tracks on its stack.
///
/// Deliberately carries only the metadata comrak needs for matching — the full AST
/// node is constructed by afm-parser at close time. Keeping this small keeps the
/// stack entry size tight (each frame in `SmallVec<[AozoraOpen; 4]>` stays under a
/// cache line).
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

/// Hook surface registered with comrak via
/// `comrak::ExtensionOptions::aozora: Option<Arc<dyn AozoraExtension + 'c>>`.
///
/// All three methods take `&self`; state that must change during parsing should be
/// held behind `Mutex` or `AtomicXxx`. Implementations must uphold the bounds
/// listed at trait level — comrak may run the parser under `catch_unwind` and may
/// share the extension across threads.
///
/// `try_*` methods are **pure classifiers**: they report what comrak should do
/// next, and never mutate comrak's parse state directly. This separation is what
/// keeps the upstream diff at a fixed, minimal set of dispatch calls.
pub trait AozoraExtension: Send + Sync + RefUnwindSafe {
    /// Called during inline scanning whenever comrak's character dispatch reaches a
    /// position the extension cares about (currently `｜` / `《`).
    ///
    /// Return `Some(InlineMatch)` to claim bytes, or `None` to let comrak continue
    /// with its default dispatch. The extension must not advance past the end of
    /// `cx.input`.
    fn try_parse_inline(&self, cx: InlineCtx<'_>) -> Option<InlineMatch>;

    /// Called at each block-start position with the current line's content.
    fn try_start_block(&self, cx: BlockCtx<'_>) -> BlockMatch;

    /// Render a recognised [`AozoraNode`] to HTML. Called from comrak's renderer at
    /// the one `NodeValue::Aozora(_)` arm. The extension emits well-formed,
    /// correctly-escaped HTML; it must not write raw text that would break the
    /// surrounding structure.
    ///
    /// Uses [`core::fmt::Write`] (rather than `std::io::Write`) to match comrak's
    /// formatter-based output pipeline, avoiding an extra UTF-8 bridge.
    ///
    /// # Errors
    ///
    /// Propagates formatter write errors.
    fn render_html(
        &self,
        node: &AozoraNode,
        writer: &mut dyn core::fmt::Write,
    ) -> core::fmt::Result;
}

impl std::fmt::Debug for dyn AozoraExtension + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<dyn AozoraExtension>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoutenKind, Ruby};

    #[test]
    fn inline_match_rejects_zero_consumed() {
        let node = AozoraNode::PageBreak;
        assert!(InlineMatch::new(node, 0).is_none());
    }

    #[test]
    fn inline_match_accepts_positive_consumed() {
        let node = AozoraNode::Ruby(Ruby {
            base: "X".into(),
            reading: "y".into(),
            delim_explicit: false,
        });
        let m = InlineMatch::new(node, 7).expect("nonzero");
        assert_eq!(m.consumed.get(), 7);
    }

    #[test]
    fn inline_ctx_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<InlineCtx<'_>>();
    }

    #[test]
    fn block_ctx_is_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<BlockCtx<'_>>();
    }

    #[test]
    fn container_kind_is_copy_and_fits_in_a_word() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<ContainerKind>();
        // u8 + discriminant, must fit in a few bytes so SmallVec entries stay tight.
        assert!(size_of::<ContainerKind>() <= 4);
    }

    #[test]
    fn block_match_is_non_exhaustive() {
        // Smoke: downstream match on BlockMatch must include `_`; this compiles
        // because we're in-crate and can omit the wildcard.
        let m = BlockMatch::Leaf(AozoraNode::Bouten(crate::Bouten {
            kind: BoutenKind::Goma,
            target: "".into(),
        }));
        let matched = matches!(m, BlockMatch::Leaf(_));
        assert!(matched);
    }
}
