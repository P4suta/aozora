//! Post-comrak AST surgery — splice Aozora nodes at every PUA sentinel
//! the lexer left in the normalized text.
//!
//! The lexer pipeline produces normalized text with Aozora constructs
//! replaced by `U+E001..=U+E004` PUA sentinels plus a
//! `PlaceholderRegistry` that maps each sentinel position back to its
//! original `AozoraNode` / `ContainerKind`. Comrak parses the
//! normalized text as vanilla CommonMark+GFM — it has no Aozora
//! awareness — so sentinels end up as ordinary characters inside
//! `NodeValue::Text` nodes (inline) or as the entire text of
//! single-char paragraphs (block).
//!
//! This module walks the resulting AST and rewires the Aozora nodes:
//!
//! * **Inline** (`U+E001`) — splits a `NodeValue::Text` at each
//!   sentinel, inserting `[Text(before), Aozora(node), Text(after)]`
//!   as sibling nodes in the original's place.
//! * **Block-leaf** (`U+E002`) — replaces the hosting paragraph
//!   with the corresponding block construct in-place.
//! * **Block-open** / **block-close** (`U+E003` / `U+E004`) — stack-
//!   walks the sentinel paragraphs in document order and wraps the
//!   intervening siblings into an `AozoraNode::Container` node.
//!
//! ## Sentinel → registry mapping
//!
//! Comrak does not preserve byte offsets from normalized text into
//! the AST, so the registry cannot be keyed by AST position. Instead
//! we exploit the 1:1 ordering guarantee: the lexer emits sentinels
//! into `normalized` in byte-offset order, and comrak preserves
//! document order, so the N-th inline sentinel encountered in an
//! in-order AST walk is always the N-th entry in `registry.inline`.
//! The same ordering logic applies to each block-sentinel class.
//!
use std::{mem, ptr};

use afm_lexer::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
    PlaceholderRegistry,
};
use afm_syntax::{AozoraNode, Container};
use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};

/// Walk `root` and splice an `Aozora` node for every inline PUA
/// sentinel (`U+E001`) in descendant `Text` nodes.
///
/// The original Text node is detached and replaced in-place by the
/// `[Text(before), Aozora(node), Text(after)]` sibling sequence.
/// Empty leading / trailing chunks are dropped rather than emitted as
/// empty Text nodes.
///
/// Pure mutation; no return value. The `arena` must be the same one
/// that parsed `root` — mixing arenas here is undefined (`typed_arena`
/// allocations only live as long as the arena that owns them).
pub fn splice_inline<'a>(
    arena: &'a Arena<'a>,
    root: &'a AstNode<'a>,
    registry: &PlaceholderRegistry,
) {
    // Snapshot the descendants first so subsequent mutations
    // (detach + insert_before) do not affect the walk.
    let text_nodes: Vec<&AstNode<'_>> = root
        .descendants()
        .filter(|n| matches!(n.data.borrow().value, NodeValue::Text(_)))
        .collect();

    let mut cursor = 0usize;
    for text_node in text_nodes {
        // Clone the text out of its RefCell before we decide to mutate
        // — we only hold the borrow long enough to look at the content.
        let original_text: String = {
            let data = text_node.data.borrow();
            match &data.value {
                NodeValue::Text(t) => t.to_string(),
                _ => continue,
            }
        };

        if !original_text.contains(INLINE_SENTINEL) {
            continue;
        }

        let chunks = split_at_sentinels(&original_text, &mut cursor, registry);

        // Insert chunks as siblings before the original; then detach the
        // original so only the new sequence remains.
        for chunk in chunks {
            let new_node = match chunk {
                Chunk::Text(s) => alloc_text(arena, s),
                Chunk::Aozora(node) => alloc_aozora(arena, node),
            };
            text_node.insert_before(new_node);
        }
        text_node.detach();
    }
}

enum Chunk {
    Text(String),
    Aozora(AozoraNode),
}

