//! Shared PUA-sentinel walk over an [`OwnedLexOutput`]'s normalized text.
//!
//! Both owned renderers — [`crate::html_owned`] and `serialize_owned`
//! — sweep the same normalized text, dispatching each PUA sentinel through the
//! owned registry and bulk-copying the plain runs between hits. The scan,
//! sentinel decoding, registry lookup, and cursor bookkeeping are identical;
//! only what happens *at* each event differs. This module owns that single scan
//! and drives a [`WalkSinkOwned`] for the renderer-specific work.
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
//! with [`WalkSinkOwned::WANTS_NEWLINES`], which selects `memchr2(0xEE, '\n')`
//! vs `memchr(0xEE)` at compile time so the serializer pays nothing for a
//! needle it does not use.

use core::fmt;

use aozora_spec::NormalizedOffset;
use aozora_syntax::owned::{NodeRefOwned, OwnedLexOutput};

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

/// The sink [`walk_owned`] drives over an [`OwnedLexOutput`]: `on_text` for
/// each plain run and `on_node` for each PUA sentinel (with the owned
/// [`NodeRefOwned`] resolved against the output's `RegistryOwned`), plus the
/// `on_newline` / `finish` hooks. Both owned renderers ([`crate::html_owned`]
/// and `serialize_owned`) implement it, so they share this single scan scaffold.
pub(crate) trait WalkSinkOwned {
    /// Whether [`walk_owned`] should surface `\n` as [`Self::on_newline`].
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
    fn on_node(&mut self, kind: SentinelKind, node: NodeRefOwned) -> fmt::Result;

    /// Finalise after the last run. Default no-op.
    fn finish(&mut self) -> fmt::Result {
        Ok(())
    }
}

/// Validate the candidate sentinel at `cand`, flush the pending plain run, and
/// dispatch the resolved node through the owned registry.
#[inline]
fn handle_sentinel_owned<S: WalkSinkOwned>(
    out: &OwnedLexOutput,
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
/// [`RegistryOwned`](aozora_syntax::owned::RegistryOwned).
///
/// # Errors
///
/// Propagates any error the sink returns from a callback.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub(crate) fn walk_owned<S: WalkSinkOwned>(out: &OwnedLexOutput, sink: &mut S) -> fmt::Result {
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
                handle_sentinel_owned(out, cand, &mut cursor, sink)?;
            }
        }
    } else {
        for cand in memchr::memchr_iter(SENTINEL_LEAD_BYTE, bytes) {
            handle_sentinel_owned(out, cand, &mut cursor, sink)?;
        }
    }

    if cursor < normalized.len() {
        sink.on_text(&normalized[cursor..])?;
    }
    sink.finish()
}
