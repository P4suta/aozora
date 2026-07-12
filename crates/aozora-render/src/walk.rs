//! Shared PUA-sentinel walk over an [`LexOutput`]'s normalized text.
//!
//! Both renderers — [`crate::html`] and `serialize`
//! — sweep the same normalized text, dispatching each PUA sentinel through the
//! owned registry and bulk-copying the plain runs between hits. The scan,
//! sentinel decoding, registry lookup, and cursor bookkeeping are identical;
//! only what happens *at* each event differs. This module owns that single scan
//! and drives a [`WalkSink`] for the renderer-specific work.
//!
//! # Why a byte scan
//!
//! Every PUA sentinel (`U+E001..U+E004`) shares the 2-byte UTF-8 prefix
//! `0xEE 0x80`; the third byte (`0x81..0x84`) names the kind. A single
//! `memchr` finds candidate lead bytes at memory-bandwidth speed (SIMD
//! via the `memchr` crate); each candidate is validated with two byte
//! loads before it is treated as a sentinel, so a PUA collision in the
//! source (the sanitize stage records a diagnostic but does not delete the bytes)
//! flows through as plain text via the cursor advance.
//!
//! # Newlines
//!
//! The HTML renderer also reacts to `\n` (paragraph / `<br />` control);
//! the serializer copies newlines verbatim as plain text. A sink opts in
//! with [`WalkSink::WANTS_NEWLINES`], which selects `memchr2(0xEE, '\n')`
//! vs `memchr(0xEE)` at compile time so the serializer pays nothing for a
//! needle it does not use.

use core::fmt;

use aozora_spec::NormalizedOffset;
use aozora_syntax::ast::{LexOutput, NodeRef};

/// First UTF-8 byte of every PUA sentinel (`U+E001..U+E004`).
const SENTINEL_LEAD_BYTE: u8 = 0xEE;
/// Second UTF-8 byte shared by every PUA sentinel.
const SENTINEL_MID_BYTE: u8 = 0x80;
/// Third UTF-8 byte of `INLINE_SENTINEL` (`U+E001`).
const INLINE_SENTINEL_TAIL: u8 = 0x81;
/// Third UTF-8 byte of `BLOCK_LEAF_SENTINEL` (`U+E002`).
const BLOCK_LEAF_SENTINEL_TAIL: u8 = 0x82;
/// Third UTF-8 byte of `BLOCK_OPEN_SENTINEL` (`U+E003`).
const BLOCK_OPEN_SENTINEL_TAIL: u8 = 0x83;
/// Third UTF-8 byte of `BLOCK_CLOSE_SENTINEL` (`U+E004`).
const BLOCK_CLOSE_SENTINEL_TAIL: u8 = 0x84;

/// Which structural role a validated sentinel plays. Mirrors the
/// `NodeRef` variant the registry returns at the sentinel's offset.
#[derive(Clone, Copy)]
pub(crate) enum SentinelKind {
    /// Inline leaf (`U+E001`) — renders within the current line.
    Inline,
    /// Block leaf (`U+E002`) — a standalone block node.
    BlockLeaf,
    /// Container open marker (`U+E003`).
    BlockOpen,
    /// Container close marker (`U+E004`).
    BlockClose,
}

/// Decode the third UTF-8 byte of a PUA-sentinel candidate. `Some` only
/// for the four well-known sentinels; any other byte is a collision
/// (plain text that happens to share the `0xEE 0x80` prefix).
#[inline]
const fn sentinel_kind_for_tail_byte(b: u8) -> Option<SentinelKind> {
    match b {
        INLINE_SENTINEL_TAIL => Some(SentinelKind::Inline),
        BLOCK_LEAF_SENTINEL_TAIL => Some(SentinelKind::BlockLeaf),
        BLOCK_OPEN_SENTINEL_TAIL => Some(SentinelKind::BlockOpen),
        BLOCK_CLOSE_SENTINEL_TAIL => Some(SentinelKind::BlockClose),
        _ => None,
    }
}

/// The sink [`walk`] drives over an [`LexOutput`]: `on_text` for
/// each plain run and `on_node` for each PUA sentinel (with the owned
/// [`NodeRef`] resolved against the output's `Registry`), plus the
/// `on_newline` / `finish` hooks. Both renderers ([`crate::html`]
/// and `serialize`) implement it, so they share this single scan scaffold.
pub(crate) trait WalkSink {
    /// Whether [`walk`] should surface `\n` as [`Self::on_newline`].
    const WANTS_NEWLINES: bool;

    /// Emit a plain-text run. Never called with an empty slice.
    fn on_text(&mut self, text: &str) -> fmt::Result;

    /// React to a `\n`. Only called when `Self::WANTS_NEWLINES`; default
    /// no-op (the serializer copies newlines verbatim through `on_text`).
    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result {
        let _ = next;
        Ok(())
    }

    /// Render a validated sentinel. `kind` is the sentinel's role; `node` is
    /// the owned registry entry at its offset.
    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result;

    /// Finalise after the last run. Default no-op.
    fn finish(&mut self) -> fmt::Result {
        Ok(())
    }
}