fn split_at_sentinels(
    text: &str,
    cursor: &mut usize,
    registry: &PlaceholderRegistry,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch == INLINE_SENTINEL {
            // The lexer guarantees one inline-registry entry per
            // sentinel. If we walk off the end, something upstream
            // desynced (empty registry passed in, or normalized text
            // and registry drifted) — preserve the sentinel character
            // as plain text so the desync is visible in the output
            // rather than silently dropped.
            if let Some((_, node)) = registry.inline.get(*cursor) {
                if !buf.is_empty() {
                    chunks.push(Chunk::Text(mem::take(&mut buf)));
                }
                chunks.push(Chunk::Aozora(node.clone()));
                *cursor += 1;
            } else {
                buf.push(ch);
            }
        } else {
            buf.push(ch);
        }
    }
    if !buf.is_empty() {
        chunks.push(Chunk::Text(buf));
    }
    chunks
}

fn alloc_text<'a>(arena: &'a Arena<'a>, text: String) -> &'a AstNode<'a> {
    // `From<NodeValue> for AstNode<'_>` builds a default-positioned
    // AstNode. Sourcepos is zero because `post_process` has no
    // normalized-to-source line tracking yet; a dedicated
    // `SourceMap` pass would layer that on later if needed.
    arena.alloc(NodeValue::Text(text.into()).into())
}

fn alloc_aozora<'a>(arena: &'a Arena<'a>, node: AozoraNode) -> &'a AstNode<'a> {
    arena.alloc(NodeValue::Aozora(Box::new(node)).into())
}

/// Walk `root` and replace every single-sentinel paragraph
/// (`Paragraph > Text("\u{E002}")`) with the corresponding
/// [`AozoraNode`] from the block-leaf registry.
///
/// Comrak parses the lexer's block-leaf sentinel line (surrounded by
/// `\n`) as a one-char `Paragraph > Text` pair. This pass detects
/// that pattern and swaps the paragraph for a new `Aozora` node in
/// place, preserving sibling order.
///
/// Paired-container splicing (block-open / block-close sentinels) is
/// handled by [`splice_paired_container`]: a single stack-walk over
/// the tagged sentinel paragraphs that wraps sibling blocks into an
/// `AozoraNode::Container` node.
pub fn splice_block_leaf<'a>(
    arena: &'a Arena<'a>,
    root: &'a AstNode<'a>,
    registry: &PlaceholderRegistry,
) {
    let paragraphs: Vec<&AstNode<'_>> = root
        .descendants()
        .filter(|n| matches!(n.data.borrow().value, NodeValue::Paragraph))
        .collect();

    let mut cursor = 0usize;
    for para in paragraphs {
        if !is_single_sentinel_paragraph(para, BLOCK_LEAF_SENTINEL) {
            continue;
        }
        let Some((_, node)) = registry.block_leaf.get(cursor) else {
            // Desync: leave the paragraph in place (the sentinel stays
            // visible in the HTML output, which makes the drift
            // diagnosable) and keep walking.
            continue;
        };
        let aozora = alloc_aozora(arena, node.clone());
        para.insert_before(aozora);
        para.detach();
        cursor += 1;
    }
}

