//! Owned lex API + the single-pass owned normalizer.
//!
//! Produces an [`LexOutput`] whose normalized text, registry, and side
//! tables are owned (lifetime-free, `Send + Sync`). The classify stage builds
//! [`Node`] directly through an
//! [`Allocator`](aozora_syntax::alloc::Allocator); this module's
//! [`Normalizer`] is the PUA-rewriter + position-recorder over those spans.
//! Every interned string lives once, in the allocator's
//! `NodeStore`, which threads straight into
//! the output — no arena, no conversion step.
//!
//! ## Pipeline
//!
//! 1. The sanitize / tokenize / pair stages run as owned-data helpers operating
//!    on byte spans and event indices — they never construct AST.
//! 2. The classify stage is invoked with an
//!    [`Allocator`](aozora_syntax::alloc::Allocator); owned AST
//!    nodes land in its `NodeStore`, strings
//!    interned through the store's [`StrInterner`](aozora_syntax::ast::StrInterner)
//!    so byte-equal content shares a single id.
//! 3. A single fused walk emits the PUA-rewritten text and builds the
//!    position-keyed registry + source-keyed side table, recording each
//!    `Node` (which is `Copy`) directly.

use core::mem::discriminant;

use crate::lexer::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, ClassifiedSpan,
    INLINE_SENTINEL, SpanKind,
};
use aozora_spec::{Diagnostic, NormalizedOffset, Span};
use aozora_syntax::ast::{ContainerPair, LexOutput, Node, NodeRef, SourceNode};
use aozora_syntax::{DirectiveKind, LineFormat, RegionClose, RegionFormat};

/// Run the lex pipeline and materialise the result as an owned, lifetime-free
/// [`LexOutput`] (`Send + Sync`).
///
/// The native owned producer: the classify stage builds the owned tree in one
/// pass (the way the retired borrowed `lex` built the borrowed one), so the
/// returned output owns all its payloads (interned strings, content / segment
/// runs, side tables). This is what `Document::parse` / `Document::lex`
/// call.
#[must_use]
pub fn lex(source: &str) -> LexOutput {
    crate::pipeline::Pipeline::run_to_completion(source)
}

/// Output recorder for the [`Normalizer`] fold.
///
/// Holds the position-keyed registry entries and the source-keyed side table.
/// Each emitted [`Node`] is `Copy`, so recording it is a plain push — no
/// conversion, no second store (the allocator's
/// `NodeStore` is authoritative and threads
/// into the output separately).
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) entries: Vec<(u32, NodeRef)>,
    pub(crate) source_nodes: Vec<SourceNode>,
}

impl Recorder {
    fn with_capacity(hint: usize) -> Self {
        Self {
            entries: Vec::with_capacity(hint),
            source_nodes: Vec::with_capacity(hint),
        }
    }

    fn push(&mut self, pos: u32, source_span: Span, nref: NodeRef) {
        self.entries.push((pos, nref));
        self.source_nodes.push(SourceNode {
            source_span,
            node: nref,
        });
    }

    fn record_inline(&mut self, pos: u32, source_span: Span, node: Node) {
        self.push(pos, source_span, NodeRef::Inline(node));
    }

    fn record_block_leaf(&mut self, pos: u32, source_span: Span, node: Node) {
        self.push(pos, source_span, NodeRef::BlockLeaf(node));
    }

    fn record_block_open(&mut self, pos: u32, source_span: Span, region: RegionFormat) {
        self.push(pos, source_span, NodeRef::BlockOpen(region));
    }

    fn record_block_close(&mut self, pos: u32, source_span: Span, close: RegionClose) {
        self.push(pos, source_span, NodeRef::BlockClose(close));
    }
}

