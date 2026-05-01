//! Source-driven projection from an [`AozoraTree`] to a
//! [`pandoc_ast::Pandoc`] document.
//!
//! Walks the source linearly, slicing it into spans by
//! [`AozoraTree::source_nodes`]. Plain runs flow into Pandoc inlines
//! verbatim (with `\n\n` paragraph splits and single `\n` →
//! `SoftBreak`). Each classified node lifts to a Pandoc inline /
//! block construct as documented in [`crate`].

use aozora::{
    AlignEnd, Annotation, AnnotationKind, AozoraHeading, AozoraHeadingKind, AozoraTree, Bouten,
    BoutenKind, BoutenPosition, ContainerKind, DoubleRuby, Gaiji, HeadingHint, Indent, Kaeriten,
    NodeRef, Ruby, Sashie, SectionKind, Segment, SourceNode, Span, TateChuYoko, Warichu,
    syntax::borrowed::{AozoraNode, Content},
};
use pandoc_ast::{Attr, Block, Inline, Pandoc};

use crate::AOZORA_CLASS_PREFIX;

/// Lift a parsed [`AozoraTree`] to a [`pandoc_ast::Pandoc`] document.
///
/// See the crate-level docs for the projection rules.
#[must_use]
pub fn to_pandoc(tree: &AozoraTree<'_>) -> Pandoc {
    let mut converter = Converter::new(tree.source(), tree.source_nodes());
    converter.run();
    Pandoc {
        meta: pandoc_ast::Map::new(),
        blocks: converter.blocks,
        // Pandoc 3.x carries this version; 1.23 is what `pandoc -t
        // json` emits as of late 2025. Older Pandoc readers
        // back-compat all the way to 1.20 (the minimum `pandoc_ast`
        // accepts).
        pandoc_api_version: vec![1, 23],
    }
}

// ---------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------

/// Block-context frame. The implicit outermost frame is the document
/// root; each container open pushes a new frame, container close
/// pops and wraps the accumulated blocks in a Pandoc Div.
struct Frame {
    /// Closed blocks accumulated under this container (Para nodes
    /// emitted as inline runs flush, plus block-leaf children).
    blocks: Vec<Block>,
    /// In-flight inline accumulator for the current paragraph.
    /// `None` means "no open paragraph" (after a flush).
    inlines: Option<Vec<Inline>>,
    /// Container kind for the wrapping `Div` (if any). The root
    /// frame carries `None`.
    container: Option<ContainerKind>,
}

impl Frame {
    fn root() -> Self {
        Self {
            blocks: Vec::new(),
            inlines: None,
            container: None,
        }
    }

    fn child(kind: ContainerKind) -> Self {
        Self {
            blocks: Vec::new(),
            inlines: None,
            container: Some(kind),
        }
    }

    fn paragraph(&mut self) -> &mut Vec<Inline> {
        self.inlines.get_or_insert_with(Vec::new)
    }

    /// Close the in-flight paragraph (if any). Trailing whitespace
    /// is trimmed by Pandoc's writer; we keep the Inline list as-is.
    fn flush_paragraph(&mut self) {
        if let Some(inlines) = self.inlines.take()
            && !inlines.is_empty()
        {
            self.blocks.push(Block::Para(inlines));
        }
    }
}

struct Converter<'src> {
    source: &'src str,
    nodes: &'src [SourceNode<'src>],
    /// Stack of block frames. Always non-empty; the bottom frame is
    /// the document root.
    stack: Vec<Frame>,
    /// Cursor into `source` (byte offset).
    cursor: usize,
    /// Final document blocks, populated by [`Converter::run`] from
    /// the root frame on completion.
    blocks: Vec<Block>,
}