/// Walk `root` and wrap the blocks between each matched
/// `BLOCK_OPEN_SENTINEL` / `BLOCK_CLOSE_SENTINEL` paragraph pair in a
/// new [`AozoraNode::Container`] AST node.
///
/// ## Algorithm
///
/// A single linear pass over the sentinel paragraphs in document
/// order, driven by a stack:
///
/// 1. Enumerate all `Paragraph > Text("\u{E003}" or "\u{E004}")`
///    nodes in document order, binding each `BLOCK_OPEN_SENTINEL`
///    paragraph to its [`afm_syntax::ContainerKind`] via the
///    registry's in-order vectors (the lexer emits opens/closes in
///    the same order as the source bytes, and comrak preserves
///    document order, so the N-th sentinel paragraph and the N-th
///    registry entry correspond).
/// 2. Walk the tagged sequence: push each Open on a stack; when a
///    Close arrives, pop the top Open and wrap everything between
///    them via [`wrap_between`].
///
/// Because we wrap when we see the *close* (i.e. innermost first),
/// subsequent outer wraps naturally pick up the wrapper as a single
/// sibling — no iteration needed for nesting. Runtime is `O(n)` in
/// the sentinel count; `O(1)` extra allocation per wrap (just the
/// new container node).
///
/// ## Orphan handling
///
/// An unmatched open (no close before EOF) leaves its sentinel
/// paragraph in place; same for an unmatched close. The sentinel
/// chars are PUA codepoints which render invisibly but are caught
/// by the property sweep in `tests/post_process_invariants.rs`.
///
/// ## Cross-parent pairs
///
/// A pair whose open and close lie under different parents (e.g. one
/// inside a blockquote and one outside) is not wrapped — the
/// cross-tree surgery is not attempted here. The open and close
/// paragraphs stay, diagnosable via their sentinel content.
pub fn splice_paired_container<'a>(
    arena: &'a Arena<'a>,
    root: &'a AstNode<'a>,
    registry: &PlaceholderRegistry,
) {
    // One snapshot in document order. Subsequent `detach` / `append`
    // do not invalidate borrowed `AstNode` references — typed_arena
    // allocations are pinned — so we can safely hold pointers into
    // the tree during the wrap pass.
    let mut open_cursor = 0usize;
    let mut close_cursor = 0usize;
    let mut tagged: Vec<TaggedPara<'_>> = Vec::new();
    for p in root
        .descendants()
        .filter(|n| matches!(n.data.borrow().value, NodeValue::Paragraph))
    {
        if is_single_sentinel_paragraph(p, BLOCK_OPEN_SENTINEL) {
            if let Some(&(_, kind)) = registry.block_open.get(open_cursor) {
                tagged.push(TaggedPara {
                    para: p,
                    role: Role::Open(kind),
                });
            }
            open_cursor += 1;
        } else if is_single_sentinel_paragraph(p, BLOCK_CLOSE_SENTINEL) {
            if registry.block_close.get(close_cursor).is_some() {
                tagged.push(TaggedPara {
                    para: p,
                    role: Role::Close,
                });
            }
            close_cursor += 1;
        }
    }

    // Stack-based balanced walk. Pop on Close → wrap the innermost
    // matched pair first so outer wraps see a single sibling (the
    // fresh container) covering the inner content.
    let mut stack: Vec<(&AstNode<'_>, afm_syntax::ContainerKind)> = Vec::new();
    for entry in tagged {
        match entry.role {
            Role::Open(kind) => stack.push((entry.para, kind)),
            Role::Close => {
                let Some((open_para, kind)) = stack.pop() else {
                    continue; // orphan close; leave in place
                };
                if !same_parent(open_para, entry.para) {
                    // Cross-parent pair: we already popped the
                    // matching open; leave both sentinel paragraphs
                    // in place (they render invisibly as PUA chars
                    // but survive for diagnostic tooling).
                    continue;
                }
                wrap_between(arena, kind, open_para, entry.para);
            }
        }
    }
}

struct TaggedPara<'a> {
    para: &'a AstNode<'a>,
    role: Role,
}

enum Role {
    Open(afm_syntax::ContainerKind),
    Close,
}

/// True when `a` and `b` are direct children of the same parent node.
fn same_parent<'a>(a: &'a AstNode<'a>, b: &'a AstNode<'a>) -> bool {
    match (a.parent(), b.parent()) {
        (Some(pa), Some(pb)) => ptr::eq(pa, pb),
        _ => false,
    }
}

/// Build a new `Aozora(Container{kind})` node, insert it in front of
/// `open_para`, move every sibling strictly between `open_para` and
/// `close_para` into it (preserving document order), and detach both
/// the open and close sentinel paragraphs.
fn wrap_between<'a>(
    arena: &'a Arena<'a>,
    kind: afm_syntax::ContainerKind,
    open_para: &'a AstNode<'a>,
    close_para: &'a AstNode<'a>,
) {
    let container = alloc_aozora(arena, AozoraNode::Container(Container { kind }));
    // Splice the container into the tree before the open sentinel so
    // it lands in the correct sibling position before we rehome
    // content into it.
    open_para.insert_before(container);

    // Collect siblings between open and close *before* mutating, so
    // detach/append does not perturb the iteration.
    let mut movers: Vec<&AstNode<'_>> = Vec::new();
    let mut cursor = open_para.next_sibling();
    while let Some(node) = cursor {
        if ptr::eq(node, close_para) {
            break;
        }
        movers.push(node);
        cursor = node.next_sibling();
    }
    for n in movers {
        n.detach();
        container.append(n);
    }

    // Finally drop the two sentinel paragraphs.
    open_para.detach();
    close_para.detach();
}