/// Single-pass owned normalizer.
///
/// Streams the PUA-rewritten text into `out` and records each emitted
/// sentinel's node through `recorder`. The classifier emits spans in source
/// order, so every sentinel position is strictly greater than the previous and
/// the registry consumes the entries via `from_sorted_slice` without
/// re-sorting. The owned nodes are built upstream by the
/// [`Allocator`](aozora_syntax::alloc::Allocator) during the
/// classify stage; this walker is the PUA-rewriter + position-recorder and does
/// zero AST allocation of its own.
#[derive(Debug)]
pub(crate) struct Normalizer<'src> {
    pub(crate) out: String,
    source: &'src str,
    pub(crate) recorder: Recorder,
    /// Stack of in-flight container opens awaiting their matching close. Each
    /// entry is the (open `NormalizedOffset`, open [`RegionFormat`]) pushed by
    /// [`SpanKind::BlockOpen`] emission; [`SpanKind::BlockClose`] pops and emits
    /// a [`ContainerPair`]. The open payload is authoritative.
    open_stack: Vec<(NormalizedOffset, RegionFormat)>,
    /// Resolved container open/close pairs in close order.
    pub(crate) container_pairs: Vec<ContainerPair>,
    /// Diagnostics observed during the fold (post-classify).
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Family tag of the most recent single-line layout directive on the
    /// current source line, if any.
    pending_single_line: Option<&'static str>,
    /// Nesting depth of open `［＃割り注］` … `［＃割り注終わり］` ranges.
    warichu_depth: u32,
}

impl<'src> Normalizer<'src> {
    pub(crate) fn new(source: &'src str, span_capacity_hint: usize) -> Self {
        Self {
            out: String::with_capacity(source.len()),
            source,
            recorder: Recorder::with_capacity(span_capacity_hint),
            open_stack: Vec::with_capacity(span_capacity_hint / 40),
            container_pairs: Vec::with_capacity(span_capacity_hint / 40),
            diagnostics: Vec::new(),
            pending_single_line: None,
            warichu_depth: 0,
        }
    }

    fn current_pos(&self) -> u32 {
        u32::try_from(self.out.len()).expect("normalized fits u32 per sanitize-stage cap")
    }

    pub(crate) fn emit(&mut self, span: &ClassifiedSpan) {
        match &span.kind {
            SpanKind::Plain => {
                self.out.push_str(span.source_span.slice(self.source));
            }
            SpanKind::Newline => {
                self.out.push('\n');
                self.pending_single_line = None;
            }
            SpanKind::Aozora(node) => {
                self.track_single_line_break(*node, span.source_span);
                if is_standalone_block_for_render(*node) {
                    self.out.push_str("\n\n");
                    let pos = self.current_pos();
                    self.out.push(BLOCK_LEAF_SENTINEL);
                    self.out.push_str("\n\n");
                    self.recorder
                        .record_block_leaf(pos, span.source_span, *node);
                } else {
                    let pos = self.current_pos();
                    self.out.push(INLINE_SENTINEL);
                    self.recorder.record_inline(pos, span.source_span, *node);
                }
            }
            SpanKind::BlockOpen(container) => {
                let inline = container.is_inline();
                if !inline {
                    self.out.push_str("\n\n");
                }
                let pos = self.current_pos();
                self.out.push(BLOCK_OPEN_SENTINEL);
                if !inline {
                    self.out.push_str("\n\n");
                }
                self.recorder
                    .record_block_open(pos, span.source_span, *container);
                self.open_stack
                    .push((NormalizedOffset::new(pos), *container));
            }
            SpanKind::BlockClose(close) => {
                let inline = close.is_inline();
                if !inline {
                    self.out.push_str("\n\n");
                }
                let pos = self.current_pos();
                self.out.push(BLOCK_CLOSE_SENTINEL);
                if !inline {
                    self.out.push_str("\n\n");
                }
                self.recorder
                    .record_block_close(pos, span.source_span, *close);
                if let Some((open_pos, open_kind)) = self.open_stack.pop() {
                    self.push_container_mismatch(open_kind, *close, span.source_span);
                    self.container_pairs.push(ContainerPair {
                        kind: open_kind,
                        open: open_pos,
                        close: NormalizedOffset::new(pos),
                    });
                }
            }
        }
    }

