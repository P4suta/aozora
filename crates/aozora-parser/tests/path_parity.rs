//! Parity harness: diff adapter-path [`parse_via_adapter`] against
//! lexer-path [`parse_via_lexer`] over a curated corpus.
//!
//! Purpose — regression prevention during the ADR-0008 cutover (E1 and
//! the follow-on lexer-recogniser fix commits). Each case either expects
//! byte-identical HTML on both paths (`gap: None`) or documents a known
//! gap with a reason code from [`ExpectedGap`]. Fix commits flip cases
//! from `gap: Some(...)` back to `gap: None`, shrinking the table
//! monotonically.
//!
//! The harness is deliberately strict in both directions:
//!
//! - An **unexpected divergence** (gap claimed `None` but HTML differs)
//!   is a regression — a previously-closed gap has re-opened, or a new
//!   case surfaces an unknown class of divergence.
//! - An **unexpected parity** (gap claimed `Some(...)` but HTML now
//!   matches) is also a failure — the gap has closed and the case needs
//!   its `gap` field cleared. This catches silent drift: a refactor that
//!   accidentally fixes a gap should force us to update the table so we
//!   notice.
//!
//! ## Why render HTML, not walk AST?
//!
//! The HTML is the product users actually see. Comparing rendered bytes
//! forces any semantic drift between the two pipelines to show up in
//! terms a user would recognise. Node-kind distributions are printed
//! alongside for diagnostic context, but the assertion is on HTML.
//!
//! ## Relation to existing tests
//!
//! The golden `tier_a_ruby_recognition_floor` and the hand-written
//! fixtures under `aozora_spec.rs` each exercise one path (whichever
//! [`afm_parser::parse`] currently delegates to). This file is the only
//! one that exercises **both paths on the same input** simultaneously,
//! which is what makes it the regression-monitor for the cutover.

use afm_parser::html::render_root_to_string;
use afm_parser::{Options, parse_via_adapter, parse_via_lexer};
use afm_syntax::AozoraNode;
use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};

/// Classes of known-acceptable divergence between the two paths while
/// the lexer recogniser coverage is still partial. Every variant maps to
/// one (or a small family of) gap-closing commit on the ADR-0008 path.
///
/// When a fix commit closes a gap, it should delete the corresponding
/// variant here (or at least drop its usage from [`CORPUS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
enum ExpectedGap {
    /// Paired-container AST wrap awaits the `AozoraNode::Container`
    /// schema extension (F5). Today the lexer drops open/close markers
    /// into sentinel-only lines but `post_process` has no wrap logic.
    PairedContainerUnimpl,
    /// Lexer output is objectively more correct than adapter output —
    /// the adapter has a pre-existing bug (double-emission of implicit
    /// ruby base text, producing invalid block-in-inline HTML around
    /// block-leaf annotations, etc.) that the lexer pipeline fixes for
    /// free. After E2 removes the adapter, this divergence disappears
    /// by the lexer becoming the only path. Listed here so the harness
    /// doesn't flag these as regressions.
    LexerImprovesAdapter,
}

struct Case {
    label: &'static str,
    input: &'static str,
    gap: Option<ExpectedGap>,
}

