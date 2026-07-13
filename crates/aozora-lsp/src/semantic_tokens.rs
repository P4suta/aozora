//! `textDocument/semanticTokens/full` — colour every notation span
//! the tree-sitter parser has identified.
//!
//! ## Why tree-sitter and not the semantic parser
//!
//! Tree-sitter gives us a wait-free read against the snapshot's
//! cheap-cloned [`Tree`]. The semantic parser would re-parse the whole
//! document on every request — beyond the keystroke-rate budget that
//! semantic tokens fire at.
//!
//! ## Token shapes
//!
//! | Tree-sitter kind        | LSP token type             | Notes                                            |
//! | ----------------------- | -------------------------- | ------------------------------------------------ |
//! | `gaiji`                 | `macro`                    | `※[#…]` whole span — distinct from ruby colour. |
//! | `explicit_ruby`         | (ignored, recurse)         | The wrapper; we emit on the ｜+brackets parts.  |
//! | `implicit_ruby`         | (ignored, recurse)         | Same.                                            |
//! | `ruby_base_explicit`    | `enum`                     | Highlights the base 漢字 of `｜base《reading》`. |
//! | `ruby_base_implicit`    | `enum`                     | Same for `base《reading》` without `｜`.         |
//! | `ruby_reading`          | `string`                   | The reading inside `《…》`.                     |
//!
//! The legend below MUST match what the server publishes in
//! `ServerCapabilities::semantic_tokens_provider`. Both lists live
//! in this module so the two never drift.
//!
//! ## Encoding
//!
//! LSP semantic tokens are a flat `Vec<u32>` of 5-tuples:
//! `(deltaLine, deltaStart, length, tokenType, tokenModifiers)`.
//! - `deltaLine` is relative to the previous token's line
//! - `deltaStart` is relative to the previous token's start char on
//!   the same line, or absolute when `deltaLine > 0`
//! - `length` / `tokenType` / `tokenModifiers` are absolute
//!
//! The encoder below visits tokens in source order (the iterative
//! tree walk is in-order) so the delta encoding is straightforward
//! one-pass.

use std::sync::Arc;

use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType, SemanticTokens};
use tree_sitter::Tree;
use tree_sitter_aozora::kind;

use crate::line_index::LineIndex;
use crate::paragraph::ParagraphSnapshot;

/// LSP semantic-token-type legend. Index = `tokenType` field in the
/// emitted tuples; the values must match the order published in
/// `ServerCapabilities::semantic_tokens_provider`.
#[must_use]
pub fn legend() -> Vec<SemanticTokenType> {
    vec![
        SemanticTokenType::MACRO,  // 0 → gaiji
        SemanticTokenType::ENUM,   // 1 → ruby base
        SemanticTokenType::STRING, // 2 → ruby reading
    ]
}

const TT_GAIJI: u32 = 0;
const TT_RUBY_BASE: u32 = 1;
const TT_RUBY_READING: u32 = 2;

/// Compute every semantic token across the document, walking each
/// paragraph's tree in turn. Returns the LSP-shaped `SemanticTokens`
/// payload (delta-encoded).
///
/// Per-paragraph tree walking is the structural payoff of the
/// paragraph-first model: each `ParagraphSnapshot` has its own
/// `tree` + `line_index`, so we can produce doc-absolute LSP
/// positions without ever materialising a doc-wide tree or
/// `LineIndex`. We keep a running `line_offset` (cumulative newlines
/// across paragraphs already visited) so the per-paragraph
/// positions land on the right document line.
///
/// The per-paragraph walk is iterative single-cursor — same shape
/// as the gaiji extraction walker — so it stays linear in tree-node
/// count.
/// Per-paragraph context handed to the tree walker. Bundling these
/// references keeps [`walk_paragraph_tree`] / [`push_token`] under
/// the workspace `too-many-arguments-threshold`.
struct ParagraphCtx<'a> {
    text: &'a str,
    line_index: &'a LineIndex,
    line_offset: u32,
}

#[must_use]
pub fn semantic_tokens_full(paragraphs: &[Arc<ParagraphSnapshot>]) -> SemanticTokens {
    let mut tokens: Vec<RawToken> = Vec::new();
    let mut line_offset: u32 = 0;
    for paragraph in paragraphs {
        if let Some(tree) = paragraph.tree.as_ref() {
            let ctx = ParagraphCtx {
                text: &paragraph.text,
                line_index: &paragraph.line_index,
                line_offset,
            };
            walk_paragraph_tree(tree, &ctx, &mut tokens);
        }
        line_offset = line_offset.saturating_add(count_newlines(&paragraph.text));
    }
    SemanticTokens {
        result_id: None,
        data: encode_delta(&tokens),
    }
}

