use super::intern::StrId;
use super::payload::{
    AngleQuote, Content, Directive, ForwardFormat, ForwardPayload, Gaiji, GaijiCanonicalOwned,
    Heading, HeadingHint, Illustration, Kaeriten, MarginNote, Node, Ruby, Segment,
};
use super::registry::NodeRef;
use super::store::{ContentRange, NodeStore, SegRange};

impl NodeStore {
    pub(crate) fn graft_node_ref(&mut self, source: &Self, node: NodeRef) -> NodeRef {
        match node {
            NodeRef::Inline(node) => NodeRef::Inline(self.graft_node(source, node)),
            NodeRef::BlockLeaf(node) => NodeRef::BlockLeaf(self.graft_node(source, node)),
            NodeRef::BlockOpen(format) => NodeRef::BlockOpen(format),
            NodeRef::BlockClose(close) => NodeRef::BlockClose(close),
        }
    }

    fn graft_node(&mut self, source: &Self, node: Node) -> Node {
        match node {
            Node::Ruby(ruby) => Node::Ruby(Ruby {
                base: graft_content_range(source, self, ruby.base),
                reading: graft_content_range(source, self, ruby.reading),
                side: ruby.side,
                base_emphasis: ruby.base_emphasis,
            }),
            Node::Format(format) => Node::Format(ForwardFormat {
                attr: format.attr,
                target: graft_content_range(source, self, format.target),
                origin: format.origin,
                payload: match format.payload {
                    ForwardPayload::AccentBody(id) => {
                        ForwardPayload::AccentBody(graft_str(source, self, id))
                    }
                    payload => payload,
                },
            }),
            Node::Gaiji(gaiji) => Node::Gaiji(graft_gaiji(source, self, gaiji)),
            Node::Line(format) => Node::Line(format),
            Node::PageBreak => Node::PageBreak,
            Node::SectionBreak(kind) => Node::SectionBreak(kind),
            Node::BodyEnd => Node::BodyEnd,
            Node::ForcedBreak => Node::ForcedBreak,
            Node::Heading(heading) => Node::Heading(Heading {
                kind: heading.kind,
                style: heading.style,
                text: graft_content_range(source, self, heading.text),
            }),
            Node::HeadingHint(hint) => Node::HeadingHint(HeadingHint {
                level: hint.level,
                style: hint.style,
                target: graft_str(source, self, hint.target),
                self_contained: hint.self_contained,
            }),
            Node::Illustration(illustration) => Node::Illustration(Illustration {
                file: graft_str(source, self, illustration.file),
                number: illustration.number.map(|id| graft_str(source, self, id)),
                dimensions: illustration
                    .dimensions
                    .map(|id| graft_str(source, self, id)),
                caption: illustration
                    .caption
                    .map(|content| graft_content(source, self, content)),
                description: illustration
                    .description
                    .map(|id| graft_str(source, self, id)),
            }),
            Node::Kaeriten(kaeriten) => Node::Kaeriten(Kaeriten {
                mark: graft_str(source, self, kaeriten.mark),
            }),
            Node::Directive(directive) => Node::Directive(Directive {
                raw: graft_str(source, self, directive.raw),
                kind: directive.kind,
            }),
            Node::AngleQuote(quote) => Node::AngleQuote(AngleQuote {
                content: graft_content_range(source, self, quote.content),
            }),
            Node::MarginNote(margin) => Node::MarginNote(MarginNote {
                kind: margin.kind,
                base: graft_content_range(source, self, margin.base),
                note: graft_content_range(source, self, margin.note),
            }),
        }
    }
}

fn graft_str(source: &NodeStore, target: &mut NodeStore, id: StrId) -> StrId {
    target.intern(source.resolve_str(id))
}

fn graft_content_range(
    source: &NodeStore,
    target: &mut NodeStore,
    range: ContentRange,
) -> ContentRange {
    let contents = source.resolve_content_range(range).to_vec();
    let contents = contents
        .into_iter()
        .map(|content| graft_content(source, target, content))
        .collect::<Vec<_>>();
    target.push_contents(&contents)
}

fn graft_content(source: &NodeStore, target: &mut NodeStore, content: Content) -> Content {
    match content {
        Content::Plain(id) => Content::Plain(graft_str(source, target, id)),
        Content::Segments(range) => Content::Segments(graft_segment_range(source, target, range)),
    }
}

fn graft_segment_range(source: &NodeStore, target: &mut NodeStore, range: SegRange) -> SegRange {
    let segments = source.resolve_seg_range(range).to_vec();
    let segments = segments
        .into_iter()
        .map(|segment| graft_segment(source, target, segment))
        .collect::<Vec<_>>();
    target.push_segments(&segments)
}

fn graft_segment(source: &NodeStore, target: &mut NodeStore, segment: Segment) -> Segment {
    match segment {
        Segment::Text(id) => Segment::Text(graft_str(source, target, id)),
        Segment::Gaiji(gaiji) => Segment::Gaiji(graft_gaiji(source, target, gaiji)),
        Segment::Directive(directive) => Segment::Directive(Directive {
            raw: graft_str(source, target, directive.raw),
            kind: directive.kind,
        }),
    }
}

fn graft_gaiji(source: &NodeStore, target: &mut NodeStore, gaiji: Gaiji) -> Gaiji {
    let canonical = match gaiji.canonical {
        GaijiCanonicalOwned::MenKuTen(value) => GaijiCanonicalOwned::MenKuTen(value),
        GaijiCanonicalOwned::Unicode(value) => GaijiCanonicalOwned::Unicode(value),
        GaijiCanonicalOwned::Unresolved { mencode } => GaijiCanonicalOwned::Unresolved {
            mencode: mencode.map(|id| graft_str(source, target, id)),
        },
    };
    Gaiji {
        hint: graft_str(source, target, gaiji.hint),
        canonical,
        mencode_separator: gaiji.mencode_separator,
        standalone: gaiji.standalone,
    }
}