impl<'src> Converter<'src> {
    fn new(source: &'src str, nodes: &'src [SourceNode<'src>]) -> Self {
        Self {
            source,
            nodes,
            stack: vec![Frame::root()],
            cursor: 0,
            blocks: Vec::new(),
        }
    }

    fn run(&mut self) {
        for entry in self.nodes {
            // Plain run between previous cursor and this node.
            self.flush_plain(entry.source_span.start as usize);
            self.dispatch_node(entry);
            self.cursor = entry.source_span.end as usize;
        }
        self.flush_plain(self.source.len());
        // Pop any unclosed containers (defensive — well-formed input
        // never reaches here, but unclosed-bracket diagnostics let
        // the document still parse).
        while self.stack.len() > 1 {
            let frame = self.stack.pop().expect("non-empty stack");
            self.close_frame(frame);
        }
        let mut root = self.stack.pop().expect("root frame");
        root.flush_paragraph();
        self.blocks = root.blocks;
    }

    /// Push the slice of plain text between `cursor` and `end` into
    /// the current paragraph. `\n\n` boundaries close the paragraph
    /// and open a fresh one; single `\n` becomes a `SoftBreak`.
    fn flush_plain(&mut self, end: usize) {
        if end <= self.cursor {
            return;
        }
        let chunk = &self.source[self.cursor..end];
        for (idx, line) in chunk.split('\n').enumerate() {
            if idx > 0 {
                // Blank line (preceded by another `\n`) closes the
                // paragraph; non-blank line emits a soft break.
                if line.is_empty() {
                    self.current_frame_mut().flush_paragraph();
                } else {
                    self.current_frame_mut().paragraph().push(Inline::SoftBreak);
                }
            }
            if !line.is_empty() {
                self.current_frame_mut()
                    .paragraph()
                    .push(Inline::Str(line.to_owned()));
            }
        }
        self.cursor = end;
    }

    fn current_frame_mut(&mut self) -> &mut Frame {
        self.stack.last_mut().expect("stack always non-empty")
    }

    fn dispatch_node(&mut self, entry: &SourceNode<'src>) {
        match entry.node {
            NodeRef::Inline(node) => self.dispatch_inline_node(node, entry.source_span),
            NodeRef::BlockLeaf(node) => self.dispatch_block_leaf(node, entry.source_span),
            NodeRef::BlockOpen(kind) => self.open_container(kind),
            NodeRef::BlockClose(_) => self.close_container(),
            // `NodeRef` is `#[non_exhaustive]`; treat unknown
            // variants as pass-through plain text.
            _ => {}
        }
    }

    fn dispatch_inline_node(&mut self, node: AozoraNode<'src>, _span: Span) {
        use AozoraNode as N;
        let inline = match node {
            N::Ruby(r) => ruby_inline(r),
            N::Bouten(b) => bouten_inline(b),
            N::TateChuYoko(t) => tate_chu_yoko_inline(t),
            N::Gaiji(g) => gaiji_inline(*g),
            N::Indent(i) => indent_inline(i),
            N::AlignEnd(a) => align_end_inline(a),
            N::Warichu(w) => warichu_inline(w),
            N::Keigakomi(_) => keigakomi_inline(),
            N::Annotation(a) => annotation_inline(*a),
            N::Kaeriten(k) => kaeriten_inline(*k),
            N::DoubleRuby(d) => double_ruby_inline(*d),
            N::HeadingHint(h) => heading_hint_inline(*h),
            // Block-leaf variants slip through here only if the
            // pipeline misclassified them; render as fallback span.
            other => Inline::Span(plain_attr(), vec![Inline::Str(format!("{other:?}"))]),
        };
        self.current_frame_mut().paragraph().push(inline);
    }

    fn dispatch_block_leaf(&mut self, node: AozoraNode<'src>, _span: Span) {
        use AozoraNode as N;
        // Block-leaf nodes close any in-flight paragraph and emit a
        // standalone block.
        self.current_frame_mut().flush_paragraph();
        let block = match node {
            N::PageBreak => Block::HorizontalRule,
            N::SectionBreak(k) => section_break_block(k),
            N::AozoraHeading(h) => aozora_heading_block(*h),
            N::Sashie(s) => sashie_block(*s),
            // Inline-typed variants here would mean a pipeline
            // misclassification; emit them inside a singleton Para
            // so the document stays renderable.
            other => Block::Para(vec![Inline::Span(
                plain_attr(),
                vec![Inline::Str(format!("{other:?}"))],
            )]),
        };
        self.current_frame_mut().blocks.push(block);
    }

    fn open_container(&mut self, kind: ContainerKind) {
        // A new container starts a new block context; flush any
        // in-flight paragraph in the current frame first.
        self.current_frame_mut().flush_paragraph();
        self.stack.push(Frame::child(kind));
    }

    fn close_container(&mut self) {
        let frame = self.stack.pop().expect("close without matching open");
        self.close_frame(frame);
    }

    fn close_frame(&mut self, mut frame: Frame) {
        frame.flush_paragraph();
        if let Some(kind) = frame.container {
            let div = Block::Div(container_attr(kind), frame.blocks);
            self.current_frame_mut().blocks.push(div);
        } else {
            // Closing the root frame is handled in `run` — getting
            // here means a stack-balance bug.
            self.current_frame_mut().blocks.extend(frame.blocks);
        }
    }
}

// ---------------------------------------------------------------------
// Per-variant inline / block builders
// ---------------------------------------------------------------------

/// Empty `Attr` used for plain inline strings that don't need a
/// class but still need to be wrapped in a `Span` for structural
/// reasons.
fn plain_attr() -> Attr {
    (String::new(), Vec::new(), Vec::new())
}

fn class_attr(class: &str) -> Attr {
    (
        String::new(),
        vec![format!("{AOZORA_CLASS_PREFIX}{class}")],
        Vec::new(),
    )
}

fn class_attr_kv(class: &str, kvs: Vec<(String, String)>) -> Attr {
    (
        String::new(),
        vec![format!("{AOZORA_CLASS_PREFIX}{class}")],
        kvs,
    )
}

fn content_to_inlines(content: Content<'_>) -> Vec<Inline> {
    let mut buf = Vec::new();
    push_content_inlines(content, &mut buf);
    buf
}

fn push_content_inlines(content: Content<'_>, buf: &mut Vec<Inline>) {
    for seg in content {
        match seg {
            Segment::Text(s) => buf.push(Inline::Str(s.to_owned())),
            Segment::Gaiji(g) => buf.push(gaiji_inline(*g)),
            Segment::Annotation(a) => buf.push(annotation_inline(*a)),
            // `Segment` is `#[non_exhaustive]`; future segment kinds
            // get a placeholder until projection logic is added.
            _ => buf.push(Inline::Str(String::new())),
        }
    }
}

fn ruby_inline(r: &Ruby<'_>) -> Inline {
    let base_inlines = content_to_inlines(r.base.get());
    let reading_inlines = content_to_inlines(r.reading.get());
    let inner = vec![
        Inline::Span(class_attr("ruby-base"), base_inlines),
        Inline::Span(class_attr("ruby-reading"), reading_inlines),
    ];
    Inline::Span(
        class_attr_kv(
            "ruby",
            vec![(
                "delim".to_owned(),
                if r.delim_explicit {
                    "explicit".to_owned()
                } else {
                    "implicit".to_owned()
                },
            )],
        ),
        inner,
    )
}

fn bouten_inline(b: &Bouten<'_>) -> Inline {
    let attr = class_attr_kv(
        "bouten",
        vec![
            ("kind".to_owned(), bouten_kind_slug(b.kind).to_owned()),
            (
                "position".to_owned(),
                bouten_position_slug(b.position).to_owned(),
            ),
        ],
    );
    Inline::Span(attr, content_to_inlines(b.target.get()))
}

fn bouten_kind_slug(k: BoutenKind) -> &'static str {
    match k {
        BoutenKind::Goma => "goma",
        BoutenKind::WhiteSesame => "white-sesame",
        BoutenKind::Circle => "circle",
        BoutenKind::WhiteCircle => "white-circle",
        BoutenKind::DoubleCircle => "double-circle",
        BoutenKind::Janome => "janome",
        BoutenKind::Cross => "cross",
        BoutenKind::WhiteTriangle => "white-triangle",
        BoutenKind::WavyLine => "wavy-line",
        BoutenKind::UnderLine => "underline",
        BoutenKind::DoubleUnderLine => "double-underline",
        _ => "unknown",
    }
}

