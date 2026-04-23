//! Parser for aozora-flavored-markdown.
//!
//! Layers Aozora Bunko typography (ruby, bouten, 縦中横, `［＃...］` annotations,
//! …) on top of a vendored comrak fork (see `/upstream/comrak`). Public entry
//! points:
//!
//! - [`parse`] — run the parser over a UTF-8 source into a comrak arena,
//!   returning the root node and a Diagnostics collector.
//! - [`html::render_to_string`] — render the parsed tree to HTML.
//! - [`Options`] — configuration; defaults enable the Aozora extension.
//!
//! Internal layout:
//! - `adapter` implements [`afm_syntax::AozoraExtension`] for the fork.
//! - `aozora/` holds the per-kind recognisers (`ruby.rs`, future `bouten.rs`, …)
//!   and the HTML renderer (`html.rs`).

#![forbid(unsafe_code)]

pub mod aozora;
pub mod html;
pub mod preparse;

mod adapter;

#[doc(hidden)]
pub mod test_support;

use std::sync::Arc;

use comrak::Arena;
use comrak::nodes::AstNode;

pub use adapter::AfmAdapter;
pub use comrak::{Arena as ComrakArena, Options as ComrakOptions};

/// Parse-time options.
///
/// Today this is a thin wrapper around `comrak::Options` with the Aozora
/// extension pre-registered; future milestones will surface afm-specific knobs
/// (strict mode, diagnostic verbosity, paired-block recognition toggles) here.
#[derive(Debug, Clone, Default)]
pub struct Options<'c> {
    pub comrak: comrak::Options<'c>,
}

impl Options<'_> {
    /// Default configuration for afm documents: Aozora extension enabled, GFM
    /// super-set enabled (tables, strikethrough, autolink, tasklist), and the
    /// CommonMark 0.31.2 defaults left intact.
    #[must_use]
    pub fn afm_default() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.extension.aozora = Some(Arc::new(AfmAdapter));
        comrak.extension.strikethrough = true;
        comrak.extension.table = true;
        comrak.extension.autolink = true;
        comrak.extension.tasklist = true;
        Self { comrak }
    }

    /// Plain CommonMark (no Aozora, no GFM). Used by the spec-conformance
    /// tests to verify our wrapper doesn't perturb comrak's CommonMark
    /// behaviour. Enables raw-HTML rendering (`render.unsafe`) because the
    /// CommonMark spec's expected outputs are all unsanitised.
    #[must_use]
    pub fn commonmark_only() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.render.r#unsafe = true;
        Self { comrak }
    }

    /// Pure-GFM feature set: the extensions the official GFM 0.29 spec
    /// exercises (strikethrough, tables, autolink, tasklist, tagfilter) with
    /// NO Aozora extension registered. Used by the GFM spec-conformance test
    /// to verify comrak's GFM output survives our wrapper without drift.
    /// `render.unsafe` is enabled for the same reason as `commonmark_only`.
    #[must_use]
    pub fn gfm_only() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.extension.strikethrough = true;
        comrak.extension.table = true;
        comrak.extension.autolink = true;
        comrak.extension.tasklist = true;
        comrak.extension.tagfilter = true;
        comrak.render.r#unsafe = true;
        Self { comrak }
    }
}

/// Parse a UTF-8 source buffer into a comrak AST with Aozora annotations
/// recognised. Delegates to `comrak::parse_document`; the arena is caller-owned
/// so downstream consumers control allocator lifetime.
///
/// When the Aozora extension is enabled on `options`, input runs through
/// [`preparse::apply_preparse`] first so accent decomposition inside `〔...〕`
/// spans is Unicode-normalised before comrak sees it (see ADR-0004).
#[must_use]
pub fn parse<'a>(arena: &'a Arena<'a>, input: &str, options: &Options<'_>) -> &'a AstNode<'a> {
    let source: std::borrow::Cow<'_, str> = if options.comrak.extension.aozora.is_some() {
        preparse::apply_preparse(input)
    } else {
        std::borrow::Cow::Borrowed(input)
    };
    comrak::parse_document(arena, &source, &options.comrak)
}

#[cfg(test)]
mod tests {
    use super::*;
    use afm_syntax::AozoraNode;
    use comrak::Arena;
    use pretty_assertions::assert_eq;
    use test_support::collect_aozora;

