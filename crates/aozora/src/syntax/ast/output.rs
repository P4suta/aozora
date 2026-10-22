use core::ops::Deref;
use std::sync::Arc;

use crate::spec::{Diagnostic, NormalizedOffset, PairLink, SourceOffset, Span};

use super::registry::{ContainerPair, NodeRef, Registry};
use super::store::NodeStore;

#[derive(Debug, Clone)]
pub(crate) enum SanitizedText {
    Shared(Arc<str>),
    Owned(String),
}

impl SanitizedText {
    pub(crate) fn shared(text: Arc<str>) -> Self {
        Self::Shared(text)
    }

    pub(crate) fn owned(text: String) -> Self {
        Self::Owned(text)
    }
}

impl Deref for SanitizedText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(text) => text,
            Self::Owned(text) => text,
        }
    }
}

impl AsRef<str> for SanitizedText {
    fn as_ref(&self) -> &str {
        self
    }
}

impl PartialEq for SanitizedText {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for SanitizedText {}

impl From<String> for SanitizedText {
    fn from(text: String) -> Self {
        Self::Owned(text)
    }
}

impl From<Arc<str>> for SanitizedText {
    fn from(text: Arc<str>) -> Self {
        Self::Shared(text)
    }
}

impl From<&str> for SanitizedText {
    fn from(text: &str) -> Self {
        Self::Owned(text.to_owned())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceNode {
    pub source_span: Span,
    pub(crate) normalized_offset: NormalizedOffset,
    pub node: NodeRef,
}

#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct LexOutput {
    pub(crate) normalized: String,
    pub(crate) sanitized: SanitizedText,
    pub(crate) source_unchanged: bool,
    pub(crate) registry: Registry,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) pairs: Vec<PairLink>,
    pub(crate) source_nodes: Vec<SourceNode>,
    pub(crate) container_pairs: Vec<ContainerPair>,
    pub(crate) store: Arc<NodeStore>,
}

#[derive(Debug)]
pub(crate) struct RegionOutput {
    pub(crate) normalized: String,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) pairs: Vec<PairLink>,
    pub(crate) source_nodes: Vec<SourceNode>,
    pub(crate) container_pairs: Vec<ContainerPair>,
    pub(crate) store: NodeStore,
}

impl LexOutput {
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "constructs the non_exhaustive LexOutput from its complete already-owned field set; a parameter object would only restate the field set"
    )]
    pub(crate) fn new(
        normalized: String,
        sanitized: impl Into<SanitizedText>,
        source_unchanged: bool,
        registry: Registry,
        mut diagnostics: Vec<Diagnostic>,
        pairs: Vec<PairLink>,
        source_nodes: Vec<SourceNode>,
        container_pairs: Vec<ContainerPair>,
        store: impl Into<Arc<NodeStore>>,
    ) -> Self {
        diagnostics.sort_unstable_by(|left, right| {
            let left_span = left.span();
            let right_span = right.span();
            left_span
                .start
                .cmp(&right_span.start)
                .then_with(|| left_span.end.cmp(&right_span.end))
                .then_with(|| left.code().cmp(right.code()))
        });
        Self {
            normalized,
            sanitized: sanitized.into(),
            source_unchanged,
            registry,
            diagnostics,
            pairs,
            source_nodes,
            container_pairs,
            store: store.into(),
        }
    }

    #[must_use]
    pub(crate) fn node_at_source(&self, src_off: SourceOffset) -> Option<&SourceNode> {
        let raw = src_off.get();
        let idx = self
            .source_nodes
            .partition_point(|entry| entry.source_span.start <= raw);
        if idx == 0 {
            return None;
        }
        let candidate = &self.source_nodes[idx - 1];
        (raw < candidate.source_span.end).then_some(candidate)
    }
}

const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LexOutput>();
};

#[cfg(test)]
mod tests {
    use super::super::payload::Node;
    use super::*;

    fn output_with(source_nodes: Vec<SourceNode>) -> LexOutput {
        LexOutput::new(
            String::new(),
            String::new(),
            true,
            Registry::empty(),
            Vec::new(),
            Vec::new(),
            source_nodes,
            Vec::new(),
            NodeStore::new(),
        )
    }

    #[test]
    fn node_at_source_covers_ranges_and_gaps() {
        let sn = vec![
            SourceNode {
                source_span: Span::new(2, 5),
                normalized_offset: NormalizedOffset::new(0),
                node: NodeRef::Inline(Node::PageBreak),
            },
            SourceNode {
                source_span: Span::new(10, 20),
                normalized_offset: NormalizedOffset::new(3),
                node: NodeRef::Inline(Node::BodyEnd),
            },
        ];
        let out = output_with(sn);

        let page = Some(NodeRef::Inline(Node::PageBreak));
        let body = Some(NodeRef::Inline(Node::BodyEnd));
        let cases: &[(u32, Option<NodeRef>)] = &[
            (0, None),
            (1, None),
            (2, page),
            (3, page),
            (4, page),
            (5, None),
            (7, None),
            (10, body),
            (15, body),
            (19, body),
            (20, None),
            (25, None),
        ];
        for &(raw, expected) in cases {
            let got = out.node_at_source(SourceOffset::new(raw)).map(|s| s.node);
            assert_eq!(got, expected, "offset {raw} should map to {expected:?}");
        }
    }

    #[test]
    fn node_at_source_on_empty_table_is_none() {
        let out = output_with(Vec::new());
        assert!(out.node_at_source(SourceOffset::new(0)).is_none());
        assert!(out.node_at_source(SourceOffset::new(42)).is_none());
    }
}