/// Returns `true` when `para` is a `Paragraph` whose single child is
/// a `Text` node containing exactly one `expected` sentinel character
/// (trimmed of surrounding ASCII whitespace that comrak may preserve
/// from line-folding).
fn is_single_sentinel_paragraph(para: &AstNode<'_>, expected: char) -> bool {
    if !matches!(para.data.borrow().value, NodeValue::Paragraph) {
        return false;
    }
    let Some(first) = para.first_child() else {
        return false;
    };
    if !ptr::eq(
        first,
        para.last_child().expect("first_child implies last_child"),
    ) {
        return false;
    }
    let data = first.data.borrow();
    let NodeValue::Text(ref text) = data.value else {
        return false;
    };
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let Some(only) = chars.next() else {
        return false;
    };
    only == expected && chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use afm_lexer::lex;
    use afm_syntax::AozoraNode;
    use comrak::{Arena, Options, parse_document};

    use super::*;

    fn lex_and_parse<'a>(
        arena: &'a Arena<'a>,
        source: &str,
    ) -> (&'a AstNode<'a>, PlaceholderRegistry) {
        let lex_out = lex(source);
        let opts = Options::default();
        let root = parse_document(arena, &lex_out.normalized, &opts);
        (root, lex_out.registry)
    }

    /// Collect every Aozora node's variant discriminator reachable from
    /// `root`. Keeps tests brief.
    fn aozora_nodes<'a>(root: &'a AstNode<'a>) -> Vec<String> {
        root.descendants()
            .filter_map(|n| {
                if let NodeValue::Aozora(ref node) = n.data.borrow().value {
                    Some(format!("{:?}", &**node).chars().take(20).collect())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn plain_text_has_no_aozora_nodes() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "hello こんにちは");
        splice_inline(&arena, root, &registry);
        assert!(aozora_nodes(root).is_empty());
    }

    #[test]
    fn inline_ruby_becomes_one_aozora_node() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "｜漢字《かんじ》");
        splice_inline(&arena, root, &registry);
        let nodes: Vec<&AstNode<'_>> = root
            .descendants()
            .filter(|n| matches!(n.data.borrow().value, NodeValue::Aozora(_)))
            .collect();
        assert_eq!(nodes.len(), 1);
        let data = nodes[0].data.borrow();
        let NodeValue::Aozora(ref aozora) = data.value else {
            panic!("expected Aozora")
        };
        assert!(matches!(**aozora, AozoraNode::Ruby(_)));
    }

    #[test]
    fn surrounding_text_is_preserved_as_sibling_text_nodes() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "前｜漢《かん》後");
        splice_inline(&arena, root, &registry);
        let para = root.first_child().expect("root has paragraph");
        let children: Vec<_> = para.children().collect();
        assert_eq!(children.len(), 3);
        assert!(matches!(
            children[0].data.borrow().value,
            NodeValue::Text(ref t) if t == "前"
        ));
        assert!(matches!(
            children[1].data.borrow().value,
            NodeValue::Aozora(_)
        ));
        assert!(matches!(
            children[2].data.borrow().value,
            NodeValue::Text(ref t) if t == "後"
        ));
    }

    #[test]
    fn two_adjacent_ruby_spans_produce_two_aozora_siblings() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "｜a《あ》｜b《い》");
        splice_inline(&arena, root, &registry);
        let para = root.first_child().unwrap();
        let aozora_count = para
            .children()
            .filter(|n| matches!(n.data.borrow().value, NodeValue::Aozora(_)))
            .count();
        assert_eq!(aozora_count, 2);
    }

    #[test]
    fn empty_registry_leaves_sentinel_in_text_but_does_not_panic() {
        let arena = Arena::new();
        // Directly simulate desync: lex produces a registry, then we
        // splice against an *empty* one. Sentinel chars remain but
        // post_process must not panic.
        let lex_out = lex("｜a《あ》");
        let opts = Options::default();
        let root = parse_document(&arena, &lex_out.normalized, &opts);
        let empty_registry = PlaceholderRegistry::default();
        splice_inline(&arena, root, &empty_registry);
        // No aozora nodes got inserted; sentinel still present.
        let has_sentinel = root
            .descendants()
            .filter_map(|n| match n.data.borrow().value {
                NodeValue::Text(ref t) => Some(t.contains(INLINE_SENTINEL)),
                _ => None,
            })
            .any(|b| b);
        assert!(has_sentinel);
    }

    #[test]
    fn splice_does_not_touch_non_text_nodes() {
        let arena = Arena::new();
        // Heading contains a Text child; ensure heading itself is not
        // mutated into an Aozora node.
        let (root, registry) = lex_and_parse(&arena, "# heading with ｜漢《か》");
        splice_inline(&arena, root, &registry);
        let heading = root.first_child().expect("heading");
        assert!(matches!(heading.data.borrow().value, NodeValue::Heading(_)));
        // The heading should still have at least one Aozora child.
        let has_aozora = heading
            .descendants()
            .any(|n| matches!(n.data.borrow().value, NodeValue::Aozora(_)));
        assert!(has_aozora);
    }

    #[test]
    fn block_sentinel_chars_are_ignored_by_inline_splice() {
        // `［＃改ページ］` generates a U+E002 block-leaf sentinel
        // which the block splice handles. The inline splice must
        // *not* mistakenly consume the block sentinel as inline.
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "前\n［＃改ページ］\n後");
        splice_inline(&arena, root, &registry);
        // No Aozora node should have been spliced — block-leaf
        // splice is a separate pass.
        let aozora_count = root
            .descendants()
            .filter(|n| matches!(n.data.borrow().value, NodeValue::Aozora(_)))
            .count();
        assert_eq!(aozora_count, 0);
    }

    #[test]
    fn splice_block_leaf_replaces_page_break_paragraph() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "前\n\n［＃改ページ］\n\n後");
        splice_block_leaf(&arena, root, &registry);
        // Expect a direct child of the document that is Aozora(PageBreak).
        let page_break_count = root
            .children()
            .filter(|n| {
                if let NodeValue::Aozora(ref node) = n.data.borrow().value {
                    matches!(**node, AozoraNode::PageBreak)
                } else {
                    false
                }
            })
            .count();
        assert_eq!(page_break_count, 1);
    }

    #[test]
    fn splice_block_leaf_preserves_surrounding_paragraphs() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "前\n\n［＃改ページ］\n\n後");
        splice_block_leaf(&arena, root, &registry);
        // First child = Paragraph("前"); middle = Aozora(PageBreak);
        // last = Paragraph("後").
        let kinds: Vec<&'static str> = root
            .children()
            .map(|n| match n.data.borrow().value {
                NodeValue::Paragraph => "paragraph",
                NodeValue::Aozora(_) => "aozora",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["paragraph", "aozora", "paragraph"]);
    }

    #[test]
    fn splice_block_leaf_does_not_touch_non_sentinel_paragraphs() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "plain paragraph");
        splice_block_leaf(&arena, root, &registry);
        // No Aozora child at all.
        assert!(
            root.children()
                .all(|n| !matches!(n.data.borrow().value, NodeValue::Aozora(_))),
        );
    }

    #[test]
    fn splice_block_leaf_handles_section_breaks_too() {
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "前\n\n［＃改丁］\n\n後");
        splice_block_leaf(&arena, root, &registry);
        let has_section_break = root.children().any(|n| {
            matches!(
                n.data.borrow().value,
                NodeValue::Aozora(ref node) if matches!(**node, AozoraNode::SectionBreak(_))
            )
        });
        assert!(has_section_break);
    }

    #[test]
    fn splice_block_leaf_empty_registry_leaves_paragraph_in_place() {
        let arena = Arena::new();
        let lex_out = lex("［＃改ページ］");
        let opts = Options::default();
        let root = parse_document(&arena, &lex_out.normalized, &opts);
        let empty_registry = PlaceholderRegistry::default();
        splice_block_leaf(&arena, root, &empty_registry);
        // The paragraph should remain, still carrying the sentinel as
        // a Text child.
        assert!(
            root.children()
                .any(|n| matches!(n.data.borrow().value, NodeValue::Paragraph)),
        );
        assert!(
            !root
                .children()
                .any(|n| matches!(n.data.borrow().value, NodeValue::Aozora(_))),
        );
    }
}
