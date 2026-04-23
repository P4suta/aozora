//! Render-only adapter implementing [`afm_syntax::AozoraExtension`] for the
//! forked comrak.
//!
//! Since the ADR-0008 cutover (E1), Aozora **parsing** is entirely driven by
//! `afm-lexer` + `afm-parser::post_process`; the old inline / block parse
//! hooks have been removed from the `AozoraExtension` trait and from the
//! comrak fork. This adapter now only implements the **render** surface
//! comrak still delegates to (comrak's HTML renderer calls `render_html`
//! when it walks a `NodeValue::Aozora(_)` arm).
//!
//! D2 will replace the trait-object dispatch with a naked `fn` pointer on
//! `comrak::Options`, at which point this module (and the `AozoraExtension`
//! trait) will be deleted. D1 keeps the trait structure so upstream comrak's
//! renderer does not need to change in the same commit.

use core::fmt;

use afm_syntax::{AozoraExtension, AozoraNode};

use crate::aozora;

/// Zero-state adapter. Registered via
/// `Options::afm_default()` → `Arc::new(AfmAdapter)`. Cheap to share across
/// threads and stateless under `catch_unwind`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AfmAdapter;

impl AozoraExtension for AfmAdapter {
    fn render_html(&self, node: &AozoraNode, writer: &mut dyn fmt::Write) -> fmt::Result {
        aozora::html::render(node, writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use afm_syntax::{Annotation, AnnotationKind};

    #[test]
    fn adapter_renders_annotation_as_hidden_span() {
        let adapter = AfmAdapter;
        let node = AozoraNode::Annotation(Annotation {
            raw: "［＃X］".into(),
            kind: AnnotationKind::Unknown,
        });
        let mut out = String::new();
        adapter.render_html(&node, &mut out).expect("write");
        assert!(
            out.contains("afm-annotation"),
            "render_html must emit afm-annotation wrapper, got {out:?}"
        );
    }

    #[test]
    fn adapter_renders_page_break_as_div() {
        let adapter = AfmAdapter;
        let mut out = String::new();
        adapter
            .render_html(&AozoraNode::PageBreak, &mut out)
            .expect("write");
        assert!(
            out.contains("afm-page-break"),
            "render_html must emit afm-page-break div, got {out:?}"
        );
    }
}
