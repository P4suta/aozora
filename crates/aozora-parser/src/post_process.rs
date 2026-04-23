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
//! * **Block-leaf** (`U+E002`) / **block-open** (`U+E003`) /
//!   **block-close** (`U+E004`) — D4 extends this module to replace
//!   the hosting paragraph with the corresponding block construct
//!   and (for open/close pairs) wrap sibling blocks in the matching
//!   container node.
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
//! ## Staging
//!
//! C4 of the `post_process` branch (this commit, D3) handles inline
//! splice. Block-level splice lands in D4.

use std::mem;

use afm_lexer::{INLINE_SENTINEL, PlaceholderRegistry};
use afm_syntax::AozoraNode;
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
    // AstNode. Sourcepos is zero since post_process does not yet have
    // normalized-to-source line tracking — SourceMap (C5b) will layer
    // that on later.
    arena.alloc(NodeValue::Text(text.into()).into())
}

fn alloc_aozora<'a>(arena: &'a Arena<'a>, node: AozoraNode) -> &'a AstNode<'a> {
    arena.alloc(NodeValue::Aozora(Box::new(node)).into())
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
        // `［＃改ページ］` generates a U+E002 block-leaf sentinel —
        // D4's block splice handles that. For D3 the inline splice
        // must *not* mistakenly consume the block sentinel as inline.
        let arena = Arena::new();
        let (root, registry) = lex_and_parse(&arena, "前\n［＃改ページ］\n後");
        splice_inline(&arena, root, &registry);
        // No Aozora node should have been spliced (block-leaf splice
        // lives in D4).
        let aozora_count = root
            .descendants()
            .filter(|n| matches!(n.data.borrow().value, NodeValue::Aozora(_)))
            .count();
        assert_eq!(aozora_count, 0);
    }
}
