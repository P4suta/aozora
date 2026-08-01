#![expect(clippy::expect_used, reason = "fmt::Write into String is infallible")]

//! HTML rendering over the semantic AST.
//!
//! Renders the normalized text in a single forward walk driving a block-level
//! `RenderState`, dispatching each PUA sentinel through an [`LexOutput`]'s
//! [`Registry`](crate::syntax::ast::Registry) and resolving its
//! interned payloads against the [`NodeStore`].
//!
//! The block-structure machinery (`RenderState`'s paragraph / container logic)
//! and the plain-run escaper (`escape_text_chunk`) are **reused** from
//! `crate::render::spelling::html` — they read only `Copy` `RegionFormat` / `RegionClose` /
//! `bool` scalars, so container HTML has a single authority. Only the
//! AST-reading per-node emitters live in the private `render_node`
//! module.

use core::fmt;

use crate::pipeline::lex;
use crate::syntax::DirectiveKind;
use crate::syntax::ast::{LexOutput, Node, NodeRef, NodeStore};

use crate::render::MAX_NESTED_SOURCE_DEPTH;
use crate::render::render_node::render_with_depth;
use crate::render::serialize::{DirectiveNormalization, SerializeOptions, serialize_with};
use crate::render::spelling::html::{RenderState, escape_text, escape_text_chunk};
use crate::render::walk::{NewlineSink, SentinelKind, WalkSink, walk_with_newlines};

/// Options controlling the opt-in HTML render path.
///
/// The default (`directives: Off`) is the byte-identical, non-judgemental
/// render: an `Unknown` directive body the parser did not recognise renders as
/// an inert `<span class="aozora-directive" hidden>`, so output never depends on
/// the notation-hygiene catalogue.
///
/// Opting in through [`crate::Snapshot::to_html_with`] reinterprets near-misses as their
/// canonical spelling — reached *transitively* through the formatter rewrite,
/// never a second copy of the catalogue — so a known 揺れ renders as a real
/// element instead of a hidden directive span. `Canonical` consults Tier1 only
/// (`render --normalize`); `Degraded` additionally reduces the lossy / judgment
/// Tier2 forms (`render --degraded`). See ADR-0022's fourth role and ADR-0026.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderOptions {
    /// Which notation-hygiene tiers to apply to `DirectiveKind::Editorial`
    /// near-misses when rendering. Set through [`RenderOptions::directives`];
    /// private so future options can be added without a breaking change.
    pub(crate) directives: DirectiveNormalization,
}

impl RenderOptions {
    /// Select which notation-hygiene tiers to apply to `DirectiveKind::Editorial`
    /// near-misses. Builder over [`RenderOptions::default`]:
    /// `RenderOptions::default().directives(DirectiveNormalization::Canonical)`.
    #[must_use]
    pub fn directives(mut self, level: DirectiveNormalization) -> Self {
        self.directives = level;
        self
    }
}

/// Render an [`LexOutput`] into HTML **after** normalising its `Unknown`
/// directive near-misses at the given [`DirectiveNormalization`] level.
///
/// This is the opt-in twin of [`render_html`]: it first re-serialises the tree
/// through the formatter rewrite (the single catalogue authority, reached
/// transitively — no catalogue copy lives here), then re-parses that ephemeral
/// canonical source and renders it. The rewrite is an internal, throwaway step:
/// neither the caller's source nor the default parse/render is mutated. A body
/// with no catalogue entry is left verbatim, so it still renders as before.
///
/// `level` is `Canonical` (Tier1) or `Degraded` (Tier1 + Tier2). `Degraded` is
/// the sole construction site of that level, so the lossy / judgment Tier2
/// reductions can reach only this throwaway render buffer — never source.
#[must_use]
pub(crate) fn render_html_normalized(out: &LexOutput, level: DirectiveNormalization) -> String {
    let normalized = serialize_with(out, SerializeOptions { directives: level });
    render_html(&lex(&normalized))
}

/// Render an [`LexOutput`] into a fresh `String`.
///
/// Allocates roughly `2 × normalized.len()` upfront. For streaming consumers
/// prefer [`render_html_into`] to avoid the intermediate `String`.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a [`fmt::Write`]
/// sink. The internal `expect` covers the trivially unreachable case.
#[must_use]
pub(crate) fn render_html(out: &LexOutput) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_html_into(out, &mut s).expect("writing to String never fails");
    s
}

/// Render an [`LexOutput`] into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited from the
/// lexer's `Span` width contract; in practice unreachable.
pub(crate) fn render_html_into<W: fmt::Write>(out: &LexOutput, writer: &mut W) -> fmt::Result {
    let mut sink = HtmlSink::<_, false> {
        store: &out.store,
        out: writer,
        state: RenderState::default(),
        nested_depth: 0,
    };
    walk_with_newlines(out, &mut sink)?;
    sink.finish()
}

