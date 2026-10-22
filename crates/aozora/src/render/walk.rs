#![expect(
    clippy::expect_used,
    reason = "the render visitor writes into an infallible String buffer"
)]

//! Drives rendering and serialization sinks over normalized text through [`walk`].

use core::fmt;

use crate::spec::NormalizedOffset;
use crate::syntax::ast::{LexOutput, NodeRef};

const SENTINEL_LEAD_BYTE: u8 = 0xEE;
const SENTINEL_BYTE_LEN: usize = 3;
const SENTINEL_MID_BYTE: u8 = 0x80;
const INLINE_SENTINEL_TAIL: u8 = 0x81;
const BLOCK_LEAF_SENTINEL_TAIL: u8 = 0x82;
const BLOCK_OPEN_SENTINEL_TAIL: u8 = 0x83;
const BLOCK_CLOSE_SENTINEL_TAIL: u8 = 0x84;

#[derive(Clone, Copy)]
pub(crate) enum SentinelKind {
    Inline,
    BlockLeaf,
    BlockOpen,
    BlockClose,
}

#[inline]
const fn sentinel_kind_for_tail_byte(b: u8) -> Option<SentinelKind> {
    match b {
        INLINE_SENTINEL_TAIL => Some(SentinelKind::Inline),
        BLOCK_LEAF_SENTINEL_TAIL => Some(SentinelKind::BlockLeaf),
        BLOCK_OPEN_SENTINEL_TAIL => Some(SentinelKind::BlockOpen),
        BLOCK_CLOSE_SENTINEL_TAIL => Some(SentinelKind::BlockClose),
        _ => None,
    }
}

pub(crate) trait WalkSink {
    /// Receives a nonempty plain-text run.
    fn on_text(&mut self, text: &str) -> fmt::Result;

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef) -> fmt::Result;
}

pub(crate) trait NewlineSink: WalkSink {
    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result;
}

#[inline]
fn handle_sentinel<S: WalkSink>(
    out: &LexOutput,
    cand: usize,
    cursor: &mut usize,
    sink: &mut S,
) -> fmt::Result {
    let normalized = out.normalized.as_str();
    let bytes = normalized.as_bytes();
    if cand + 2 >= bytes.len() || bytes[cand + 1] != SENTINEL_MID_BYTE {
        return Ok(());
    }
    let Some(kind) = sentinel_kind_for_tail_byte(bytes[cand + 2]) else {
        return Ok(());
    };

    if *cursor < cand {
        sink.on_text(&normalized[*cursor..cand])?;
    }
    let byte_pos = u32::try_from(cand).expect("normalized fits u32 per sanitize-stage cap");
    if let Some(node) = out.registry.node_at(NormalizedOffset::new(byte_pos)) {
        sink.on_node(kind, node)?;
    }
    *cursor = cand + SENTINEL_BYTE_LEN;
    Ok(())
}

pub(crate) fn walk<S: WalkSink>(out: &LexOutput, sink: &mut S) -> fmt::Result {
    let normalized = out.normalized.as_str();
    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    for cand in memchr::memchr_iter(SENTINEL_LEAD_BYTE, bytes) {
        handle_sentinel(out, cand, &mut cursor, sink)?;
    }

    if cursor < normalized.len() {
        sink.on_text(&normalized[cursor..])?;
    }
    Ok(())
}

pub(crate) fn walk_with_newlines<S: NewlineSink>(out: &LexOutput, sink: &mut S) -> fmt::Result {
    let normalized = out.normalized.as_str();
    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    for cand in memchr::memchr2_iter(SENTINEL_LEAD_BYTE, b'\n', bytes) {
        if bytes[cand] == b'\n' {
            if cursor < cand {
                sink.on_text(&normalized[cursor..cand])?;
            }
            sink.on_newline(bytes.get(cand + 1).copied())?;
            cursor = cand + 1;
        } else {
            handle_sentinel(out, cand, &mut cursor, sink)?;
        }
    }

    if cursor < normalized.len() {
        sink.on_text(&normalized[cursor..])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::ast::{Node, NodeStore, Registry};

    #[derive(Default)]
    struct CollectSink {
        text: String,
        nodes: usize,
        newlines: usize,
    }

    impl WalkSink for CollectSink {
        fn on_text(&mut self, text: &str) -> fmt::Result {
            assert!(
                !text.is_empty(),
                "on_text is never called with an empty run"
            );
            self.text.push_str(text);
            Ok(())
        }

        fn on_node(&mut self, _kind: SentinelKind, _node: NodeRef) -> fmt::Result {
            self.nodes += 1;
            Ok(())
        }
    }

    impl NewlineSink for CollectSink {
        fn on_newline(&mut self, _next: Option<u8>) -> fmt::Result {
            self.newlines += 1;
            Ok(())
        }
    }

    fn output_with(normalized: &str, registry: Registry) -> LexOutput {
        LexOutput::new(
            normalized.to_owned(),
            String::new(),
            true,
            registry,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            NodeStore::new(),
        )
    }

    #[test]
    fn non_sentinel_pua_collision_flows_through_as_plain_text() {
        let out = output_with("ab\u{E041}cd", Registry::empty());
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(
            sink.text, "ab\u{E041}cd",
            "the collision char must stay a plain-text byte run, not split it"
        );
        assert_eq!(sink.nodes, 0, "a collision dispatches no sentinel node");
    }

    #[test]
    fn genuine_inline_sentinel_is_dispatched_and_splits_the_run() {
        let normalized = "ab\u{E001}cd";
        let cand = "ab".len();
        let byte_pos = u32::try_from(cand).unwrap();
        let registry = Registry::from_sorted_slice(&[(byte_pos, NodeRef::Inline(Node::PageBreak))]);
        let out = output_with(normalized, registry);
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(
            sink.text, "abcd",
            "the sentinel is consumed; only the flanking runs surface"
        );
        assert_eq!(sink.nodes, 1, "exactly one inline node is dispatched");
    }

    #[test]
    fn sentinel_at_end_does_not_emit_empty_text() {
        let normalized = "\u{E001}";
        let registry = Registry::from_sorted_slice(&[(0, NodeRef::Inline(Node::PageBreak))]);
        let out = output_with(normalized, registry);
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert!(sink.text.is_empty());
        assert_eq!(sink.nodes, 1);
    }

    #[test]
    fn plain_text_is_copied_verbatim() {
        let out = output_with("hello world", Registry::empty());
        let mut sink = CollectSink::default();
        walk(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(sink.text, "hello world");
        assert_eq!(sink.nodes, 0);
    }

    #[test]
    fn newline_walk_surfaces_each_newline() {
        let out = output_with("a\nb\n", Registry::empty());
        let mut sink = CollectSink::default();
        walk_with_newlines(&out, &mut sink).expect("walk into a collector is infallible");
        assert_eq!(sink.text, "ab");
        assert_eq!(sink.newlines, 2);
    }
}