fn bouten_position_slug(p: BoutenPosition) -> &'static str {
    match p {
        BoutenPosition::Right => "right",
        BoutenPosition::Left => "left",
        _ => "unknown",
    }
}

fn tate_chu_yoko_inline(t: &TateChuYoko<'_>) -> Inline {
    Inline::Span(
        class_attr("tate-chu-yoko"),
        content_to_inlines(t.text.get()),
    )
}

fn gaiji_inline(g: Gaiji<'_>) -> Inline {
    let mut kvs = vec![("description".to_owned(), g.description.to_owned())];
    if let Some(mencode) = g.mencode {
        kvs.push(("mencode".to_owned(), mencode.to_owned()));
    }
    let inner = g.ucs.map_or_else(
        || vec![Inline::Str("〓".to_owned())],
        |resolved| vec![Inline::Str(format!("{resolved:?}"))],
    );
    Inline::Span(class_attr_kv("gaiji", kvs), inner)
}

fn indent_inline(i: Indent) -> Inline {
    Inline::Span(
        class_attr_kv("indent", vec![("amount".to_owned(), i.amount.to_string())]),
        Vec::new(),
    )
}

fn align_end_inline(a: AlignEnd) -> Inline {
    Inline::Span(
        class_attr_kv(
            "align-end",
            vec![("offset".to_owned(), a.offset.to_string())],
        ),
        Vec::new(),
    )
}

