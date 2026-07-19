//! CST build path: project `aozora::Snapshot` into a rowan
//! `SyntaxNode` tree using the public source-node + container-pair
//! surface only.

use rowan::GreenNodeBuilder;

use crate::cst::kind::{SyntaxKind, SyntaxNode};
use crate::syntax::ast::{NodeRef, SourceNode};

/// Build the CST from the sanitized source + classified
/// source-node table.
///
/// `sanitized_source` MUST be the output of the pipeline's `sanitize`
/// stage over the original source — the `source_nodes` table's
/// `source_span` coordinates live in
/// sanitized-source bytes. For typical inputs (no BOM, LF only, no
/// long decorative rule lines, no `〔…〕` accent spans) sanitized
/// equals the original source byte-for-byte; documents that exercise
/// any of those preprocessing rules will diverge.
///
/// Lossless: the leaf-text concatenation equals `sanitized_source`.
/// (Note that this is the *sanitized* contract, not the original
/// source; the meta crate's `aozora::cst::from_tree` runs the
/// sanitize pass internally and exposes the same property.)
#[must_use]
pub(crate) fn build_cst(sanitized_source: &str, source_nodes: &[SourceNode]) -> SyntaxNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(rowan::SyntaxKind(SyntaxKind::Document as u16));

    let mut walker = Walker::new(&mut builder, sanitized_source, source_nodes);
    walker.run();

    builder.finish_node();
    let green = builder.finish();
    SyntaxNode::new_root(green)
}

struct Walker<'a, 'src> {
    builder: &'a mut GreenNodeBuilder<'static>,
    source: &'src str,
    nodes: &'src [SourceNode],
    cursor: usize,
    /// Container nesting depth. Each `BlockOpen` opens a `Container`
    /// node; the matching `BlockClose` finishes it. We do not track
    /// the kind here — the CST nests structurally, the AST keeps the
    /// rich variant detail.
    open_containers: usize,
}

impl<'a, 'src> Walker<'a, 'src> {
    fn new(
        builder: &'a mut GreenNodeBuilder<'static>,
        source: &'src str,
        nodes: &'src [SourceNode],
    ) -> Self {
        Self {
            builder,
            source,
            nodes,
            cursor: 0,
            open_containers: 0,
        }
    }

    fn run(&mut self) {
        for entry in self.nodes {
            let span_start = entry.source_span.start as usize;
            let span_end = entry.source_span.end as usize;
            self.flush_plain(span_start);
            self.dispatch_node(entry, span_start, span_end);
            self.cursor = span_end;
        }
        self.flush_plain(self.source.len());
        // Close any containers the source left open (unclosed
        // diagnostics) so the document tree is well-formed.
        while self.open_containers > 0 {
            self.builder.finish_node();
            self.open_containers -= 1;
        }
    }

    fn flush_plain(&mut self, end: usize) {
        if end <= self.cursor {
            return;
        }
        let chunk = &self.source[self.cursor..end];
        self.token(SyntaxKind::Plain, chunk);
        self.cursor = end;
    }

    fn dispatch_node(&mut self, entry: &SourceNode, span_start: usize, span_end: usize) {
        let span_text = &self.source[span_start..span_end];
        match entry.node {
            NodeRef::Inline(_) | NodeRef::BlockLeaf(_) => {
                self.start_node(SyntaxKind::Construct);
                self.token(SyntaxKind::ConstructText, span_text);
                self.builder.finish_node();
            }
            NodeRef::BlockOpen(_) => {
                self.start_node(SyntaxKind::Container);
                self.token(SyntaxKind::ContainerOpen, span_text);
                self.open_containers += 1;
            }
            NodeRef::BlockClose(_) => {
                self.token(SyntaxKind::ContainerClose, span_text);
                if self.open_containers > 0 {
                    self.builder.finish_node();
                    self.open_containers -= 1;
                }
                // A close without a matching open arrives as a
                // standalone token at the document level — we
                // already wrote its text via `token` above, so the
                // bytes are preserved and the lossless invariant
                // still holds.
            }
        }
    }

    fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(rowan::SyntaxKind(kind as u16));
    }

    fn token(&mut self, kind: SyntaxKind, text: &str) {
        self.builder.token(rowan::SyntaxKind(kind as u16), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use crate::pipeline::lexer::sanitize;

    fn lossless(src: &str) {
        let sanitized = sanitize(src);
        let doc = Document::new(src);
        let tree = doc.snapshot();
        let cst = build_cst(&sanitized.text, tree.source_nodes());
        let mut reconstructed = String::new();
        for step in cst.preorder_with_tokens() {
            if let rowan::WalkEvent::Enter(rowan::NodeOrToken::Token(t)) = step {
                reconstructed.push_str(t.text());
            }
        }
        assert_eq!(
            reconstructed,
            sanitized.text.as_ref(),
            "lossless invariant: leaves != sanitized source"
        );
    }

    #[test]
    fn empty_input() {
        lossless("");
    }

    #[test]
    fn plain_text_is_one_token() {
        lossless("Hello, world.");
    }

    #[test]
    fn ruby_round_trips_through_cst() {
        lossless("｜青梅《おうめ》");
    }

    #[test]
    fn container_round_trips_through_cst() {
        lossless(
            "前置き\n\
             ［＃ここから2字下げ］\n\
             本文\n\
             ［＃ここで字下げ終わり］\n\
             後書き",
        );
    }

    #[test]
    fn nested_containers_round_trip() {
        lossless(
            "［＃ここから2字下げ］\n\
             外\n\
             ［＃ここから3字下げ］\n\
             内\n\
             ［＃ここで字下げ終わり］\n\
             外戻り\n\
             ［＃ここで字下げ終わり］",
        );
    }

    #[test]
    fn unclosed_container_still_round_trips() {
        // Unclosed open means the parser's container stack stays
        // open at EOF. Our walker closes pending Container nodes
        // defensively so the tree is well-formed; the lossless
        // property holds.
        lossless("［＃ここから2字下げ］\n途中で打ち切り");
    }

    fn build(src: &str) -> SyntaxNode {
        let sanitized = sanitize(src);
        let doc = Document::new(src);
        let tree = doc.snapshot();
        build_cst(&sanitized.text, tree.source_nodes())
    }

    fn count_kind(cst: &SyntaxNode, kind: SyntaxKind) -> usize {
        cst.descendants_with_tokens()
            .filter(|el| el.kind() == kind)
            .count()
    }

    #[test]
    fn inline_construct_emits_construct_node() {
        // The `Inline`/`BlockLeaf` arm wraps a classified span in a
        // `Construct` node carrying one `ConstructText` token.
        // Deleting the arm degrades the ruby to a bare `Plain` token
        // (bytes preserved, structure lost), so pin the node + token.
        let cst = build("｜青梅《おうめ》");
        assert_eq!(count_kind(&cst, SyntaxKind::Construct), 1);
        assert_eq!(count_kind(&cst, SyntaxKind::ConstructText), 1);
        assert_eq!(count_kind(&cst, SyntaxKind::Container), 0);
    }

    #[test]
    fn container_structure_and_close_scope() {
        let cst = build(
            "前置き\n\
             ［＃ここから2字下げ］\n\
             本文\n\
             ［＃ここで字下げ終わり］\n\
             後書き",
        );
        // `BlockOpen` arm -> exactly one `Container` node + one
        // `ContainerOpen` token. Deleting the arm drops both.
        assert_eq!(count_kind(&cst, SyntaxKind::Container), 1);
        assert_eq!(count_kind(&cst, SyntaxKind::ContainerOpen), 1);
        // `BlockClose` arm -> exactly one `ContainerClose` token.
        // Deleting the arm degrades it to a `Plain` token.
        assert_eq!(count_kind(&cst, SyntaxKind::ContainerClose), 1);

        // Line 106 `open_containers > 0`: on a matched close the
        // `Container` finishes AT the close boundary, so the trailing
        // "後書き" run lands at the `Document` root as a sibling of
        // the closed `Container`. Under the `>`->`<` mutation the
        // finish never fires (`usize < 0` is unsatisfiable), the
        // container stays open, and the trailing text is nested
        // inside it instead. Pin the trailing run as a direct root
        // child.
        let mut root_plain = Vec::new();
        for el in cst.children_with_tokens() {
            let Some(t) = el.as_token() else { continue };
            if t.kind() == SyntaxKind::Plain {
                root_plain.push(t.text().to_owned());
            }
        }
        assert!(
            root_plain.iter().any(|t| t.contains("後書き")),
            "trailing text must be a Document-root sibling, not nested \
             in the closed Container: {root_plain:?}"
        );
    }

    #[test]
    fn document_root_is_document_kind() {
        let sanitized = sanitize("hi");
        let doc = Document::new("hi");
        let tree = doc.snapshot();
        let cst = build_cst(&sanitized.text, tree.source_nodes());
        assert_eq!(cst.kind(), SyntaxKind::Document);
    }
}