    #[test]
    fn parses_plain_paragraph_tree_shape() {
        let arena = Arena::new();
        let opts = Options::afm_default();
        let root = parse(&arena, "Hello, world.", &opts);
        let paragraph = root.first_child().expect("document has a first child");
        assert_eq!(paragraph.data.borrow().value.xml_node_name(), "paragraph");
    }

    #[test]
    fn ruby_with_explicit_delimiter_captures_base_and_reading() {
        let nodes = collect_aozora("｜青梅《おうめ》へ");
        assert_eq!(nodes.len(), 1);
        let AozoraNode::Ruby(r) = &nodes[0] else {
            panic!("expected Ruby, got {:?}", nodes[0]);
        };
        assert_eq!(&*r.base, "青梅");
        assert_eq!(&*r.reading, "おうめ");
        assert!(r.delim_explicit);
    }

    #[test]
    fn ruby_with_implicit_delimiter_recovers_base_from_kanji() {
        let nodes = collect_aozora("彼は日本《にほん》へ");
        assert_eq!(nodes.len(), 1);
        let AozoraNode::Ruby(r) = &nodes[0] else {
            panic!("expected Ruby, got {:?}", nodes[0]);
        };
        assert_eq!(&*r.base, "日本");
        assert_eq!(&*r.reading, "にほん");
        assert!(!r.delim_explicit);
    }

    #[test]
    fn bracketed_page_break_is_consumed_inline_and_promoted() {
        // C2 promotes ［＃改ページ］ to AozoraNode::PageBreak; the bracket text
        // no longer survives at all, even inside the afm-annotation wrapper.
        let src = "前［＃改ページ］後";
        let nodes = collect_aozora(src);
        assert_eq!(nodes.len(), 1, "expected 1 promoted node, got {nodes:?}");
        assert!(
            matches!(nodes[0], AozoraNode::PageBreak),
            "expected PageBreak, got {:?}",
            nodes[0]
        );
        // Tier A still holds: ［＃ appears nowhere in the HTML.
        let html = html::render_to_string(src);
        assert_tier_a_no_bare_brackets(&html);
    }

    #[test]
    fn unknown_bracketed_annotation_stays_in_annotation_wrapper() {
        let src = "前［＃ほげふが］後";
        let nodes = collect_aozora(src);
        assert_eq!(nodes.len(), 1);
        let AozoraNode::Annotation(a) = &nodes[0] else {
            panic!("expected Annotation, got {:?}", nodes[0]);
        };
        assert_eq!(&*a.raw, "［＃ほげふが］");
        let html = html::render_to_string(src);
        assert_tier_a_no_bare_brackets(&html);
    }

    #[test]
    fn gaiji_reference_mark_is_consumed() {
        let src = "語※［＃「木＋吶のつくり」、第3水準1-85-54］で";
        let nodes = collect_aozora(src);
        assert!(nodes.iter().any(|n| matches!(n, AozoraNode::Annotation(_))));
        let html = html::render_to_string(src);
        assert_tier_a_no_bare_brackets(&html);
    }

    /// Tier A canary: `［＃` and `※［＃` never appear in the output outside an
    /// `afm-annotation` wrapper. Used by parse/render integration tests.
    fn assert_tier_a_no_bare_brackets(html: &str) {
        test_support::assert_no_bare(html, "［＃");
    }

    #[test]
    fn commonmark_only_mode_does_not_recognise_ruby() {
        let arena = Arena::new();
        let opts = Options::commonmark_only();
        let root = parse(&arena, "｜青梅《おうめ》へ", &opts);
        let mut found = Vec::<AozoraNode>::new();
        test_support::collect_aozora_recursive(root, &mut found);
        assert_eq!(
            found.len(),
            0,
            "plain CommonMark leaked Aozora nodes: {found:?}"
        );
    }

    #[test]
    fn multiple_ruby_annotations_in_one_paragraph_all_captured() {
        let nodes = collect_aozora("｜青梅《おうめ》と｜鶴見《つるみ》の間");
        assert_eq!(nodes.len(), 2);
        for n in &nodes {
            assert!(matches!(n, AozoraNode::Ruby(_)));
        }
    }
}