fn warichu_inline(w: &Warichu<'_>) -> Inline {
    let upper = Inline::Span(class_attr("warichu-upper"), content_to_inlines(w.upper));
    let lower = Inline::Span(class_attr("warichu-lower"), content_to_inlines(w.lower));
    Inline::Span(class_attr("warichu"), vec![upper, lower])
}

fn keigakomi_inline() -> Inline {
    Inline::Span(class_attr("keigakomi"), Vec::new())
}

fn annotation_inline(a: Annotation<'_>) -> Inline {
    Inline::Span(
        class_attr_kv(
            "annotation",
            vec![
                ("kind".to_owned(), annotation_kind_slug(a.kind).to_owned()),
                ("raw".to_owned(), a.raw.as_str().to_owned()),
            ],
        ),
        Vec::new(),
    )
}

fn annotation_kind_slug(k: AnnotationKind) -> &'static str {
    match k {
        AnnotationKind::Unknown => "unknown",
        AnnotationKind::AsIs => "as-is",
        AnnotationKind::TextualNote => "textual-note",
        AnnotationKind::InvalidRubySpan => "invalid-ruby-span",
        _ => "other",
    }
}

fn kaeriten_inline(k: Kaeriten<'_>) -> Inline {
    Inline::Span(
        class_attr_kv(
            "kaeriten",
            vec![("mark".to_owned(), k.mark.as_str().to_owned())],
        ),
        Vec::new(),
    )
}

fn double_ruby_inline(d: DoubleRuby<'_>) -> Inline {
    Inline::Span(
        class_attr("double-ruby"),
        content_to_inlines(d.content.get()),
    )
}

fn heading_hint_inline(h: HeadingHint<'_>) -> Inline {
    Inline::Span(
        class_attr_kv(
            "heading-hint",
            vec![
                ("level".to_owned(), h.level.to_string()),
                ("target".to_owned(), h.target.as_str().to_owned()),
            ],
        ),
        Vec::new(),
    )
}

fn section_break_block(k: SectionKind) -> Block {
    let slug = match k {
        SectionKind::Choho => "choho",
        SectionKind::Dan => "dan",
        SectionKind::Spread => "spread",
        _ => "other",
    };
    Block::Div(
        (
            String::new(),
            vec![
                format!("{AOZORA_CLASS_PREFIX}section-break"),
                format!("{AOZORA_CLASS_PREFIX}section-break-{slug}"),
            ],
            Vec::new(),
        ),
        Vec::new(),
    )
}

fn aozora_heading_block(h: AozoraHeading<'_>) -> Block {
    let level: i64 = match h.kind {
        AozoraHeadingKind::Window => 2,
        AozoraHeadingKind::Sub => 3,
        _ => 4,
    };
    Block::Header(
        level,
        class_attr_kv(
            "heading",
            vec![("kind".to_owned(), heading_kind_slug(h.kind).to_owned())],
        ),
        content_to_inlines(h.text.get()),
    )
}

fn heading_kind_slug(k: AozoraHeadingKind) -> &'static str {
    match k {
        AozoraHeadingKind::Window => "window",
        AozoraHeadingKind::Sub => "sub",
        _ => "other",
    }
}

fn sashie_block(s: Sashie<'_>) -> Block {
    let alt = s.caption.map(content_to_inlines).unwrap_or_default();
    let target = (s.file.as_str().to_owned(), String::new());
    Block::Para(vec![Inline::Image(class_attr("sashie"), alt, target)])
}