/// Curated corpus: every category of Aozora construct we care about
/// during the cutover, plus CommonMark baselines. Keep entries short
/// and self-describing; diagnostic reports print `label` so the case
/// needs to be identifiable at a glance in CI logs.
const CORPUS: &[Case] = &[
    // -------------------------------------------------------------------
    // CommonMark baselines — every path must be identical on these.
    // -------------------------------------------------------------------
    Case {
        label: "plain_text",
        input: "Hello, world.",
        gap: None,
    },
    Case {
        label: "plain_paragraph_with_punctuation",
        input: "あいうえお、かきくけこ。",
        gap: None,
    },
    Case {
        label: "commonmark_emphasis",
        input: "これは **強い** と *弱い* の対比。",
        gap: None,
    },
    Case {
        label: "commonmark_link",
        input: "詳細は [here](https://example.com) を参照。",
        gap: None,
    },
    Case {
        label: "commonmark_inline_code",
        input: "実行は `cargo test` で。",
        gap: None,
    },
    Case {
        label: "commonmark_multi_paragraph",
        input: "第一段落。\n\n第二段落。",
        gap: None,
    },
    // -------------------------------------------------------------------
    // Ruby — explicit and implicit forms; adapter and lexer both ship.
    // -------------------------------------------------------------------
    Case {
        label: "ruby_explicit_single",
        input: "｜青梅《おうめ》へ",
        gap: None,
    },
    Case {
        // Adapter emits `彼は日本<ruby>日本...`, double-printing "日本".
        // Lexer emits `彼は<ruby>日本...`, the correct behaviour.
        label: "ruby_implicit_kanji",
        input: "彼は日本《にほん》へ",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    Case {
        label: "ruby_multiple_in_paragraph",
        input: "｜青梅《おうめ》と｜鶴見《つるみ》の間",
        gap: None,
    },
    Case {
        label: "ruby_followed_by_text",
        input: "｜青梅《おうめ》の朝。",
        gap: None,
    },
    Case {
        label: "ruby_preceded_by_text",
        input: "今朝、｜青梅《おうめ》にて。",
        gap: None,
    },
    // -------------------------------------------------------------------
    // Unknown bracket annotation — adapter and lexer both wrap as
    // hidden afm-annotation span. Parity expected.
    // -------------------------------------------------------------------
    Case {
        label: "unknown_annotation_inline",
        input: "前［＃ほげふが］後",
        gap: None,
    },
    Case {
        label: "unknown_annotation_line_start",
        input: "［＃ほげふが］\n次の行",
        gap: None,
    },
    // -------------------------------------------------------------------
    // Block-leaf annotations at inline position. Adapter embeds the
    // block `<div>` *inside* the surrounding `<p>`, producing invalid
    // HTML (block-in-inline). Lexer correctly splits the paragraph
    // around the block-leaf sentinel, producing well-formed HTML.
    // -------------------------------------------------------------------
    Case {
        label: "page_break_inline_position",
        input: "前［＃改ページ］後",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    Case {
        label: "section_break_inline_position",
        input: "前［＃改丁］後",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    Case {
        label: "page_break_at_document_start",
        input: "［＃改ページ］後",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    Case {
        label: "page_break_at_document_end",
        input: "前［＃改ページ］",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    Case {
        label: "multiple_page_breaks_in_paragraph",
        input: "A［＃改ページ］B［＃改ページ］C",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    // -------------------------------------------------------------------
    // Block-leaf annotations on their own line — both paths emit a
    // block-level node. Parity expected.
    // -------------------------------------------------------------------
    Case {
        // Adapter emits `<p>前\n<div class=\"afm-page-break\"></div>\n後</p>` —
        // a block-level <div> nested inside a <p>, which is invalid HTML.
        // Lexer emits `<p>前</p><div .../><p>後</p>`, structurally valid.
        label: "page_break_line_alone",
        input: "前\n［＃改ページ］\n後",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    // -------------------------------------------------------------------
    // Classifier validation gaps: invalid parametrics that should fall
    // back to the generic Annotation but the lexer promotes anyway.
    // -------------------------------------------------------------------
    Case {
        label: "indent_zero_digit_invalid",
        input: "［＃０字下げ］本文\n",
        gap: None,
    },
    Case {
        label: "indent_zero_ascii_digit_invalid",
        input: "［＃0字下げ］本文\n",
        gap: None,
    },
    Case {
        label: "align_end_zero_digit_invalid",
        input: "前［＃地から0字上げ］後\n",
        gap: None,
    },
    Case {
        label: "forward_bouten_no_preceding_target",
        input: "［＃「X」に傍点］あと\n",
        gap: None,
    },
    Case {
        label: "forward_bouten_target_with_different_char",
        input: "Y［＃「X」に傍点］あと\n",
        gap: None,
    },
    Case {
        label: "forward_tcy_no_preceding_target",
        input: "［＃「29」は縦中横］後\n",
        gap: None,
    },
    // -------------------------------------------------------------------
    // Forward-reference bouten where target does precede — parity
    // expected (both paths recognise).
    // -------------------------------------------------------------------
    Case {
        label: "forward_bouten_with_preceding_target",
        input: "冒頭でXが先行する。X［＃「X」に傍点］の強調。",
        gap: None,
    },
    // -------------------------------------------------------------------
    // Gaiji reference mark. Lexer emits AozoraNode::Gaiji; adapter emits
    // AozoraNode::Annotation. Both render as hidden annotation today so
    // the HTML may actually be parity — mark SemanticUplift if not.
    // -------------------------------------------------------------------
    Case {
        // Adapter wraps the gaiji in a *hidden* afm-annotation span
        // with the raw `［＃…］` body. The lexer promotes to the richer
        // `AozoraNode::Gaiji` and renders a *visible* afm-gaiji span
        // showing the description — strictly more information for
        // readers. Content of the HTML differs; semantics are richer.
        label: "gaiji_reference_with_description",
        input: "語※［＃「木＋吶のつくり」、第3水準1-85-54］で",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    Case {
        label: "gaiji_reference_minimal_body",
        input: "前※［＃「〻」、U+303B］後",
        gap: Some(ExpectedGap::LexerImprovesAdapter),
    },
    // -------------------------------------------------------------------
    // Paired container (字下げ). F5 will wrap children; today both
    // paths leave paragraph structure roughly intact but differ in
    // where the sentinel falls.
    // -------------------------------------------------------------------
    Case {
        label: "paired_container_jisage",
        input: "［＃ここから２字下げ］\n囲まれた段落。\n［＃ここで字下げ終わり］",
        gap: Some(ExpectedGap::PairedContainerUnimpl),
    },
];

// ---------------------------------------------------------------------------
// Core comparison primitives
// ---------------------------------------------------------------------------

fn render_path(
    input: &str,
    via: for<'a> fn(&'a Arena<'a>, &str, &Options<'_>) -> &'a AstNode<'a>,
) -> (String, NodeKindDigest) {
    let arena = Arena::new();
    let opts = Options::afm_default();
    let root = via(&arena, input, &opts);
    let html = render_root_to_string(root, &opts);
    let digest = NodeKindDigest::from_root(root);
    (html, digest)
}

/// Summary of `AozoraNode` variants observed in a parsed document, in
/// document order. Used for diagnostic reports alongside HTML diffs;
/// not part of the assertion itself.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeKindDigest {
    kinds: Vec<&'static str>,
}

impl NodeKindDigest {
    fn from_root<'a>(root: &'a AstNode<'a>) -> Self {
        let mut kinds = Vec::new();
        walk_aozora(root, &mut kinds);
        Self { kinds }
    }
}

fn walk_aozora<'a>(node: &'a AstNode<'a>, out: &mut Vec<&'static str>) {
    if let NodeValue::Aozora(ref boxed) = node.data.borrow().value {
        out.push(variant_name(boxed));
    }
    for child in node.children() {
        walk_aozora(child, out);
    }
}

fn variant_name(node: &AozoraNode) -> &'static str {
    // Matching by discriminant keeps the harness decoupled from concrete
    // node field changes; the xml_node_name is stable.
    match node {
        AozoraNode::Ruby(_) => "Ruby",
        AozoraNode::Bouten(_) => "Bouten",
        AozoraNode::TateChuYoko(_) => "TateChuYoko",
        AozoraNode::Warichu(_) => "Warichu",
        AozoraNode::Annotation(_) => "Annotation",
        AozoraNode::Gaiji(_) => "Gaiji",
        AozoraNode::Kaeriten(_) => "Kaeriten",
        AozoraNode::PageBreak => "PageBreak",
        AozoraNode::SectionBreak(_) => "SectionBreak",
        AozoraNode::AozoraHeading(_) => "AozoraHeading",
        AozoraNode::Indent(_) => "Indent",
        AozoraNode::AlignEnd(_) => "AlignEnd",
        AozoraNode::Sashie(_) => "Sashie",
        AozoraNode::Keigakomi(_) => "Keigakomi",
        _ => "<unknown>",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The master assertion: every case either matches on both paths (when
/// `gap: None`) or diverges in the documented way (when `gap: Some(_)`).
/// Both directions are strict — see the module docs for why.
#[test]
fn lexer_path_matches_adapter_over_curated_corpus() {
    let mut unexpected_diff: Vec<Divergence> = Vec::new();
    let mut unexpected_parity: Vec<&'static str> = Vec::new();

    for case in CORPUS {
        let (adapter_html, adapter_kinds) = render_path(case.input, parse_via_adapter);
        let (lexer_html, lexer_kinds) = render_path(case.input, parse_via_lexer);
        let matches = adapter_html == lexer_html;
        match (matches, case.gap) {
            (true, Some(_)) => unexpected_parity.push(case.label),
            (false, None) => unexpected_diff.push(Divergence {
                label: case.label,
                input: case.input,
                adapter_html,
                lexer_html,
                adapter_kinds,
                lexer_kinds,
            }),
            _ => {}
        }
    }

    assert!(
        unexpected_diff.is_empty() && unexpected_parity.is_empty(),
        "path_parity regressions detected.\n\n{}{}",
        format_unexpected_diffs(&unexpected_diff),
        format_unexpected_parities(&unexpected_parity),
    );
}

struct Divergence {
    label: &'static str,
    input: &'static str,
    adapter_html: String,
    lexer_html: String,
    adapter_kinds: NodeKindDigest,
    lexer_kinds: NodeKindDigest,
}

fn format_unexpected_diffs(diffs: &[Divergence]) -> String {
    use std::fmt::Write;
    if diffs.is_empty() {
        return String::new();
    }
    let mut s = format!("{} case(s) diverged unexpectedly:\n", diffs.len());
    for d in diffs {
        write!(
            &mut s,
            "\n--- {} ---\n  input:        {:?}\n  adapter HTML: {:?}\n  lexer HTML:   {:?}\n  adapter kinds: {:?}\n  lexer kinds:   {:?}\n",
            d.label,
            d.input,
            d.adapter_html,
            d.lexer_html,
            d.adapter_kinds.kinds,
            d.lexer_kinds.kinds,
        )
        .expect("writing to String is infallible");
    }
    s
}

fn format_unexpected_parities(labels: &[&str]) -> String {
    if labels.is_empty() {
        return String::new();
    }
    format!(
        "\n{} case(s) reached parity unexpectedly (flip `gap: Some(...)` to `gap: None` and drop the ExpectedGap variant if no other case references it): {:?}\n",
        labels.len(),
        labels
    )
}

/// Diagnostic report: always-passing test that dumps the current
/// per-case parity status. Useful for copy-paste into PR descriptions
/// when doing gap-fix commits — you can see which gaps closed and which
/// remain open at a glance. Runs via `cargo nextest run -E
/// 'test(path_parity::print_parity_status)'`.
#[test]
#[ignore = "diagnostic; run on demand"]
fn print_parity_status() {
    println!("\nPath parity status ({} cases):\n", CORPUS.len());
    for case in CORPUS {
        let (adapter_html, _adapter_kinds) = render_path(case.input, parse_via_adapter);
        let (lexer_html, _lexer_kinds) = render_path(case.input, parse_via_lexer);
        let matches = adapter_html == lexer_html;
        let tag = match (matches, case.gap) {
            (true, None) => "OK",
            (true, Some(_)) => "DRIFT-TO-PARITY",
            (false, None) => "REGRESSION",
            (false, Some(gap)) => match gap {
                ExpectedGap::PairedContainerUnimpl => "gap:paired-container",
                ExpectedGap::LexerImprovesAdapter => "lexer-better",
            },
        };
        println!("  [{tag:22}] {}", case.label);
    }
}
