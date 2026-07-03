//! Owned-AST HTML rendering.
//!
//! Renders the normalized text in a single forward walk driving a block-level
//! `RenderState`, dispatching each PUA sentinel through an [`OwnedLexOutput`]'s
//! [`RegistryOwned`](aozora_syntax::owned::RegistryOwned) and resolving its
//! interned payloads against the [`NodeStore`].
//!
//! The block-structure machinery (`RenderState`'s paragraph / container logic)
//! and the plain-run escaper (`escape_text_chunk`) are **reused** from
//! `crate::html` — they read only `Copy` `RegionFormat` / `RegionClose` /
//! `bool` scalars, so container HTML has a single authority. Only the
//! AST-reading per-node emitters live in the private `render_node_owned`
//! module.

use core::fmt;

use aozora_syntax::DirectiveKind;
use aozora_syntax::owned::{NodeOwned, NodeRefOwned, NodeStore, OwnedLexOutput};

use crate::html::{RenderState, escape_text_chunk};
use crate::render_node_owned::render_owned;
use crate::walk::{SentinelKind, WalkSinkOwned, walk_owned};

/// Render an [`OwnedLexOutput`] into a fresh `String`.
///
/// Allocates roughly `2 × normalized.len()` upfront. For streaming consumers
/// prefer [`render_html_owned_into`] to avoid the intermediate `String`.
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

/// [`WalkSinkOwned`] that emits semantic HTML5 from the owned AST, threading the
/// [`NodeStore`] (the resolve authority) into every AST emitter and reusing
/// `crate::html`'s [`RenderState`] for all block / paragraph / container
/// structure.
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
                // Inline warichu open / close are structural, not per-node:
                // their span must stay balanced across paragraph boundaries, so
                // `RenderState` owns the depth counter (mirroring container
                // balance) rather than the AST emitter writing the tags
                // unconditionally (#415). Every other inline node renders as
                // before. Check `kind` through a borrow, then move `n` into
                // `render_owned` on the fall-through.
                match &n {
                    NodeOwned::Directive(a) if a.kind == DirectiveKind::WarichuOpen => {
                        self.state.open_warichu(self.out)
                    }
                    NodeOwned::Directive(a) if a.kind == DirectiveKind::WarichuClose => {
                        self.state.close_warichu(self.out)
                    }
                    _ => render_owned(n, self.store, self.out),
                }
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
            // Sentinel without a matching registry entry: best-effort skip.
            _ => Ok(()),
        }
    }

    fn finish(&mut self) -> fmt::Result {
        // Close any region a source left open (an unbalanced ［＃ここから…］):
        // to_html must emit balanced markup even though to_source keeps the
        // imbalance verbatim. The unclosed region extends to end-of-document.
        self.state.drain_open_containers(self.out)?;
        self.state.close_paragraph(self.out)
    }
}
