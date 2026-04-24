//! Parser for aozora-flavored-markdown.
//!
//! Layers Aozora Bunko typography (ruby, bouten, 縦中横, `［＃...］` annotations,
//! …) on top of a vendored comrak fork (see `/upstream/comrak`). Public entry
//! points:
//!
//! - [`parse`] — run the parser over a UTF-8 source into a comrak arena,
//!   returning a [`ParseResult`] carrying the root node, any lexer
//!   diagnostics, and the [`ParseArtifacts`] needed by [`serialize`].
//! - [`serialize`] — invert the pipeline, emitting afm text from a
//!   [`ParseResult`] via registry-driven PUA-sentinel substitution.
//! - [`html::render_to_string`] — render the parsed tree to HTML.
//! - [`Options`] — configuration; defaults enable the Aozora render hook.
//!
//! Internal layout (see ADR-0008 for the architectural rationale):
//! - `aozora/html` is the HTML renderer; its `render` function is registered
//!   on `comrak::Options.extension.render_aozora` as a naked `fn` pointer.
//! - `post_process` splices `NodeValue::Aozora(...)` nodes into comrak's AST
//!   at each PUA sentinel the lexer planted, including stack-walked
//!   paired-container wrapping.

#![forbid(unsafe_code)]

pub mod aozora;
pub mod html;
pub mod serialize;

#[doc(hidden)]
pub mod test_support;

use afm_lexer::{Diagnostic, PlaceholderRegistry};
use comrak::Arena;
use comrak::nodes::AstNode;

pub mod post_process;

pub use afm_lexer::Diagnostic as LexerDiagnostic;
pub use comrak::{Arena as ComrakArena, Options as ComrakOptions};
pub use serialize::serialize;

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
    /// Default configuration for afm documents: Aozora render hook enabled,
    /// GFM super-set enabled (tables, strikethrough, autolink, tasklist), and
    /// the CommonMark 0.31.2 defaults left intact.
    #[must_use]
    pub fn afm_default() -> Self {
        let mut comrak = comrak::Options::default();
        comrak.extension.render_aozora = Some(aozora::html::render);
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

/// Output of [`parse`]: the AST root plus every diagnostic the lexer
/// emitted for the input and (when the Aozora pipeline ran) the raw
/// [`ParseArtifacts`] needed to invert the pipeline back to afm text.
///
/// The lifetime `'a` is the arena lifetime — `root` is a reference
/// into the typed-arena allocator the caller provided. Dropping the
/// arena invalidates the result, so callers must keep both alive
/// for the same scope (just as comrak's own `parse_document` output
/// requires).
///
/// `diagnostics` is always present; it is `Vec::new()` when the
/// lexer found nothing to complain about. Consumers that want a
/// pass/fail decision (the CLI's `--strict` flag, Language-Server
/// integrations, corpus sweeps) can inspect `diagnostics.is_empty()`
/// without having to rerun the lexer.
///
/// `artifacts` is populated when the Aozora pipeline ran (the
/// `render_aozora` hook is registered on the options — which
/// [`Options::afm_default`] does). It carries the normalized text
/// and the placeholder registry, and powers [`serialize`] without
/// re-running the lexer or re-walking the AST. For
/// [`Options::commonmark_only`] / [`Options::gfm_only`] the field
/// is `None`.
#[derive(Debug)]
pub struct ParseResult<'a> {
    /// Root of the parsed document tree. Alive for the arena lifetime.
    pub root: &'a AstNode<'a>,
    /// Non-fatal observations from the lexer (unclosed opens, stray
    /// triggers, PUA collisions, …). Empty on the happy path.
    pub diagnostics: Vec<Diagnostic>,
    /// Lexer-side artifacts. `None` when the Aozora pipeline was
    /// not invoked (commonmark-only / gfm-only paths).
    pub artifacts: Option<ParseArtifacts>,
}

/// Inputs to [`serialize`] that the lexer computed during [`parse`].
///
/// Holds the PUA-sentinel-normalized text and the placeholder
/// registry that maps every sentinel position back to the originating
/// [`afm_syntax::AozoraNode`] / [`afm_syntax::ContainerKind`].
///
/// This bundle is the lexer's side of the parse pipeline captured so
/// the serializer can run the inverse transformation without having
/// to re-parse. Storing it on [`ParseResult`] costs one String + one
/// struct move (no cloning) — the lexer produced them fresh and they
/// would otherwise be dropped at the end of `parse()`.
#[derive(Debug, Clone)]
pub struct ParseArtifacts {
    /// Normalized source text: the original afm input with every
    /// Aozora span replaced by a PUA sentinel, CommonMark structure
    /// otherwise preserved.
    pub normalized: String,
    /// Sentinel-position → originating `AozoraNode` / `ContainerKind`
    /// lookup. Consumed by [`serialize`] via the registry's
    /// `inline_at` / `block_leaf_at` / `block_open_at` / `block_close_at`
    /// binary-search accessors.
    pub registry: PlaceholderRegistry,
}