    /// Flag a container close whose family differs from its matched open.
    fn push_container_mismatch(&mut self, open: RegionFormat, close: RegionClose, span: Span) {
        let expected = RegionClose::of(open);
        if discriminant(&expected) != discriminant(&close) {
            self.diagnostics
                .push(Diagnostic::mismatched_container_close(
                    span,
                    open.kind_str(),
                    close.kind_str(),
                ));
        } else if let (
            RegionClose::Bouten {
                kind: open_kind, ..
            },
            RegionClose::Bouten {
                kind: close_kind, ..
            },
        ) = (expected, close)
            && open_kind.is_line() != close_kind.is_line()
        {
            self.diagnostics
                .push(Diagnostic::mismatched_bouten_container(
                    span,
                    open_kind.family_str(),
                    close_kind.family_str(),
                ));
        }
    }

    /// Single-line-container break tracker for one classified `Aozora` node.
    fn track_single_line_break(&mut self, node: Node, break_span: Span) {
        match node {
            Node::Line(LineFormat::Indent { .. }) => {
                self.pending_single_line = Some("indent");
            }
            Node::Line(LineFormat::AlignEnd { .. }) => {
                self.pending_single_line = Some("align-end");
            }
            Node::Line(LineFormat::Center { .. }) => {
                self.pending_single_line = Some("center");
            }
            Node::Line(LineFormat::Gothic) => {
                self.pending_single_line = Some("line-gothic");
            }
            Node::Directive(ann) => match ann.kind {
                DirectiveKind::WarichuOpen => self.warichu_depth += 1,
                DirectiveKind::WarichuClose => {
                    self.warichu_depth = self.warichu_depth.saturating_sub(1);
                }
                _ => {}
            },
            Node::PageBreak | Node::SectionBreak(_) => {
                if let Some(container) = self.pending_single_line.take() {
                    self.diagnostics
                        .push(Diagnostic::break_in_single_line_container(
                            break_span, container,
                        ));
                } else if self.warichu_depth > 0 {
                    self.diagnostics
                        .push(Diagnostic::break_in_single_line_container(
                            break_span, "warichu",
                        ));
                }
            }
            _ => {}
        }
    }
}

/// Whether an owned AST node is a standalone block (renders on its own line, no
/// surrounding plain-text context required). Pinned by variant kind so adding a
/// new standalone-block variant only needs updating here.
fn is_standalone_block_for_render(node: Node) -> bool {
    matches!(
        node,
        Node::PageBreak
            | Node::SectionBreak(_)
            | Node::BodyEnd
            | Node::Heading(_)
            | Node::Illustration(_)
    )
}