fn count_newlines(s: &str) -> u32 {
    u32::try_from(s.bytes().filter(|&b| b == b'\n').count()).unwrap_or(u32::MAX)
}

fn walk_paragraph_tree(tree: &Tree, ctx: &ParagraphCtx<'_>, out: &mut Vec<RawToken>) {
    let mut cursor = tree.root_node().walk();
    'walk: loop {
        let node = cursor.node();
        if node.is_error() {
            // Don't descend; lateral move below.
        } else {
            match node.kind() {
                kind::GAIJI => {
                    push_token(out, node, ctx, TT_GAIJI);
                }
                kind::RUBY_BASE_EXPLICIT | kind::RUBY_BASE_IMPLICIT => {
                    if is_inside_ruby(node) {
                        push_token(out, node, ctx, TT_RUBY_BASE);
                    }
                }
                kind::RUBY_READING => {
                    if is_inside_ruby(node) {
                        push_token(out, node, ctx, TT_RUBY_READING);
                    }
                }
                _ => {
                    if cursor.goto_first_child() {
                        continue;
                    }
                }
            }
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
}

/// `true` iff the node's immediate parent is one of the ruby
/// container kinds. Filters out tree-sitter error-recovery
/// emissions where a stray `ruby_base_implicit` appears with an
/// `ERROR` (or other) parent.
fn is_inside_ruby(node: tree_sitter::Node<'_>) -> bool {
    matches!(
        node.parent().map(|p| p.kind()),
        Some(kind::EXPLICIT_RUBY | kind::IMPLICIT_RUBY)
    )
}

/// Pre-encoded token in absolute coordinates, awaiting the delta
/// pass. We sort/visit in source order so the encoder is one-pass.
#[derive(Debug, Clone, Copy)]
struct RawToken {
    line: u32,
    start_char: u32,
    length: u32,
    token_type: u32,
}

fn push_token(
    out: &mut Vec<RawToken>,
    node: tree_sitter::Node<'_>,
    ctx: &ParagraphCtx<'_>,
    token_type: u32,
) {
    let mut start = ctx.line_index.position(ctx.text, node.start_byte());
    let mut end = ctx.line_index.position(ctx.text, node.end_byte());
    start.line = start.line.saturating_add(ctx.line_offset);
    end.line = end.line.saturating_add(ctx.line_offset);
    // Single-line tokens only (LSP spec); split multi-line spans
    // into per-line segments so the editor highlights each line.
    if start.line == end.line {
        out.push(RawToken {
            line: start.line,
            start_char: start.character,
            length: end.character.saturating_sub(start.character),
            token_type,
        });
        return;
    }
    // Multi-line: emit one token per spanned line. The first line
    // runs from `start.character` to end-of-line; intermediate lines
    // span the full line; the last line runs from 0 to `end.character`.
    // Since we don't know exact line lengths cheaply, we use a large
    // sentinel `u32::MAX >> 1` for intermediate-line lengths — most
    // editors treat oversized lengths as "to end of line".
    let between_len = u32::MAX >> 1;
    let first_line_len = u32::MAX >> 1; // sentinel: rest of line
    out.push(RawToken {
        line: start.line,
        start_char: start.character,
        length: first_line_len,
        token_type,
    });
    for line in (start.line + 1)..end.line {
        out.push(RawToken {
            line,
            start_char: 0,
            length: between_len,
            token_type,
        });
    }
    if end.character > 0 {
        out.push(RawToken {
            line: end.line,
            start_char: 0,
            length: end.character,
            token_type,
        });
    }
}

fn encode_delta(raw: &[RawToken]) -> Vec<SemanticToken> {
    let mut out: Vec<SemanticToken> = Vec::with_capacity(raw.len());
    let mut prev_line: u32 = 0;
    let mut prev_start: u32 = 0;
    for tok in raw {
        let delta_line = tok.line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            tok.start_char.saturating_sub(prev_start)
        } else {
            tok.start_char
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: tok.length,
            token_type: tok.token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = tok.line;
        prev_start = tok.start_char;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use ropey::Rope;

    use crate::paragraph::{ParagraphBuffer, build_paragraph_snapshot, paragraph_byte_ranges};

    fn paragraphs_for(src: &str) -> Vec<Arc<ParagraphSnapshot>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_aozora::LANGUAGE.into())
            .unwrap();
        let rope = Rope::from(src);
        let ranges = paragraph_byte_ranges(&rope);
        let mut out: Vec<Arc<ParagraphSnapshot>> = Vec::new();
        for range in ranges {
            let slice = rope.byte_slice(range.clone()).to_string();
            let mut p = ParagraphBuffer::new(Rope::from(slice));
            p.reparse(&mut parser);
            out.push(Arc::new(build_paragraph_snapshot(&p, range.start)));
        }
        out
    }

    fn tokens_for(src: &str) -> Vec<SemanticToken> {
        let paragraphs = paragraphs_for(src);
        semantic_tokens_full(&paragraphs).data
    }

    #[test]
    fn legend_is_stable_index_order() {
        let l = legend();
        assert_eq!(l[TT_GAIJI as usize], SemanticTokenType::MACRO);
        assert_eq!(l[TT_RUBY_BASE as usize], SemanticTokenType::ENUM);
        assert_eq!(l[TT_RUBY_READING as usize], SemanticTokenType::STRING);
    }

    #[test]
    fn empty_doc_yields_no_tokens() {
        let tokens = tokens_for("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn plain_text_yields_no_tokens() {
        let tokens = tokens_for("ただの文章\n二行目\n");
        assert!(tokens.is_empty());
    }

    #[test]
    fn gaiji_emits_one_macro_token() {
        let src = "※［＃「desc」、第3水準1-85-54］";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TT_GAIJI);
        assert_eq!(tokens[0].delta_line, 0);
        assert_eq!(tokens[0].delta_start, 0);
    }

    #[test]
    fn explicit_ruby_emits_base_then_reading() {
        let src = "｜青空《あおぞら》";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 2, "{tokens:?}");
        assert_eq!(tokens[0].token_type, TT_RUBY_BASE);
        assert_eq!(tokens[1].token_type, TT_RUBY_READING);
        // Reading comes after base on the same line.
        assert_eq!(tokens[1].delta_line, 0);
        assert!(tokens[1].delta_start > 0);
    }

    #[test]
    fn implicit_ruby_emits_base_then_reading() {
        let src = "青空《あおぞら》";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 2, "{tokens:?}");
        assert_eq!(tokens[0].token_type, TT_RUBY_BASE);
        assert_eq!(tokens[1].token_type, TT_RUBY_READING);
    }

    #[test]
    fn delta_encoding_resets_on_new_line() {
        let src = "｜青空《あおぞら》\n｜白雲《はくうん》";
        let tokens = tokens_for(src);
        // Tokens: base1, reading1, base2, reading2
        assert_eq!(tokens.len(), 4);
        // 3rd token (base2) is on the next line, delta_line == 1.
        // delta_start is absolute when delta_line > 0; tree-sitter
        // places `ruby_base_explicit`'s start *after* the `｜` so
        // the absolute char position is 1 (｜ is one UTF-16 unit).
        assert_eq!(tokens[2].delta_line, 1);
        assert_eq!(tokens[2].delta_start, 1);
    }

    #[test]
    fn multiple_gaiji_in_source_order() {
        let src = "※［＃「a」、X］\n※［＃「b」、Y］";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 2);
        assert!(tokens.iter().all(|t| t.token_type == TT_GAIJI));
        assert_eq!(tokens[1].delta_line, 1);
    }

    // --- Direct-unit helpers for the internal functions -----------------
    //
    // The multi-line segmentation branch of `push_token` is unreachable
    // through `semantic_tokens_full`: every tree-sitter token body in the
    // grammar is `[^…\n]+`, so no gaiji / ruby span ever crosses a `\n`.
    // To exercise (and pin) that branch we hand `push_token` the
    // whole-document root node, which legitimately spans multiple lines.

    fn parse_whole(src: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_aozora::LANGUAGE.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    fn find_kind<'t>(node: tree_sitter::Node<'t>, target: &str) -> Option<tree_sitter::Node<'t>> {
        if node.kind() == target {
            return Some(node);
        }
        let mut cursor = node.walk();
        // Each recursion walks with its own cursor, so iterating the
        // borrowed one here is safe (no collect needed).
        node.children(&mut cursor)
            .find_map(|child| find_kind(child, target))
    }

    /// Drive `push_token` directly against a tree's whole-document root
    /// node so the multi-line segmentation branch is reachable in-crate
    /// under default features. `line_offset` is applied exactly as the
    /// real per-paragraph walk applies it.
    fn push_root_token(src: &str, line_offset: u32) -> Vec<RawToken> {
        let tree = parse_whole(src);
        let line_index = LineIndex::new(src);
        let ctx = ParagraphCtx {
            text: src,
            line_index: &line_index,
            line_offset,
        };
        let mut out: Vec<RawToken> = Vec::new();
        push_token(&mut out, tree.root_node(), &ctx, TT_GAIJI);
        out
    }

    /// `count_newlines` counts `\n` BYTES, exactly. `"a\nb\nc"` has two
    /// newlines among five bytes; `"あ\nい\nう"` has two among eleven.
    /// Pinning the exact count kills:
    /// - body → `0` (would report 0 for `"a\nb\nc"`),
    /// - body → `1` (would report 1 for both `""` and `"a\nb\nc"`),
    /// - `==` → `!=` on the byte test (would count the 3 / 15 / 9
    ///   NON-newline bytes instead of the newlines).
    #[test]
    fn count_newlines_counts_newline_bytes_exactly() {
        assert_eq!(count_newlines(""), 0);
        assert_eq!(count_newlines("no newline here"), 0);
        assert_eq!(count_newlines("a\nb\nc"), 2);
        // 11 bytes, 9 of them non-newline, 2 newlines — a byte-count vs
        // newline-count confusion is unambiguous here.
        assert_eq!(count_newlines("あ\nい\nう"), 2);
    }

    /// `is_inside_ruby` is `true` exactly when the node's parent is a
    /// ruby container. The whole-document root node has NO parent, so it
    /// is `false`; a `ruby_base_implicit` inside `base《reading》` sits
    /// under `implicit_ruby`, so it is `true`. Asserting BOTH arms kills
    /// the `-> bool with true` mutation, which flattens the `false` arm.
    #[test]
    fn is_inside_ruby_true_and_false_arms() {
        let tree = parse_whole("青空《あおぞら》");
        // FALSE: the root node has no parent at all.
        assert!(
            !is_inside_ruby(tree.root_node()),
            "root node has no parent → not inside ruby",
        );
        // TRUE: the implicit-ruby base's parent is `implicit_ruby`.
        let base = find_kind(tree.root_node(), kind::RUBY_BASE_IMPLICIT)
            .expect("implicit ruby base present");
        assert!(
            is_inside_ruby(base),
            "ruby_base_implicit's parent is implicit_ruby → inside ruby",
        );
    }

    /// Multi-line token segmentation. Source `"AB\nCDE\nFG"` with
    /// `line_offset = 10` gives start = (line 10, col 0) and
    /// end = (line 12, col 2), so `push_token` emits exactly three
    /// segments: a first-line segment, one between-line segment, and a
    /// last-line segment. Concrete values pin the arithmetic:
    /// - `>>` → `<<` on the between-line sentinel (would make `out[1]`
    ///   `u32::MAX << 1` = `4_294_967_294` instead of `2_147_483_647`),
    /// - `>>` → `<<` on the first-line sentinel (same, for `out[0]`),
    /// - `+` → `-` in the between-line range start (`(10-1)..12` → three
    ///   between segments → `len` 5, and `out[1].line` 9),
    /// - `+` → `*` in the between-line range start (`(10*1)..12` → two
    ///   between segments → `len` 4, and `out[1].line` 10),
    /// - `>` → `==` and `>` → `<` on the last-line guard (`end.character`
    ///   is 2, so both drop the last segment → `len` 2).
    #[test]
    fn push_token_multiline_emits_first_between_last_segments() {
        let out = push_root_token("AB\nCDE\nFG", 10);
        assert_eq!(out.len(), 3, "first + one between + last: {out:?}");

        // First-line segment.
        assert_eq!(out[0].line, 10);
        assert_eq!(out[0].start_char, 0);
        assert_eq!(out[0].length, u32::MAX >> 1);

        // Between-line segment: exactly line 11.
        assert_eq!(out[1].line, 11);
        assert_eq!(out[1].start_char, 0);
        assert_eq!(out[1].length, u32::MAX >> 1);

        // Last-line segment: cols 0..2 on line 12.
        assert_eq!(out[2].line, 12);
        assert_eq!(out[2].start_char, 0);
        assert_eq!(out[2].length, 2);
    }

    /// Multi-line token whose END lands at column 0 of a fresh line, so
    /// the last-line segment MUST be suppressed. Source `"AB\nCD\n"` with
    /// `line_offset = 5` gives start = (line 5, col 0) and
    /// end = (line 7, col 0). Because `end.character == 0`, only the
    /// first + between segments are emitted (2 total).
    ///
    /// This is the case that separates `>` from `>=` on the last-line
    /// guard: `0 > 0` is false (correct — no empty trailing segment), but
    /// `0 >= 0` is true (would push a bogus zero-length line-7 segment,
    /// making `len` 3). Kills `>` → `>=`.
    #[test]
    fn push_token_multiline_end_at_line_start_emits_no_last_segment() {
        let out = push_root_token("AB\nCD\n", 5);
        assert_eq!(out.len(), 2, "first + one between, no last: {out:?}");
        assert_eq!(out[0].line, 5);
        assert_eq!(out[1].line, 6);
        assert!(
            out.iter().all(|t| t.line != 7),
            "no zero-length segment on the end line: {out:?}",
        );
    }
}
