//! Owned-AST HTML rendering.
//!
//! Owned mirror of `crate::html`: the same single forward walk over the
//! normalized text driving the same block-level `RenderState`, but each PUA
//! sentinel dispatches through an [`OwnedLexOutput`]'s
//! [`RegistryOwned`](aozora_syntax::owned::RegistryOwned) and resolves its
//! interned payloads against the [`NodeStore`], instead of borrowing
//! `&'src str` / `NodeRef<'src>`.
//!
//! The block-structure machinery (`RenderState`'s paragraph / container logic)
//! and the plain-run escaper (`escape_text_chunk`) are **reused** from
//! `crate::html` — they read only `Copy` `RegionFormat` / `RegionClose` /
//! `bool` scalars identical in both worlds, so there is a single
//! container-HTML authority. Only the AST-reading per-node emitters fork, in
//! the private `render_node_owned` module.
//!
//! Proven byte-identical to `crate::html::render_to_string` by the
//! differential gate in `crates/aozora/tests/owned_html_gate.rs`.

use core::fmt;

use aozora_syntax::owned::{NodeRefOwned, NodeStore, OwnedLexOutput};

use crate::html::{RenderState, escape_text_chunk};
use crate::render_node_owned::render_owned;
use crate::walk::{SentinelKind, WalkSinkOwned, walk_owned};

/// Render an [`OwnedLexOutput`] into a fresh `String`.
///
/// Owned mirror of `crate::html::render_to_string`: allocates roughly
/// `2 × normalized.len()` upfront. For streaming consumers prefer
/// [`render_html_owned_into`] to avoid the intermediate `String`.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`fmt::Write`]
/// sink. The internal `expect` covers the trivially unreachable case.
#[must_use]
pub fn render_html_owned(out: &OwnedLexOutput) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_html_owned_into(out, &mut s).expect("writing to String never fails");
    s
}

/// Render an [`OwnedLexOutput`] into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub fn render_html_owned_into<W: fmt::Write>(out: &OwnedLexOutput, writer: &mut W) -> fmt::Result {
    let mut sink = HtmlSinkOwned {
        store: &out.store,
        out: writer,
        state: RenderState::default(),
    };
    walk_owned(out, &mut sink)
}

/// [`WalkSinkOwned`] that emits semantic HTML5 from the owned AST. Owned mirror
/// of `crate::html::HtmlSink`; threads the [`NodeStore`] (the resolve
/// authority) into every AST emitter and reuses the borrowed [`RenderState`]
/// for all block / paragraph / container structure.
struct HtmlSinkOwned<'a, W: fmt::Write> {
    store: &'a NodeStore,
    out: &'a mut W,
    state: RenderState,
}

impl<W: fmt::Write> WalkSinkOwned for HtmlSinkOwned<'_, W> {
    // HTML output treats `\n` as structural (paragraph / line break).
    const WANTS_NEWLINES: bool = true;

    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.state.ensure_in_paragraph(self.out)?;
        escape_text_chunk(text, self.out)
    }

    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result {
        match next {
            // A blank line closes the current paragraph.
            Some(b'\n') => self.state.close_paragraph(self.out),
            // A lone newline inside a paragraph is a line break.
            Some(_) if self.state.in_paragraph => self.out.write_str("<br />\n"),
            // A newline outside a paragraph (e.g. between blocks) is dropped.
            Some(_) | None => Ok(()),
        }
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRefOwned) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRefOwned::Inline(n)) => {
                self.state.ensure_in_paragraph(self.out)?;
                render_owned(n, self.store, self.out)
            }
            (SentinelKind::BlockLeaf, NodeRefOwned::BlockLeaf(n)) => {
                self.state.before_block_emit(self.out)?;
                render_owned(n, self.store, self.out)?;
                self.state.after_block_emit();
                Ok(())
            }
            (SentinelKind::BlockOpen, NodeRefOwned::BlockOpen(open)) => {
                self.state.open_container(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRefOwned::BlockClose(_close)) => {
                self.state.close_container(self.out)
            }
            // Sentinel without a matching registry entry: best-effort skip,
            // mirroring the borrowed sink.
            _ => Ok(()),
        }
    }

    fn finish(&mut self) -> fmt::Result {
        self.state.close_paragraph(self.out)
    }
}
