//! Shared PUA-sentinel walk over a [`LexOutput`]'s normalized text.
//!
//! Both renderers — [`crate::html`] and [`crate::serialize`] — sweep the
//! same normalized text, dispatching each PUA sentinel through the
//! borrowed registry and bulk-copying the plain runs between hits. The
//! scan, sentinel decoding, registry lookup, and cursor bookkeeping are
//! identical; only what happens *at* each event differs. This module
//! owns that single scan and drives a [`WalkSink`] for the
//! renderer-specific work.
//!
//! # Why a byte scan
//!
//! Every PUA sentinel (`U+E001..U+E004`) shares the 2-byte UTF-8 prefix
//! `0xEE 0x80`; the third byte (`0x81..0x84`) names the kind. A single
//! `memchr` finds candidate lead bytes at memory-bandwidth speed (SIMD
//! via the `memchr` crate); each candidate is validated with two byte
//! loads before it is treated as a sentinel, so a PUA collision in the
//! source (Phase 0 records a diagnostic but does not delete the bytes)
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

use aozora_pipeline::LexOutput;
use aozora_spec::NormalizedOffset;
use aozora_syntax::borrowed::NodeRef;

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

/// Renderer-specific handler driven by [`walk`].
///
/// `walk` calls `on_text` for each plain run, `on_node` for each
/// validated sentinel, and (when [`Self::WANTS_NEWLINES`]) `on_newline`
/// for each `\n`. A sink that returns from `on_node` without matching the
/// `(kind, node)` pair leaves the sentinel unrendered — the same
/// best-effort policy both renderers used for a sentinel whose registry
/// entry is missing or of the wrong variant.
pub(crate) trait WalkSink {
    /// Whether `walk` should surface `\n` as [`Self::on_newline`] events.
    /// `true` selects `memchr2(0xEE, '\n')`; `false` selects the cheaper
    /// `memchr(0xEE)` and folds newlines into the plain runs.
    const WANTS_NEWLINES: bool;

    /// Emit a plain-text run (the bytes between two structural events).
    /// Never called with an empty slice.
    fn on_text(&mut self, text: &str) -> fmt::Result;

    /// React to a `\n`. `next` is the byte immediately after it (for the
    /// blank-line vs line-break distinction). Only called when
    /// [`Self::WANTS_NEWLINES`]; the default is a no-op.
    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result {
        let _ = next;
        Ok(())
    }

    /// Render a validated sentinel. `kind` is the sentinel's role; `node`
    /// is the registry entry at its offset. The sink matches the
    /// `(kind, node)` cross-product itself.
    fn on_node(&mut self, kind: SentinelKind, node: NodeRef<'_>) -> fmt::Result;

    /// Finalise after the last run (e.g. close a trailing paragraph).
    /// Default no-op.
    fn finish(&mut self) -> fmt::Result {
        Ok(())
    }
}

/// Validate the sentinel candidate at `cand`, flush the pending plain run
/// before it, and dispatch it to the sink. A collision (invalid tail
/// byte or truncated prefix) leaves `cursor` untouched so the bytes flow
/// through as plain text on the next event.
#[inline]
fn handle_sentinel<S: WalkSink>(
    out: &LexOutput<'_>,
    cand: usize,
    cursor: &mut usize,
    sink: &mut S,
) -> fmt::Result {
    let normalized = out.normalized;
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
    let byte_pos = u32::try_from(cand).expect("normalized fits u32 per Phase 0 cap");
    if let Some(node) = out.registry.node_at(NormalizedOffset::new(byte_pos)) {
        sink.on_node(kind, node)?;
    }
    *cursor = cand + 3;
    Ok(())
}

/// Drive `sink` over `out`'s normalized text in a single forward pass.
///
/// # Errors
///
/// Propagates any error the sink returns from a callback.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited
/// from the lexer's `Span` width contract; in practice unreachable
/// (Phase 0 sanitize already gates on this bound).
pub(crate) fn walk<S: WalkSink>(out: &LexOutput<'_>, sink: &mut S) -> fmt::Result {
    let normalized = out.normalized;
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