// Container registries: pure copy of (u32, RegionFormat) / RegionClose — all
// `Copy`. A static assertion pins the no-conversion expectation.
const _: fn() = || {
    fn assert_copy<T: Copy>() {}
    assert_copy::<(u32, RegionFormat)>();
    assert_copy::<RegionClose>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_spec::{NormalizedOffset, Sentinel};
    use aozora_syntax::IndentBlock;
    use aozora_syntax::ast::{Content, Directive, StrId};

    #[test]
    fn recorder_with_capacity_preallocates_both_tables() {
        // Kills the `with_capacity -> Default::default()` mutant: the hinted
        // constructor must reserve capacity in both side tables, whereas
        // `Default` yields empty (capacity-0) vecs.
        let r = Recorder::with_capacity(64);
        assert!(
            r.entries.capacity() >= 64,
            "entries should be preallocated to the hint, got {}",
            r.entries.capacity()
        );
        assert!(
            r.source_nodes.capacity() >= 64,
            "source_nodes should be preallocated to the hint, got {}",
            r.source_nodes.capacity()
        );
    }

    #[test]
    fn track_single_line_break_sets_family_tag_per_line_format() {
        // Kills the deleted `Center { .. }` and `Gothic` match arms (and pins
        // the sibling indent / align-end arms): each single-line layout node
        // must stamp its own family tag onto `pending_single_line`.
        let cases: &[(Node, &'static str)] = &[
            (
                Node::Line(LineFormat::Indent {
                    amount: 1,
                    end_offset: None,
                }),
                "indent",
            ),
            (Node::Line(LineFormat::AlignEnd { offset: 0 }), "align-end"),
            (Node::Line(LineFormat::Center { page: false }), "center"),
            (Node::Line(LineFormat::Gothic), "line-gothic"),
        ];
        for &(node, expected) in cases {
            let mut norm = Normalizer::new("", 0);
            norm.track_single_line_break(node, Span::new(0, 0));
            assert_eq!(
                norm.pending_single_line,
                Some(expected),
                "node {node:?} should flag single-line family {expected}"
            );
        }
    }

    #[test]
    fn track_single_line_break_warichu_close_decrements_depth() {
        // Kills the deleted `DirectiveKind::WarichuClose` match arm: after an
        // open then a close the nesting depth must return to zero. With the
        // arm gone the close is a no-op and the depth stays at 1.
        let mut norm = Normalizer::new("", 0);
        let open = Node::Directive(Directive {
            raw: StrId(0),
            kind: DirectiveKind::WarichuOpen,
        });
        let close = Node::Directive(Directive {
            raw: StrId(0),
            kind: DirectiveKind::WarichuClose,
        });
        norm.track_single_line_break(open, Span::new(0, 0));
        assert_eq!(norm.warichu_depth, 1, "open should increment warichu depth");
        norm.track_single_line_break(close, Span::new(0, 0));
        assert_eq!(
            norm.warichu_depth, 0,
            "close should decrement warichu depth back to zero"
        );
    }

    #[test]
    fn lex_materialises_ruby_resolving_back_to_source_text() {
        let src = "｜青梅《おうめ》";
        let owned = lex(src);

        // The single inline entry resolves back to the ruby base / reading.
        let Some((pos, _)) = owned.registry.iter_kind(Sentinel::Inline).next() else {
            panic!("expected one inline entry");
        };
        let Some(hit) = owned.registry.node_at(NormalizedOffset::new(pos)) else {
            panic!("expected an owned registry hit");
        };
        let NodeRef::Inline(Node::Ruby(r)) = hit else {
            panic!("expected an owned inline ruby, got {hit:?}");
        };
        let base = owned.store.resolve_content_range(r.base);
        let reading = owned.store.resolve_content_range(r.reading);
        let Content::Plain(base_id) = base[0] else {
            panic!("expected a plain ruby base");
        };
        let Content::Plain(reading_id) = reading[0] else {
            panic!("expected a plain ruby reading");
        };
        assert_eq!(owned.store.resolve_str(base_id), "青梅");
        assert_eq!(owned.store.resolve_str(reading_id), "おうめ");
    }

    #[test]
    fn empty_source_round_trips() {
        let out = lex("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
        assert_eq!(out.sanitized_len, 0);
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let out = lex("hello, world");
        assert_eq!(out.normalized, "hello, world");
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn explicit_ruby_lands_in_inline_registry() {
        let out = lex("｜青梅《おうめ》");
        assert_eq!(out.registry.count_kind(Sentinel::Inline), 1);
        let (pos, nr) = out
            .registry
            .iter_kind(Sentinel::Inline)
            .next()
            .expect("one entry");
        assert!(out.normalized.as_bytes()[pos as usize..].starts_with(&[0xEE, 0x80, 0x81]));
        let NodeRef::Inline(node) = nr else {
            panic!("expected NodeRef::Inline, got {nr:?}");
        };
        assert!(matches!(node, Node::Ruby(_)));
    }

    #[test]
    fn page_break_lands_in_block_leaf_registry() {
        let out = lex("text［＃改ページ］more");
        assert_eq!(out.registry.count_kind(Sentinel::BlockLeaf), 1);
        let (_pos, nr) = out
            .registry
            .iter_kind(Sentinel::BlockLeaf)
            .next()
            .expect("one entry");
        let NodeRef::BlockLeaf(node) = nr else {
            panic!("expected NodeRef::BlockLeaf, got {nr:?}");
        };
        assert!(matches!(node, Node::PageBreak));
    }

    #[test]
    fn paired_container_lands_in_open_close_registries() {
        let out = lex("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
        assert_eq!(out.registry.count_kind(Sentinel::BlockOpen), 1);
        assert_eq!(out.registry.count_kind(Sentinel::BlockClose), 1);
        let (_, nr) = out.registry.iter_kind(Sentinel::BlockOpen).next().unwrap();
        let NodeRef::BlockOpen(kind) = nr else {
            panic!("expected NodeRef::BlockOpen, got {nr:?}");
        };
        assert!(matches!(
            kind,
            RegionFormat::Indent(IndentBlock { amount: 2, .. })
        ));
    }

    #[test]
    fn diagnostics_carry_through_to_output() {
        let out = lex("source has \u{E001} reserved sentinel");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn sanitized_len_equals_input_for_plain_text() {
        let input = "plain text\nwith newline";
        let out = lex(input);
        assert_eq!(usize::try_from(out.sanitized_len), Ok(input.len()));
    }

    #[test]
    fn container_kind_indent_amount_preserved() {
        let out = lex("［＃ここから3字下げ］\ntext\n［＃ここで字下げ終わり］");
        let (_, nr) = out.registry.iter_kind(Sentinel::BlockOpen).next().unwrap();
        let NodeRef::BlockOpen(kind) = nr else {
            panic!("expected NodeRef::BlockOpen, got {nr:?}");
        };
        match kind {
            RegionFormat::Indent(IndentBlock { amount, .. }) => assert_eq!(amount, 3),
            other => panic!("expected Indent {{ amount: 3 }}, got {other:?}"),
        }
    }

    #[test]
    fn dense_corpus_paragraph_lands_expected_pieces() {
        let src = "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
                   なる珍しき木が立つ。［＃ここから2字下げ］\n\
                   その下で人々は語らひ、［＃「青空」に傍点］\n\
                   ［＃ここで字下げ終わり］";
        let out = lex(src);
        assert_eq!(out.registry.count_kind(Sentinel::Inline), 3);
        assert_eq!(out.registry.count_kind(Sentinel::BlockLeaf), 0);
        assert_eq!(out.registry.count_kind(Sentinel::BlockOpen), 1);
        assert_eq!(out.registry.count_kind(Sentinel::BlockClose), 1);
        for (pos, _) in out.registry.iter_kind(Sentinel::Inline) {
            assert!(out.registry.node_at(NormalizedOffset::new(pos)).is_some());
        }
    }

    #[test]
    fn block_open_close_padding_is_blank_line_sentinel_blank_line() {
        let src = "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］";
        let out = lex(src);

        let (open_pos, _) = out
            .registry
            .iter_kind(Sentinel::BlockOpen)
            .next()
            .expect("one open entry");
        let (close_pos, _) = out
            .registry
            .iter_kind(Sentinel::BlockClose)
            .next()
            .expect("one close entry");

        let bytes = out.normalized.as_bytes();
        let open_sentinel_bytes = "\u{E003}".as_bytes();
        let close_sentinel_bytes = "\u{E004}".as_bytes();

        assert!(open_pos as usize >= 2);
        assert_eq!(&bytes[(open_pos as usize - 2)..open_pos as usize], b"\n\n");
        let open_after = open_pos as usize + open_sentinel_bytes.len();
        assert_eq!(&bytes[open_pos as usize..open_after], open_sentinel_bytes);
        assert!(open_after + 2 <= bytes.len());
        assert_eq!(&bytes[open_after..open_after + 2], b"\n\n");

        assert!(close_pos as usize >= 2);
        assert_eq!(
            &bytes[(close_pos as usize - 2)..close_pos as usize],
            b"\n\n"
        );
        let close_after = close_pos as usize + close_sentinel_bytes.len();
        assert_eq!(
            &bytes[close_pos as usize..close_after],
            close_sentinel_bytes
        );
        assert!(close_after + 2 <= bytes.len());
        assert_eq!(&bytes[close_after..close_after + 2], b"\n\n");
    }
}