pub(super) fn render_inline_source<W: fmt::Write>(
    source: &str,
    writer: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    if nested_depth >= MAX_NESTED_SOURCE_DEPTH {
        return escape_text(source, writer);
    }
    let output = lex(source);
    if output.source_nodes.iter().any(|entry| match entry.node {
        NodeRef::Inline(_) => false,
        NodeRef::BlockLeaf(_) | NodeRef::BlockOpen(_) | NodeRef::BlockClose(_) => true,
    }) {
        return escape_text(source, writer);
    }
    let mut sink = HtmlSink::<_, true> {
        store: &output.store,
        out: writer,
        state: RenderState::default(),
        nested_depth: nested_depth + 1,
    };
    walk_with_newlines(&output, &mut sink)?;
    sink.finish()
}

/// [`WalkSink`] that emits semantic HTML5 from the AST, threading the
/// [`NodeStore`] (the resolve authority) into every AST emitter and reusing
/// `crate::render::spelling::html`'s [`RenderState`] for all block / paragraph / container
/// structure.
struct HtmlSink<'a, W: fmt::Write, const INLINE: bool> {
    store: &'a NodeStore,
    out: &'a mut W,
    state: RenderState,
    nested_depth: usize,
}

impl<W: fmt::Write, const INLINE: bool> HtmlSink<'_, W, INLINE> {
    fn finish(&mut self) -> fmt::Result {
        if INLINE {
            self.state.drain_open_warichu(self.out)
        } else {
            self.state.finish(self.out)
        }
    }
}

impl<W: fmt::Write, const INLINE: bool> WalkSink for HtmlSink<'_, W, INLINE> {
    fn on_text(&mut self, text: &str) -> fmt::Result {
        if INLINE {
            return escape_text_chunk(text, self.out);
        }
        self.state.ensure_in_paragraph(self.out)?;
        escape_text_chunk(text, self.out)
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result {
        if INLINE {
            return match (kind, node) {
                (SentinelKind::Inline, NodeRef::Inline(n))
                | (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(n)) => match &n {
                    Node::Directive(a) if a.kind == DirectiveKind::WarichuOpen => {
                        self.state.open_warichu(self.out)
                    }
                    Node::Directive(a) if a.kind == DirectiveKind::WarichuClose => {
                        self.state.close_warichu(self.out)
                    }
                    _ => render_with_depth(n, self.store, self.out, self.nested_depth),
                },
                _ => Ok(()),
            };
        }
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(n)) => {
                self.state.ensure_in_paragraph(self.out)?;
                // RenderState splits crossed phrasing scopes into balanced
                // fragments instead of emitting overlapping tags (#415).
                match &n {
                    Node::Directive(a) if a.kind == DirectiveKind::WarichuOpen => {
                        self.state.open_warichu(self.out)
                    }
                    Node::Directive(a) if a.kind == DirectiveKind::WarichuClose => {
                        self.state.close_warichu(self.out)
                    }
                    _ => render_with_depth(n, self.store, self.out, self.nested_depth),
                }
            }
            (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(n)) => {
                self.state.before_block_emit(self.out)?;
                render_with_depth(n, self.store, self.out, self.nested_depth)?;
                self.state.after_block_emit();
                Ok(())
            }
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(open)) => {
                self.state.open_container(open, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(close)) => {
                // Pass the close marker's inline-ness so a stray inline close in
                // a paragraph gap cancels a pending reopen rather than popping a
                // block off the stack (#420).
                self.state.close_container(close.is_inline(), self.out)
            }
            // Sentinel without a matching registry entry: best-effort skip.
            _ => Ok(()),
        }
    }
}