/// Parse a UTF-8 source buffer into a comrak AST with Aozora annotations
/// recognised.
///
/// ADR-0008 pipeline:
///
/// 1. [`afm_lexer::lex`] — BOM strip, CRLF→LF, `〔...〕` accent
///    decomposition (ADR-0004), tokenise + pair + classify + normalise
///    every Aozora construct into a PUA sentinel (`U+E001..U+E004`)
///    plus a `PlaceholderRegistry` that maps each sentinel back to its
///    `AozoraNode` / `ContainerKind`. Diagnostics from this pass are
///    forwarded into [`ParseResult::diagnostics`] verbatim.
/// 2. `comrak::parse_document` on the normalised text — comrak sees
///    only plain CommonMark+GFM. Upstream has no Aozora parse hooks;
///    the render-side `fn` pointer on
///    `Options::extension::render_aozora` is the only comrak/afm seam.
/// 3. [`post_process::splice_inline`] / [`post_process::splice_block_leaf`]
///    / [`post_process::splice_paired_container`] — walk the resulting
///    AST, replace sentinels with real `NodeValue::Aozora(...)` nodes
///    from the registry, and wrap paired-container pairs into an
///    `AozoraNode::Container` block.
///
/// When the Aozora render hook is not enabled on `options`, this is a
/// straight `comrak::parse_document` passthrough — CommonMark / GFM
/// only — and [`ParseResult::diagnostics`] is empty.
pub fn parse<'a>(arena: &'a Arena<'a>, input: &str, options: &Options<'_>) -> ParseResult<'a> {
    if options.comrak.extension.render_aozora.is_some() {
        let lex_out = afm_lexer::lex(input);
        let root = comrak::parse_document(arena, &lex_out.normalized, &options.comrak);
        post_process::splice_inline(arena, root, &lex_out.registry);
        post_process::splice_block_leaf(arena, root, &lex_out.registry);
        post_process::splice_paired_container(arena, root, &lex_out.registry);
        // Move normalized + registry into ParseArtifacts (no clone);
        // diagnostics break out separately so `serialize(&result)` never
        // has to probe them.
        ParseResult {
            root,
            diagnostics: lex_out.diagnostics,
            artifacts: Some(ParseArtifacts {
                normalized: lex_out.normalized,
                registry: lex_out.registry,
            }),
        }
    } else {
        ParseResult {
            root: comrak::parse_document(arena, input, &options.comrak),
            diagnostics: Vec::new(),
            artifacts: None,
        }
    }
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
        let result = parse(&arena, "Hello, world.", &opts);
        let paragraph = result
            .root
            .first_child()
            .expect("document has a first child");
        assert_eq!(paragraph.data.borrow().value.xml_node_name(), "paragraph");
    }

    #[test]
    fn ruby_with_explicit_delimiter_captures_base_and_reading() {
        let nodes = collect_aozora("｜青梅《おうめ》へ");
        assert_eq!(nodes.len(), 1);
        let AozoraNode::Ruby(r) = &nodes[0] else {
            panic!("expected Ruby, got {:?}", nodes[0]);
        };
        assert_eq!(r.base.as_plain().expect("plain"), "青梅");
        assert_eq!(r.reading.as_plain().expect("plain"), "おうめ");
        assert!(r.delim_explicit);
    }

    #[test]
    fn ruby_with_implicit_delimiter_recovers_base_from_kanji() {
        let nodes = collect_aozora("彼は日本《にほん》へ");
        assert_eq!(nodes.len(), 1);
        let AozoraNode::Ruby(r) = &nodes[0] else {
            panic!("expected Ruby, got {:?}", nodes[0]);
        };
        assert_eq!(r.base.as_plain().expect("plain"), "日本");
        assert_eq!(r.reading.as_plain().expect("plain"), "にほん");
        assert!(!r.delim_explicit);
    }

    #[test]
    fn bracketed_page_break_is_consumed_inline_and_promoted() {
        // `［＃改ページ］` promotes to `AozoraNode::PageBreak`; the bracket text
        // no longer survives — not even inside the afm-annotation wrapper.
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
        // The `※［＃…］` reference mark must produce *some* Aozora node
        // — the adapter path promotes to the generic `Annotation` (it
        // never introspects gaiji descriptions), while the lexer path
        // promotes to the richer `Gaiji` variant. Either is acceptable;
        // the hard invariant is that the `［＃` marker is consumed so
        // the Tier-A no-bare-bracket guarantee still holds.
        let src = "語※［＃「木＋吶のつくり」、第3水準1-85-54］で";
        let nodes = collect_aozora(src);
        assert!(
            nodes
                .iter()
                .any(|n| matches!(n, AozoraNode::Annotation(_) | AozoraNode::Gaiji(_))),
            "expected at least one Annotation or Gaiji node, got {nodes:?}"
        );
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
        let result = parse(&arena, "｜青梅《おうめ》へ", &opts);
        assert!(
            result.diagnostics.is_empty(),
            "commonmark-only pass-through must not emit lexer diagnostics"
        );
        let mut found = Vec::<AozoraNode>::new();
        test_support::collect_aozora_recursive(result.root, &mut found);
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
