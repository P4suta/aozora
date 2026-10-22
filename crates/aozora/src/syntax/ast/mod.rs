// Lifetime-free AST handles let cached parses outlive their Document and cross threads (ADR-0036).

mod graft;
mod intern;
mod output;
mod payload;
mod registry;
mod store;

#[cfg(test)]
pub(crate) use intern::StrId;
pub(crate) use output::LexOutput;
pub(crate) use output::RegionOutput;
pub(crate) use output::SanitizedText;
pub(crate) use output::SourceNode;
pub(crate) use payload::{
    AngleQuote, Content, Directive, ForwardFormat, ForwardPayload, Gaiji, GaijiCanonicalOwned,
    Heading, HeadingHint, Illustration, Kaeriten, MarginNote, Node, Ruby, Segment,
    content_ruby_base_class, gaiji_ruby_base_class, node_is_content_segment,
};
pub(crate) use registry::Registry;
pub(crate) use registry::{ContainerPair, NodeRef};
pub(crate) use store::ContentRange;
pub(crate) use store::NodeStore;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{NormalizedOffset, Span};
    use crate::syntax::RubySide;

    #[test]
    fn lex_output_is_send_and_sync() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LexOutput>();
        assert_send_sync::<NodeStore>();
        assert_send_sync::<Registry>();
    }

    #[test]
    fn hand_built_ruby_output_resolves_back() {
        let mut store = NodeStore::new();

        let base_id = store.intern("日本");
        let reading_id = store.intern("にほん");
        let base = store.push_contents(&[Content::Plain(base_id)]);
        let reading = store.push_contents(&[Content::Plain(reading_id)]);

        let ruby = Ruby {
            base,
            reading,
            side: RubySide::Right,
            base_emphasis: None,
        };
        let node = Node::Ruby(ruby);

        let registry = Registry::from_sorted_slice(&[(0u32, NodeRef::Inline(node))]);
        let source_nodes = vec![SourceNode {
            source_span: Span::new(0, 12),
            normalized_offset: NormalizedOffset::new(0),
            node: NodeRef::Inline(node),
        }];

        let out = LexOutput {
            normalized: String::from("\u{E001}"),
            sanitized: String::from("日本").into(),
            source_unchanged: true,
            registry,
            diagnostics: Vec::new(),
            pairs: Vec::new(),
            source_nodes,
            container_pairs: Vec::new(),
            store: store.into(),
        };

        let hit = out
            .registry
            .node_at(NormalizedOffset::new(0))
            .expect("registered inline node at position 0");
        let NodeRef::Inline(Node::Ruby(got)) = hit else {
            panic!("expected an inline ruby node, got {hit:?}");
        };

        let base_run = out.store.resolve_content_range(got.base);
        assert_eq!(base_run.len(), 1, "ruby base is one content entry");
        let Content::Plain(got_base_id) = base_run[0] else {
            panic!("expected a Plain base content, got {:?}", base_run[0]);
        };
        assert_eq!(out.store.resolve_str(got_base_id), "日本", "base text");

        let reading_run = out.store.resolve_content_range(got.reading);
        assert_eq!(reading_run.len(), 1, "ruby reading is one content entry");
        let Content::Plain(got_reading_id) = reading_run[0] else {
            panic!("expected a Plain reading content, got {:?}", reading_run[0]);
        };
        assert_eq!(
            out.store.resolve_str(got_reading_id),
            "にほん",
            "reading text"
        );

        assert_eq!(got.side, RubySide::Right, "ruby side preserved");
    }
}
