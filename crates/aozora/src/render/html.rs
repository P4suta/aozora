#![expect(clippy::expect_used, reason = "fmt::Write into String is infallible")]

use core::fmt;

use crate::pipeline::lex;
use crate::syntax::DirectiveKind;
use crate::syntax::ast::{LexOutput, Node, NodeRef, NodeStore};

use crate::render::MAX_NESTED_SOURCE_DEPTH;
use crate::render::render_node::render_with_depth;
use crate::render::serialize::{DirectiveNormalization, SerializeOptions, serialize_with};
use crate::render::spelling::html::{RenderState, escape_text, escape_text_chunk};
use crate::render::walk::{NewlineSink, SentinelKind, WalkSink, walk_with_newlines};

/// Options for HTML rendering.
///
/// Directive normalization is opt-in and uses an ephemeral source buffer (ADR-0022 and ADR-0026).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderOptions {
    pub(crate) directives: DirectiveNormalization,
}

impl RenderOptions {
    /// Select the directive normalization tier for rendering.
    #[must_use]
    pub fn directives(mut self, level: DirectiveNormalization) -> Self {
        self.directives = level;
        self
    }
}

// Lossy Tier2 reductions affect only this temporary render buffer, preserving source text (ADR-0026).
#[must_use]
pub(crate) fn render_html_normalized(out: &LexOutput, level: DirectiveNormalization) -> String {
    let normalized = serialize_with(out, SerializeOptions { directives: level });
    render_html(&lex(&normalized))
}

#[must_use]
pub(crate) fn render_html(out: &LexOutput) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_html_into(out, &mut s).expect("writing to String never fails");
    s
}

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
                self.state.close_container(close.is_inline(), self.out)
            }
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
    use crate::syntax::alloc::Allocator;

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

    #[test]
    fn non_warichu_directive_reaches_the_emitter_in_both_modes() {
        fn render_editor_note<const INLINE: bool>() -> String {
            let mut allocator = Allocator::new();
            let directive =
                allocator.make_directive("［＃入力者注(5)］", DirectiveKind::EditorNote);
            let node = allocator.annotation(directive);
            let store = allocator.into_store();
            let mut output = String::new();
            let mut sink = HtmlSink::<_, INLINE> {
                store: &store,
                out: &mut output,
                state: RenderState::default(),
                nested_depth: 0,
            };
            sink.on_node(SentinelKind::Inline, NodeRef::Inline(node))
                .expect("render into String is infallible");
            sink.finish().expect("render into String is infallible");
            output
        }

        assert_eq!(
            render_editor_note::<false>(),
            "<p><sup class=\"aozora-editor-note\">注5</sup></p>\n",
        );
        assert_eq!(
            render_editor_note::<true>(),
            "<sup class=\"aozora-editor-note\">注5</sup>",
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
