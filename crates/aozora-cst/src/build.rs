//! CST build path: project [`aozora::AozoraTree`] into a rowan
//! `SyntaxNode` tree using the public source-node + container-pair
//! surface only.

use aozora_pipeline::{NodeRef, SourceNode};
use rowan::GreenNodeBuilder;

use crate::kind::{SyntaxKind, SyntaxNode};

/// Build the CST from the sanitized source + classified
/// source-node table.
///
/// `sanitized_source` MUST be the output of
/// [`aozora_pipeline::lexer::sanitize`] over the original source —
/// the `source_nodes` table's `source_span` coordinates live in
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
pub fn build_cst(sanitized_source: &str, source_nodes: &[SourceNode<'_>]) -> SyntaxNode {
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
    nodes: &'src [SourceNode<'src>],
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
        nodes: &'src [SourceNode<'src>],
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

    fn dispatch_node(&mut self, entry: &SourceNode<'src>, span_start: usize, span_end: usize) {
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
            // `NodeRef` is `#[non_exhaustive]`; future variants
            // surface as plain bytes until the projection adds
            // dedicated handling.
            _ => self.token(SyntaxKind::Plain, span_text),
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
    use aozora::Document;
    use aozora_pipeline::lexer::sanitize;

    fn lossless(src: &str) {
        let sanitized = sanitize(src);
        let doc = Document::new(src);
        let tree = doc.parse();
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

    #[test]
    fn document_root_is_document_kind() {
        let sanitized = sanitize("hi");
        let doc = Document::new("hi");
        let tree = doc.parse();
        let cst = build_cst(&sanitized.text, tree.source_nodes());
        assert_eq!(cst.kind(), SyntaxKind::Document);
    }
}