/// Validate the candidate sentinel at `cand`, flush the pending plain run, and
/// dispatch the resolved node through the owned registry.
#[inline]
fn handle_sentinel<S: WalkSink>(
    out: &LexOutput,
    cand: usize,
    cursor: &mut usize,
    sink: &mut S,
) -> fmt::Result {
    let normalized = out.normalized.as_str();
    let bytes = normalized.as_bytes();
    if cand + 2 >= bytes.len() || bytes[cand + 1] != SENTINEL_MID_BYTE {
        return Ok(());
    }
    let Some(kind) = sentinel_kind_for_tail_byte(bytes[cand + 2]) else {
        return Ok(());
    };

    if *cursor < cand {
        sink.on_text(&normalized[*cursor..cand])?;
    }
    let byte_pos = u32::try_from(cand).expect("normalized fits u32 per sanitize-stage cap");
    if let Some(node) = out.registry.node_at(NormalizedOffset::new(byte_pos)) {
        sink.on_node(kind, node)?;
    }
    *cursor = cand + 3;
    Ok(())
}

/// Drive `sink` over `out`'s normalized text in a single forward pass, reading
/// `out.normalized.as_str()` and resolving sentinels through the owned
/// [`Registry`](aozora_syntax::ast::Registry).
///
/// # Errors
///
/// Propagates any error the sink returns from a callback.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub(crate) fn walk<S: WalkSink>(out: &LexOutput, sink: &mut S) -> fmt::Result {
    let normalized = out.normalized.as_str();
    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    if S::WANTS_NEWLINES {
        for cand in memchr::memchr2_iter(SENTINEL_LEAD_BYTE, b'\n', bytes) {
            if bytes[cand] == b'\n' {
                if cursor < cand {
                    sink.on_text(&normalized[cursor..cand])?;
                }
                sink.on_newline(bytes.get(cand + 1).copied())?;
                cursor = cand + 1;
            } else {
                handle_sentinel(out, cand, &mut cursor, sink)?;
            }
        }
    } else {
        for cand in memchr::memchr_iter(SENTINEL_LEAD_BYTE, bytes) {
            handle_sentinel(out, cand, &mut cursor, sink)?;
        }
    }

    if cursor < normalized.len() {
        sink.on_text(&normalized[cursor..])?;
    }
    sink.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::ast::{InternStats, Node, NodeStore, Registry};

    /// A [`WalkSink`] that records exactly what the scan emits: the concatenated
    /// plain-text runs and a per-hook event log. Pins the *bytes* the walk
    /// surfaces, not merely that it succeeds.
    #[derive(Default)]
    struct CollectSink {
        text: String,
        nodes: usize,
        finished: usize,
    }

    impl WalkSink for CollectSink {
        const WANTS_NEWLINES: bool = false;

        fn on_text(&mut self, text: &str) -> fmt::Result {
            assert!(
                !text.is_empty(),
                "on_text is never called with an empty run"
            );
            self.text.push_str(text);
            Ok(())
        }

        fn on_node(&mut self, _kind: SentinelKind, _node: NodeRef) -> fmt::Result {
            self.nodes += 1;
            Ok(())
        }

        fn finish(&mut self) -> fmt::Result {
            self.finished += 1;
            Ok(())
        }
    }

    /// Assemble a `LexOutput` whose only populated fields are the normalized
    /// text and the registry — the two the scan reads.
    fn output_with(normalized: &str, registry: Registry) -> LexOutput {
        LexOutput::new(
            normalized.to_owned(),
            String::new(),
            registry,
            Vec::new(),
            0,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            InternStats::default(),
            NodeStore::new(),
        )
    }

    /// U+E041 (`0xEE 0x81 0x81`) shares the `0xEE` lead byte of a PUA sentinel,
    /// and its third byte `0x81` equals `INLINE_SENTINEL_TAIL` — but its second
    /// byte is `0x81`, not the `0x80` (`SENTINEL_MID_BYTE`) every sentinel
    /// carries. The mid-byte guard must reject it so the char flows through
    /// verbatim as plain text.
    ///
    /// Swapping the guard's `||` to `&&` (line 114) makes
    /// `cand + 2 >= len (false) && mid != 0x80` short-circuit to `false`, so the
    /// early return is skipped, the collision decodes as an inline sentinel, the
    /// run is split at the char, and the char is dropped from the output. Pins
    /// the exact surfaced bytes on both sides of the guard.
    #[test]
    fn non_sentinel_pua_collision_flows_through_as_plain_text() {
        let out = output_with("ab\u{E041}cd", Registry::empty());
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(
            sink.text, "ab\u{E041}cd",
            "the collision char must stay a plain-text byte run, not split it"
        );
        assert_eq!(sink.nodes, 0, "a collision dispatches no sentinel node");
    }

    /// The mirror case: a genuine inline sentinel (`0xEE 0x80 0x81`) with a
    /// matching registry entry *is* dispatched — the guard's `||` false-arm
    /// (mid byte `== 0x80`) falls through to the tail decode. Confirms the walk
    /// splits the surrounding text and fires `on_node` exactly once, so the
    /// collision test above is pinning a real behavioural boundary.
    #[test]
    fn genuine_inline_sentinel_is_dispatched_and_splits_the_run() {
        let normalized = "ab\u{E001}cd";
        let cand = "ab".len();
        let byte_pos = u32::try_from(cand).unwrap();
        let registry = Registry::from_sorted_slice(&[(byte_pos, NodeRef::Inline(Node::PageBreak))]);
        let out = output_with(normalized, registry);
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(
            sink.text, "abcd",
            "the sentinel is consumed; only the flanking runs surface"
        );
        assert_eq!(sink.nodes, 1, "exactly one inline node is dispatched");
    }

    /// Plain text with no `0xEE` byte is copied whole through the single final
    /// flush, and `finish` runs exactly once at the end.
    #[test]
    fn plain_text_is_copied_verbatim_and_finished_once() {
        let out = output_with("hello world", Registry::empty());
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(sink.text, "hello world");
        assert_eq!(sink.nodes, 0);
        assert_eq!(sink.finished, 1, "finish fires exactly once");
    }
}