impl<W: fmt::Write, const INLINE: bool> NewlineSink for HtmlSink<'_, W, INLINE> {
    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result {
        if INLINE {
            return next.map_or(Ok(()), |_| self.out.write_str("<br />\n"));
        }
        match next {
            Some(b'\n') => self.state.close_paragraph(self.out),
            Some(_) if self.state.in_paragraph => self.out.write_str("<br />\n"),
            Some(_) | None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_render_emits_actual_paragraph_html() {
        let out = lex("Hello.");
        assert_eq!(
            render_html_normalized(&out, DirectiveNormalization::Canonical),
            "<p>Hello.</p>\n",
        );
    }

    #[test]
    fn render_options_directives_changes_near_miss_html() {
        let document = crate::parse("本文［＃ゴチック］続き").expect("source parses");
        let snapshot = document.snapshot();
        let default = snapshot.to_html_with(RenderOptions::default());
        let canonical = snapshot
            .to_html_with(RenderOptions::default().directives(DirectiveNormalization::Canonical));

        assert_ne!(canonical, default);
        assert!(default.contains("aozora-directive"));
        assert!(!canonical.contains("aozora-directive"));
    }

    /// A non-warichu inline directive (`［＃入力者注(5)］`) must fall through to the
    /// per-node emitter: the `WarichuClose` match guard is a genuine equality
    /// test, not an unconditional `true`. Were the guard always true, this
    /// editor note would be routed to `close_warichu` (a no-op with no open
    /// warichu) and its visible `注5` superscript would vanish.
    #[test]
    fn non_warichu_inline_directive_renders_via_emitter_not_close_warichu() {
        let out = lex("本文［＃入力者注(5)］続き");
        assert_eq!(
            render_html(&out),
            "<p>本文<sup class=\"aozora-editor-note\">注5</sup>続き</p>\n",
        );
    }

    #[test]
    fn forward_format_preserves_nested_ruby() {
        let out = lex("二年程｜經《た》つうちに［＃「二年程」～「つうちに」に傍点］");
        let html = render_html(&out);
        assert!(html.contains("<em class=\"aozora-bouten"));
        assert!(html.contains("<ruby>經<rp>(</rp><rt>た</rt>"));
        assert!(!html.contains("｜經《た》"), "html: {html}");
    }

    #[test]
    fn non_bouten_format_preserves_nested_gaiji() {
        let source = "［＃「※［＃「木＋吶のつくり」、第3水準1-85-54］」は太字］";
        let html = render_html(&lex(source));

        assert!(html.contains(r#"<b class="aozora-futoji">"#), "{html}");
        assert!(html.contains(r#"<span class="aozora-gaiji""#), "{html}");
        assert!(html.contains('枘'), "{html}");
        assert!(!html.contains("※［＃"), "{html}");
    }

    #[test]
    fn block_nested_source_falls_back_to_escaped_notation() {
        let html = render_html(&lex("［＃「［＃改ページ］」に傍点］"));

        assert!(html.contains("［＃改ページ］"), "{html}");
        assert!(!html.contains("<div"), "{html}");
        assert!(!html.contains("<br"), "{html}");
    }

    #[test]
    fn nested_source_warichu_is_balanced() {
        let stray_close = render_html(&lex("［＃「［＃割り注終わり］X」に傍点］"));
        assert!(!stray_close.contains("</span>X"), "{stray_close}");

        let unclosed = render_html(&lex("［＃「［＃割り注］X」に傍点］"));
        assert_eq!(
            unclosed.matches(r#"<span class="aozora-warichu">"#).count(),
            unclosed.matches("</span>").count(),
            "{unclosed}"
        );
    }

    #[test]
    fn crossed_warichu_and_region_scopes_are_split_into_valid_html() {
        let warichu_outer = render_html(&lex(
            "［＃割り注］［＃太字］X［＃割り注終わり］Y［＃太字終わり］",
        ));
        assert_eq!(
            warichu_outer,
            "<p><span class=\"aozora-warichu\"><b class=\"aozora-futoji\">X</b></span><b class=\"aozora-futoji\">Y</b></p>\n"
        );

        let region_outer = render_html(&lex(
            "［＃太字］［＃割り注］X［＃太字終わり］Y［＃割り注終わり］",
        ));
        assert_eq!(
            region_outer,
            "<p><b class=\"aozora-futoji\"><span class=\"aozora-warichu\">X</span></b><span class=\"aozora-warichu\">Y</span></p>\n"
        );
    }

    #[test]
    fn nested_unmatched_warichu_close_is_an_inert_marker() {
        let html = render_html(&lex("｜X《［＃割り注終わり］Y》"));

        assert!(!html.contains("<rt></span>"), "{html}");
        assert!(
            html.contains(r#"<span class="aozora-directive" hidden>"#),
            "{html}"
        );
        assert!(html.contains("Y</rt>"), "{html}");
    }

    #[test]
    fn deeply_nested_source_stops_reparsing_and_preserves_escaped_tail() {
        let mut source = "<&".to_owned();
        for _ in 0..MAX_NESTED_SOURCE_DEPTH + 2 {
            source = format!("［＃「{source}」に傍点］");
        }

        let html = render_html(&lex(&source));
        assert_eq!(
            html.matches(r#"<em class="aozora-bouten"#).count(),
            MAX_NESTED_SOURCE_DEPTH + 1,
            "{html}"
        );
        assert!(html.contains("［＃「"), "{html}");
        assert!(html.contains("&lt;&amp;"), "{html}");
        assert!(!html.contains("<&"), "{html}");
    }
}
