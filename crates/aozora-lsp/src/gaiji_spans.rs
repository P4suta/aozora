//! Gaiji-span extraction primitives.
//!
//! The pure walker that extracts every `※［＃description、mencode］` span
//! from a single tree-sitter [`Tree`] in local (tree-relative) offsets.
//! It is deliberately paragraph-agnostic — [`crate::paragraph`] owns the
//! shift into doc-absolute coordinates.
//!
//! The walk holds one `TreeCursor` across the whole tree (a recursive
//! form would allocate a cursor per node). The `Query` API is slower
//! here: its general pattern-matching automaton has more per-node
//! overhead than this single-kind dispatch.

use std::sync::Arc;

use tree_sitter::{Node, Tree};
use tree_sitter_aozora::kind;

/// One `※［＃description、mencode］` occurrence.
///
/// `description` and `mencode` are `Arc<str>` rather than `String`
/// so the snapshot rebuild can reuse them across snapshot generations
/// via pointer bump — the body of a gaiji span doesn't change unless
/// its containing paragraph was re-parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaijiSpan {
    /// Byte offset of the leading `※`. Whether this is paragraph-
    /// local or doc-absolute depends on the caller — the walker
    /// returns local; `crate::paragraph::build_paragraph_snapshot`
    /// (private) shifts them to absolute.
    pub start_byte: u32,
    /// Byte offset just past the closing `］`. Same coordinate
    /// frame as `start_byte`.
    pub end_byte: u32,
    /// Description text — the bit between `「` and `」`, brackets
    /// stripped. Used as the resolution key for the gaiji-chuki
    /// dictionary fallback.
    pub description: Arc<str>,
    /// `第3水準1-85-54` / `U+1234` / similar — `None` if the source
    /// omitted the mencode tail.
    pub mencode: Option<Arc<str>>,
}

/// Walk `tree` once and extract every gaiji span.
///
/// Returned spans have **byte offsets relative to `text`** (the tree's
/// coordinate frame). Output is sorted by `start_byte` because the
/// tree visit is in source order.
#[must_use]
pub fn extract_gaiji_spans_from_tree(tree: &Tree, text: &str) -> Arc<[Arc<GaijiSpan>]> {
    let mut spans: Vec<Arc<GaijiSpan>> = Vec::new();
    let mut cursor = tree.root_node().walk();
    'walk: loop {
        let node = cursor.node();
        if node.kind() == kind::GAIJI {
            if let Some(span) = build_span(node, text) {
                spans.push(Arc::new(span));
            }
            // gaiji nodes are leaves for this walk — skip descent.
        } else if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    spans.into()
}

fn build_span(gaiji: Node<'_>, text: &str) -> Option<GaijiSpan> {
    // gaiji wraps a slug whose `body` field carries
    // `description、mencode`.
    let mut cursor = gaiji.walk();
    let slug = gaiji
        .named_children(&mut cursor)
        .find(|c| c.kind() == kind::SLUG)?;
    let body_node = slug.child_by_field_name("body")?;
    let body_text = body_node.utf8_text(text.as_bytes()).ok()?;
    let (description, mencode) = parse_body(body_text);
    Some(GaijiSpan {
        start_byte: u32::try_from(gaiji.start_byte()).unwrap_or(u32::MAX),
        end_byte: u32::try_from(gaiji.end_byte()).unwrap_or(u32::MAX),
        description,
        mencode,
    })
}

/// Split `description, mencode` from a slug body. The first `、`
/// delimits description from mencode; if absent, the whole body is
/// the description and mencode is `None`. Strips the surrounding
/// `「…」` from the description if present.
fn parse_body(body: &str) -> (Arc<str>, Option<Arc<str>>) {
    let (description, rest) = body.find('、').map_or((body, None), |i| {
        let head = body[..i].trim();
        let tail = body[i + '、'.len_utf8()..].trim();
        (head, Some(tail))
    });
    let description = description.trim().trim_matches(|c| c == '「' || c == '」');
    let mencode = rest.map(str::trim).filter(|s| !s.is_empty()).map(Arc::from);
    (Arc::from(description), mencode)
}

/// Filter `spans` to those whose `start_byte` lies in
/// `[start_byte, end_byte)`. Uses binary search on the sorted
/// `start_byte` field so the per-request cost stays
/// `O(log spans + matches)`.
#[must_use]
pub(crate) fn spans_in_byte_range(
    spans: &[Arc<GaijiSpan>],
    start_byte: usize,
    end_byte: usize,
) -> &[Arc<GaijiSpan>] {
    let start = u32::try_from(start_byte).unwrap_or(u32::MAX);
    let end = u32::try_from(end_byte).unwrap_or(u32::MAX);
    let lo = spans.partition_point(|s| s.end_byte <= start);
    let hi = spans.partition_point(|s| s.start_byte < end);
    &spans[lo..hi.max(lo)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse(src: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_aozora::LANGUAGE.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    #[test]
    fn empty_source_yields_no_spans() {
        let tree = parse("");
        assert!(extract_gaiji_spans_from_tree(&tree, "").is_empty());
    }

    #[test]
    fn extracts_one_gaiji_span() {
        let src = "前※［＃「desc」、第3水準1-85-54］後";
        let tree = parse(src);
        let spans = extract_gaiji_spans_from_tree(&tree, src);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(&*span.description, "desc");
        assert_eq!(span.mencode.as_deref(), Some("第3水準1-85-54"));
        assert_eq!(span.start_byte as usize, src.find('※').unwrap());
    }

    #[test]
    fn extracts_multiple_spans_in_source_order() {
        let src = "※［＃「a」、第3水準1-85-54］\n※［＃「b」、第3水準1-85-9］";
        let tree = parse(src);
        let spans = extract_gaiji_spans_from_tree(&tree, src);
        assert_eq!(spans.len(), 2);
        assert!(spans[0].start_byte < spans[1].start_byte);
        assert_eq!(&*spans[0].description, "a");
        assert_eq!(&*spans[1].description, "b");
    }

    #[test]
    fn description_only_form_yields_none_mencode() {
        let src = "※［＃「desc-only」］";
        let tree = parse(src);
        let spans = extract_gaiji_spans_from_tree(&tree, src);
        assert_eq!(spans.len(), 1);
        assert_eq!(&*spans[0].description, "desc-only");
        assert!(spans[0].mencode.is_none());
    }

    #[test]
    fn binary_search_filters_out_of_range_spans() {
        let src = "※［＃「a」、X］\n※［＃「b」、X］\n※［＃「c」、X］";
        let tree = parse(src);
        let spans = extract_gaiji_spans_from_tree(&tree, src);
        let b_start = src.find("※［＃「b").unwrap();
        let b_end = src
            .match_indices('］')
            .nth(1)
            .map(|(i, _)| i + '］'.len_utf8())
            .unwrap();
        let inside = spans_in_byte_range(&spans, b_start, b_end);
        assert_eq!(inside.len(), 1);
        assert_eq!(&*inside[0].description, "b");
    }
}