fn container_attr(kind: ContainerKind) -> Attr {
    let (slug, kvs): (&str, Vec<(String, String)>) = match kind {
        ContainerKind::Indent { amount } => (
            "container-indent",
            vec![("amount".to_owned(), amount.to_string())],
        ),
        ContainerKind::Warichu => ("container-warichu", Vec::new()),
        ContainerKind::Keigakomi => ("container-keigakomi", Vec::new()),
        ContainerKind::AlignEnd { offset } => (
            "container-align-end",
            vec![("offset".to_owned(), offset.to_string())],
        ),
        _ => ("container-unknown", Vec::new()),
    };
    (
        String::new(),
        vec![format!("{AOZORA_CLASS_PREFIX}{slug}")],
        kvs,
    )
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aozora::Document;

    /// Plain text round-trips into a single Pandoc Para of `Inline::Str`.
    #[test]
    fn plain_text_becomes_para() {
        let doc = Document::new("Hello, world.");
        let pandoc = to_pandoc(&doc.parse());
        assert_eq!(pandoc.blocks.len(), 1, "{:?}", pandoc.blocks);
        match &pandoc.blocks[0] {
            Block::Para(inlines) => match inlines.as_slice() {
                [Inline::Str(s)] => assert_eq!(s, "Hello, world."),
                other => panic!("expected single Str, got {other:?}"),
            },
            other => panic!("expected Para, got {other:?}"),
        }
    }

    /// `\n\n` splits into two Para blocks; single `\n` yields `SoftBreak`.
    #[test]
    fn double_newline_splits_paragraphs() {
        let doc = Document::new("One\nstill one.\n\nTwo.");
        let pandoc = to_pandoc(&doc.parse());
        let para_count = pandoc
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Para(_)))
            .count();
        assert_eq!(para_count, 2, "expected two paragraphs");
        if let Block::Para(inlines) = &pandoc.blocks[0] {
            assert!(
                inlines.iter().any(|i| matches!(i, Inline::SoftBreak)),
                "first para should carry a SoftBreak"
            );
        }
    }

    /// Ruby with explicit delimiter projects to a Span.aozora-ruby
    /// carrying base / reading sub-spans.
    #[test]
    fn ruby_projects_to_span() {
        let doc = Document::new("｜青梅《おうめ》");
        let pandoc = to_pandoc(&doc.parse());
        let para = match &pandoc.blocks[0] {
            Block::Para(inlines) => inlines,
            other => panic!("expected Para, got {other:?}"),
        };
        let ruby = para
            .iter()
            .find_map(|i| match i {
                Inline::Span(attr, inlines)
                    if attr.1.iter().any(|c| c.contains("aozora-ruby"))
                        && !attr.1.iter().any(|c| c.contains("ruby-")) =>
                {
                    Some(inlines)
                }
                _ => None,
            })
            .expect("ruby span present");
        assert_eq!(ruby.len(), 2, "ruby span has base + reading children");
    }

    /// Page break closes the in-flight paragraph and emits an
    /// `HorizontalRule` block.
    #[test]
    fn page_break_emits_horizontal_rule() {
        let doc = Document::new("before\n［＃改ページ］\nafter");
        let pandoc = to_pandoc(&doc.parse());
        assert!(
            pandoc
                .blocks
                .iter()
                .any(|b| matches!(b, Block::HorizontalRule)),
            "expected HorizontalRule for page break: {:?}",
            pandoc.blocks
        );
    }

    /// Container open / close wraps inner blocks in a Pandoc Div.
    #[test]
    fn indent_container_wraps_in_div() {
        let doc = Document::new(
            "outside\n\n\
             ［＃ここから2字下げ］\n\
             indented body\n\
             ［＃ここで字下げ終わり］\n\n\
             after",
        );
        let pandoc = to_pandoc(&doc.parse());
        let has_indent_div = pandoc.blocks.iter().any(|b| {
            matches!(
                b,
                Block::Div(attr, _)
                    if attr.1.iter().any(|c| c.contains("aozora-container-indent"))
            )
        });
        assert!(has_indent_div, "no indent Div: {:?}", pandoc.blocks);
    }
}
